//! Launcher settings: the game binary + data dir to use, and the window options
//! that get passed to the game as environment variables on launch. Persisted as
//! JSON under `~/Library/Application Support/MarioBuilder64Launcher/launcher.json`.

use crate::core::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Window options the launcher hands the game via env vars (see `core::game`).
/// The game falls back to these same defaults when the vars are absent, so an
/// unconfigured launch behaves exactly like running the binary by hand.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowSettings {
    pub width: u32,
    pub height: u32,
    pub fullscreen: bool,
}

impl Default for WindowSettings {
    fn default() -> Self {
        // Matches the hardcoded SDL_CreateWindow size in mb64_main.cpp.
        WindowSettings { width: 1600, height: 960, fullscreen: false }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// Path to the built `mario_builder_64` executable.
    pub game_binary: PathBuf,
    /// The game's working directory (where its ROM, SD card, and saves live).
    pub data_dir: PathBuf,
    /// The last ROM the user picked (for display only; the real ROM lives in the
    /// data dir once provisioned).
    pub rom_source: Option<PathBuf>,
    pub window: WindowSettings,
}

impl Settings {
    /// Defaults derived from the repo layout (falls back to the current dir if the
    /// repo root can't be found, e.g. a packaged build).
    pub fn defaults() -> Self {
        let repo = paths::find_repo_root().unwrap_or_else(|| PathBuf::from("."));
        Settings {
            game_binary: paths::default_game_binary(&repo),
            data_dir: paths::default_data_dir(&repo),
            rom_source: None,
            window: WindowSettings::default(),
        }
    }

    /// Load persisted settings, or the defaults if none exist / can't be parsed.
    pub fn load() -> Self {
        let path = paths::settings_file();
        match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| Settings::defaults()),
            Err(_) => Settings::defaults(),
        }
    }

    /// Persist settings (best-effort, atomic via a temp file + rename).
    pub fn save(&self) -> anyhow::Result<()> {
        let dir = paths::launcher_support_dir();
        std::fs::create_dir_all(&dir)?;
        let final_path = paths::settings_file();
        let tmp = final_path.with_extension("json.tmp");
        let json = serde_json::to_vec_pretty(self)?;
        std::fs::write(&tmp, json)?;
        std::fs::rename(&tmp, &final_path)?;
        Ok(())
    }
}
