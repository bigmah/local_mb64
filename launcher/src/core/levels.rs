//! LevelShareSquare browse + download client.
//!
//! LSS drives its own website from a public, unauthenticated JSON API; we hit the
//! same endpoints to list/search Mario Builder 64 levels and pull the raw `.mb64`
//! bytes the game reads. No login is involved — the website's "Play" button for
//! MB64 (which has no in-browser play) just downloads the file, and we do the same:
//!
//!   - browse:   GET /api/levels/filter/get?game=4&page=N&sort=…&search=…
//!               -> { levels: [...], numberOfPages }
//!   - detail:   GET /api/levels/<id>?allAuthors=1
//!               -> { level: { …, author: { username }, description, … } }
//!   - download: GET /api/levels/<id>/code?noDescription=1&play=1
//!               -> { success, levelData: { type:"Buffer", data:[u8,…] } }
//!   - thumbs:   https://cdn.levelsharesquare.com/thumbnail/<id>.webp
//!
//! `game=4` is Mario Builder 64. The download's `levelData` Buffer is the exact
//! on-SD-card file (it begins with the "MB64-vX.Y" header), so we write it verbatim
//! into the virtual SD card (see [`crate::core::sdcard`]).

use anyhow::{anyhow, Result};
use serde::Deserialize;

/// Base URL of the LevelShareSquare API (same origin the website uses).
const API: &str = "https://levelsharesquare.com/api";
/// LSS game id for Mario Builder 64 (game 0 is Super Mario Construct, whose
/// levels are a different JSON format — MB64 is 4, matching the site's `.mb64`→4
/// mapping). MB64 levels come back as real binary `.mb64` files.
const MB64_GAME: u32 = 4;

/// CDN thumbnail URL for a level id (loaded directly by the webview `<img>`).
pub fn thumbnail_url(id: &str) -> String {
    format!("https://cdn.levelsharesquare.com/thumbnail/{id}.webp")
}

/// Public web page for a level (the "Open on the site" affordance).
pub fn level_page_url(id: &str) -> String {
    format!("https://levelsharesquare.com/levels/{id}")
}

/// Build the shared HTTP client. Cheap to clone (reference-counted internally).
pub fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .user_agent(concat!("mb64-launcher/", env!("CARGO_PKG_VERSION")))
        .build()
        .unwrap_or_default()
}

/// How to order browse results (mirrors LSS `SortTypes`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    Newest,
    Rating,
    Plays,
    Favourites,
}

impl Sort {
    /// The `sort` query value the API expects.
    fn api_value(self) -> &'static str {
        match self {
            Sort::Newest => "published",
            Sort::Rating => "rating",
            Sort::Plays => "plays",
            Sort::Favourites => "favourites",
        }
    }

    /// Human label for the sort dropdown.
    pub fn label(self) -> &'static str {
        match self {
            Sort::Newest => "Newest",
            Sort::Rating => "Top rated",
            Sort::Plays => "Most played",
            Sort::Favourites => "Most favourited",
        }
    }

    pub const ALL: [Sort; 4] = [Sort::Newest, Sort::Rating, Sort::Plays, Sort::Favourites];
}

/// One level as shown in the browse grid.
#[derive(Clone, PartialEq)]
pub struct LevelSummary {
    pub id: String,
    pub name: String,
    pub difficulty: String,
    pub game_version: String,
    pub plays: u64,
    pub favourites: u64,
    pub rating: f64,
    pub rate_count: u64,
}

impl LevelSummary {
    pub fn thumbnail(&self) -> String {
        thumbnail_url(&self.id)
    }
}

/// A page of browse results.
#[derive(Clone, PartialEq)]
pub struct LevelPage {
    pub levels: Vec<LevelSummary>,
    pub num_pages: u32,
}

/// Full detail for the selected level (description + author username).
#[derive(Clone, PartialEq)]
pub struct LevelDetail {
    pub author: String,
    pub description: String,
}

// ── raw API shapes (only the fields we use) ────────────────────────────────────

#[derive(Deserialize)]
struct RawPage {
    #[serde(default)]
    levels: Vec<RawLevel>,
    #[serde(rename = "numberOfPages")]
    number_of_pages: Option<u32>,
}

#[derive(Deserialize)]
struct RawLevel {
    #[serde(rename = "_id")]
    id: String,
    name: Option<String>,
    difficulty: Option<String>,
    #[serde(rename = "gameVersion")]
    game_version: Option<String>,
    plays: Option<u64>,
    favourites: Option<u64>,
    rating: Option<f64>,
    #[serde(rename = "rateCount")]
    rate_count: Option<u64>,
}

impl From<RawLevel> for LevelSummary {
    fn from(r: RawLevel) -> Self {
        LevelSummary {
            id: r.id,
            name: r.name.unwrap_or_else(|| "Untitled".into()),
            difficulty: r.difficulty.unwrap_or_default(),
            game_version: r.game_version.unwrap_or_default(),
            plays: r.plays.unwrap_or(0),
            favourites: r.favourites.unwrap_or(0),
            rating: r.rating.unwrap_or(0.0),
            rate_count: r.rate_count.unwrap_or(0),
        }
    }
}

#[derive(Deserialize)]
struct RawDetailWrap {
    level: RawDetail,
}

#[derive(Deserialize)]
struct RawDetail {
    author: Option<RawAuthor>,
    description: Option<String>,
}

#[derive(Deserialize)]
struct RawAuthor {
    username: Option<String>,
}

#[derive(Deserialize)]
struct RawCode {
    #[serde(rename = "levelData")]
    level_data: Option<LevelData>,
    #[serde(rename = "responseMessage")]
    response_message: Option<String>,
}

/// `levelData` is either a Node Buffer (`{type:"Buffer", data:[…]}`) for real
/// binary `.mb64` levels, or a string for legacy/text "code" levels the game
/// can't load directly.
#[derive(Deserialize)]
#[serde(untagged)]
enum LevelData {
    Buffer {
        #[serde(rename = "type")]
        kind: String,
        data: Vec<u8>,
    },
    // Legacy/text "code" levels — captured so we can report them, not load them.
    Text(#[allow(dead_code)] String),
}

// ── requests ───────────────────────────────────────────────────────────────────

/// Fetch one page of MB64 levels, optionally filtered by a search query.
pub async fn fetch_page(
    client: &reqwest::Client,
    page: u32,
    sort: Sort,
    search: &str,
) -> Result<LevelPage> {
    let mut req = client.get(format!("{API}/levels/filter/get")).query(&[
        ("game", MB64_GAME.to_string()),
        ("page", page.max(1).to_string()),
        ("sort", sort.api_value().to_string()),
    ]);
    let search = search.trim();
    if !search.is_empty() {
        req = req.query(&[("search", search)]);
    }
    let raw: RawPage = req.send().await?.error_for_status()?.json().await?;
    Ok(LevelPage {
        num_pages: raw.number_of_pages.unwrap_or(1).max(1),
        levels: raw.levels.into_iter().map(Into::into).collect(),
    })
}

/// Fetch description + author username for a single level.
pub async fn fetch_detail(client: &reqwest::Client, id: &str) -> Result<LevelDetail> {
    let raw: RawDetailWrap = client
        .get(format!("{API}/levels/{id}"))
        .query(&[("allAuthors", "1")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    Ok(LevelDetail {
        author: raw
            .level
            .author
            .and_then(|a| a.username)
            .unwrap_or_else(|| "Unknown".into()),
        description: raw.level.description.unwrap_or_default(),
    })
}

/// Download a level's raw `.mb64` file bytes (the exact on-SD-card format).
pub async fn download_mb64(client: &reqwest::Client, id: &str) -> Result<Vec<u8>> {
    let raw: RawCode = client
        .get(format!("{API}/levels/{id}/code"))
        .query(&[("noDescription", "1"), ("play", "1")])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    match raw.level_data {
        Some(LevelData::Buffer { kind, data }) if kind == "Buffer" && !data.is_empty() => Ok(data),
        Some(LevelData::Buffer { data, .. }) if !data.is_empty() => Ok(data),
        Some(LevelData::Text(_)) | Some(LevelData::Buffer { .. }) => Err(anyhow!(
            "This level isn't available as a downloadable .mb64 file."
        )),
        None => Err(anyhow!(raw
            .response_message
            .unwrap_or_else(|| "No level data was returned.".into()))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opt-in, hits the network. Confirms the reqwest client + serde shapes parse
    /// real LSS responses at runtime, and that both level storage formats are
    /// handled: a file-backed level downloads as a real `.mb64`, while an
    /// html5-only level surfaces a friendly error (not a broken file).
    ///   cargo test -p mb64-launcher live_api -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn live_api() {
        let client = client();

        // Browse + detail parse.
        let page = fetch_page(&client, 1, Sort::Newest, "")
            .await
            .expect("fetch_page");
        assert!(!page.levels.is_empty() && page.num_pages >= 1);
        eprintln!("page1: {} levels, {} pages", page.levels.len(), page.num_pages);
        let detail = fetch_detail(&client, &page.levels[0].id)
            .await
            .expect("fetch_detail");
        assert!(!detail.author.is_empty());

        // A file-backed level downloads as a real binary `.mb64`.
        let bytes = download_mb64(&client, "6a0ffe6c0441295c085055c2")
            .await
            .expect("download a file-backed level");
        assert!(bytes.starts_with(b"MB64"), "expected MB64 header");
        eprintln!("file-backed: {} bytes, header {:?}", bytes.len(),
            std::str::from_utf8(&bytes[..9]).unwrap_or("?"));

        // A JSON/html5-format level (this id is a Super Mario Construct level) is
        // rejected with a message rather than written as a broken `.mb64`.
        let err = download_mb64(&client, "6a3c81f33e11434283eb3129")
            .await
            .expect_err("html5/JSON level should error");
        eprintln!("html5/JSON correctly rejected: {err}");
    }
}
