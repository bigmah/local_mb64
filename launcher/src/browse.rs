//! The "Browse online levels" view: search/sort LevelShareSquare's Mario Builder
//! 64 catalog, preview a level, and download it straight into the game's virtual
//! SD card so it shows up in the in-game level browser.
//!
//! Everything here is read-only HTTP against LSS's public API (see
//! [`crate::core::levels`]) plus a local FAT write on install (see
//! [`crate::core::sdcard`]). Thumbnails are loaded directly by the webview from
//! the LSS CDN, so we never fetch image bytes in Rust.

use crate::core::levels::{self, LevelDetail, LevelPage, LevelSummary, Sort};
use crate::core::sdcard;
use dioxus::prelude::*;
use std::path::PathBuf;

/// Open a URL in the user's default browser (macOS `open`).
fn open_url(url: &str) {
    let _ = std::process::Command::new("open").arg(url).spawn();
}

/// CSS modifier for a difficulty chip's color.
fn difficulty_class(d: &str) -> &'static str {
    match d.to_lowercase().as_str() {
        "easy" => "diff-easy",
        "normal" => "diff-normal",
        "hard" => "diff-hard",
        "expert" => "diff-expert",
        "extreme" => "diff-extreme",
        _ => "diff-other",
    }
}

#[component]
pub fn BrowseLevels(
    data_dir: PathBuf,
    running: bool,
    on_play: EventHandler<()>,
    on_back: EventHandler<()>,
) -> Element {
    let client = use_hook(levels::client);

    let mut page = use_signal(|| 1u32);
    let mut sort = use_signal(|| Sort::Newest);
    let mut query = use_signal(String::new); // applied search term
    let mut search_input = use_signal(String::new); // live text-box contents

    // None = loading; Some(Ok/Err) = a settled page request.
    let mut result = use_signal(|| None as Option<Result<LevelPage, String>>);

    // (Re)fetch whenever page / sort / query change (also runs once on mount).
    {
        let client = client.clone();
        use_effect(move || {
            let (p, s, q) = (page(), sort(), query());
            let client = client.clone();
            result.set(None);
            spawn(async move {
                let r = levels::fetch_page(&client, p, s, &q)
                    .await
                    .map_err(|e| e.to_string());
                result.set(Some(r));
            });
        });
    }

    // The level whose detail modal is open, plus its (lazily fetched) detail.
    let mut selected = use_signal(|| None as Option<LevelSummary>);
    let mut detail = use_signal(|| None as Option<Result<LevelDetail, String>>);
    let mut install_msg = use_signal(|| None as Option<(String, bool)>); // (text, is_error)
    let installing = use_signal(|| false);

    {
        let client = client.clone();
        use_effect(move || {
            let sel = selected();
            install_msg.set(None);
            let client = client.clone();
            if let Some(s) = sel {
                detail.set(None);
                spawn(async move {
                    let r = levels::fetch_detail(&client, &s.id)
                        .await
                        .map_err(|e| e.to_string());
                    detail.set(Some(r));
                });
            }
        });
    }

    // Filenames already on the card, so cards can show an "Installed" badge.
    let mut installed = {
        let dd = data_dir.clone();
        use_signal(move || sdcard::installed_filenames(&dd))
    };

    // Download + install the given level into the SD card image.
    let download = {
        let client = client.clone();
        let data_dir = data_dir.clone();
        use_callback(move |s: LevelSummary| {
            let client = client.clone();
            let data_dir = data_dir.clone();
            let mut installing = installing;
            installing.set(true);
            install_msg.set(Some(("Downloading…".to_string(), false)));
            spawn(async move {
                match levels::download_mb64(&client, &s.id).await {
                    Ok(bytes) => match sdcard::install(&data_dir, &s.name, &bytes) {
                        Ok(fname) => {
                            install_msg.set(Some((
                                format!("Installed “{fname}”. Press Play, then pick it in the in-game level browser."),
                                false,
                            )));
                            installed.set(sdcard::installed_filenames(&data_dir));
                        }
                        Err(e) => install_msg.set(Some((format!("Couldn't install: {e}"), true))),
                    },
                    Err(e) => install_msg.set(Some((format!("Download failed: {e}"), true))),
                }
                installing.set(false);
            });
        })
    };

    // Apply the search box: jump back to page 1 with the typed query.
    let mut apply_search = move || {
        page.set(1);
        query.set(search_input());
    };

    rsx! {
        div { class: "browse",
            div { class: "browse-top",
                button { class: "link back", onclick: move |_| on_back.call(()), "← Launcher" }
                div { class: "browse-title",
                    h2 { "Browse online levels" }
                    span { class: "muted small", "From LevelShareSquare" }
                }
            }

            div { class: "browse-controls",
                input {
                    class: "search",
                    r#type: "text",
                    placeholder: "Search levels…",
                    value: "{search_input}",
                    oninput: move |e| search_input.set(e.value()),
                    onkeydown: move |e| if e.key() == Key::Enter { apply_search() },
                }
                button { class: "btn", onclick: move |_| apply_search(), "Search" }
                select {
                    class: "sort",
                    onchange: move |e| {
                        let idx: usize = e.value().parse().unwrap_or(0);
                        sort.set(Sort::ALL.get(idx).copied().unwrap_or(Sort::Newest));
                        page.set(1);
                    },
                    for (i, opt) in Sort::ALL.iter().enumerate() {
                        option { value: "{i}", selected: *opt == sort(), "{opt.label()}" }
                    }
                }
            }

            if running {
                div { class: "notice",
                    "The game is running — stop it first to install new levels (they'd be overwritten otherwise)."
                }
            }

            // Results.
            match result() {
                None => rsx! {
                    div { class: "browse-state",
                        div { class: "spinner" }
                        p { class: "muted", "Loading levels…" }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "browse-state",
                        div { class: "big-x", "!" }
                        p { class: "muted", "Couldn't load levels: {e}" }
                        button { class: "btn", onclick: move |_| query.set(query()), "Try again" }
                    }
                },
                Some(Ok(pg)) => {
                    let num_pages = pg.num_pages;
                    let cur = page();
                    rsx! {
                        if pg.levels.is_empty() {
                            div { class: "browse-state",
                                p { class: "muted", "No levels found." }
                            }
                        } else {
                            div { class: "lvl-grid",
                                for level in pg.levels.clone() {
                                    {
                                        let is_installed = installed().iter().any(|f| *f == sdcard::level_filename(&level.name));
                                        let sel = level.clone();
                                        rsx! {
                                            button {
                                                key: "{level.id}",
                                                class: "lvl-card",
                                                onclick: move |_| selected.set(Some(sel.clone())),
                                                div { class: "thumb",
                                                    img { src: "{level.thumbnail()}", loading: "lazy" }
                                                    if is_installed {
                                                        span { class: "badge", "Installed" }
                                                    }
                                                }
                                                div { class: "lvl-meta",
                                                    div { class: "lvl-name", "{level.name}" }
                                                    div { class: "lvl-sub",
                                                        span { class: "chip {difficulty_class(&level.difficulty)}", "{level.difficulty}" }
                                                        span { class: "stat", "▶ {level.plays}" }
                                                        span { class: "stat", "★ {level.rating:.1}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            div { class: "pager",
                                button {
                                    class: "btn",
                                    disabled: cur <= 1,
                                    onclick: move |_| page.set((page() - 1).max(1)),
                                    "‹ Prev"
                                }
                                span { class: "muted small", "Page {cur} of {num_pages}" }
                                button {
                                    class: "btn",
                                    disabled: cur >= num_pages,
                                    onclick: move |_| page.set(page() + 1),
                                    "Next ›"
                                }
                            }
                        }
                    }
                }
            }

            // Detail modal.
            if let Some(sel) = selected() {
                LevelModal {
                    level: sel.clone(),
                    detail: detail(),
                    install_msg: install_msg(),
                    installing: installing(),
                    running,
                    on_close: move |_| selected.set(None),
                    on_download: move |_| download.call(sel.clone()),
                    on_play: move |_| {
                        on_play.call(());
                        on_back.call(());
                    },
                }
            }
        }
    }
}

#[component]
fn LevelModal(
    level: LevelSummary,
    detail: Option<Result<LevelDetail, String>>,
    install_msg: Option<(String, bool)>,
    installing: bool,
    running: bool,
    on_close: EventHandler<()>,
    on_download: EventHandler<()>,
    on_play: EventHandler<()>,
) -> Element {
    let page_url = levels::level_page_url(&level.id);
    rsx! {
        div { class: "modal-backdrop", onclick: move |_| on_close.call(()),
            div { class: "modal", onclick: move |e| e.stop_propagation(),
                button { class: "modal-x", onclick: move |_| on_close.call(()), "×" }
                div { class: "modal-thumb",
                    img { src: "{level.thumbnail()}" }
                }
                h3 { "{level.name}" }
                div { class: "lvl-sub",
                    span { class: "chip {difficulty_class(&level.difficulty)}", "{level.difficulty}" }
                    if !level.game_version.is_empty() {
                        span { class: "stat", "{level.game_version}" }
                    }
                    span { class: "stat", "▶ {level.plays} plays" }
                    span { class: "stat", "★ {level.rating:.1} ({level.rate_count})" }
                    span { class: "stat", "♥ {level.favourites}" }
                }

                match detail {
                    None => rsx! { p { class: "muted small", "Loading details…" } },
                    Some(Ok(d)) => rsx! {
                        p { class: "muted small author", "by {d.author}" }
                        if !d.description.trim().is_empty() {
                            pre { class: "lvl-desc", "{d.description}" }
                        }
                    },
                    Some(Err(_)) => rsx! { span {} },
                }

                if let Some((msg, is_err)) = install_msg {
                    div { class: if is_err { "notice err" } else { "notice ok-notice" }, "{msg}" }
                }

                div { class: "modal-actions",
                    if installing {
                        button { class: "btn play-btn", disabled: true, "Installing…" }
                    } else {
                        button {
                            class: "btn play-btn",
                            disabled: running,
                            onclick: move |_| on_download.call(()),
                            "⬇  Download & install"
                        }
                    }
                    button { class: "btn", onclick: move |_| on_play.call(()), "▶  Play" }
                    button { class: "link", onclick: move |_| open_url(&page_url), "Open on site" }
                }
                if running {
                    p { class: "muted small", "Stop the game to install this level." }
                }
            }
        }
    }
}
