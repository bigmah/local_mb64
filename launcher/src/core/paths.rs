//! Where things live. The launcher is, for now, a developer tool that runs out of
//! the repo, so the defaults are derived from the repo layout (the game binary at
//! `app/build/mario_builder_64`, the data dir at `app/`). All of these are
//! overridable and persisted in the launcher settings.

use std::path::{Path, PathBuf};

/// The marker that identifies the repo root (the vendored game source submodule).
const REPO_MARKER: &str = "vendor/Mario-Builder-64";

/// Walk up from a starting directory looking for the repo root (the dir that
/// contains [`REPO_MARKER`]).
fn ascend_to_repo(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        if d.join(REPO_MARKER).exists() {
            return Some(d.to_path_buf());
        }
        dir = d.parent();
    }
    None
}

/// Best-effort discovery of the repo root: try the launcher executable's location
/// first (e.g. `<repo>/target/debug/mb64-launcher`), then the current directory.
pub fn find_repo_root() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent().and_then(ascend_to_repo) {
            return Some(dir);
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        if let Some(dir) = ascend_to_repo(&cwd) {
            return Some(dir);
        }
    }
    None
}

/// Default path to the built game executable, given the repo root.
pub fn default_game_binary(repo: &Path) -> PathBuf {
    repo.join("app/build/mario_builder_64")
}

/// Default data directory == the game's working directory. This is where the game
/// keeps the provisioned ROM (`mb64.us.z64`), the virtual SD card (`mb64_sd.img`),
/// and saves (`saves/mb64.us.bin`). Defaulting to `app/` means existing progress
/// from running the binary by hand is picked up automatically.
pub fn default_data_dir(repo: &Path) -> PathBuf {
    repo.join("app")
}

/// `~/Library/Application Support/MarioBuilder64Launcher` — where the launcher's
/// own settings live (distinct from the game's data dir).
pub fn launcher_support_dir() -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    home.join("Library/Application Support/MarioBuilder64Launcher")
}

/// The launcher settings file.
pub fn settings_file() -> PathBuf {
    launcher_support_dir().join("launcher.json")
}

/// Reveal a path in Finder (macOS). Best-effort; errors are ignored by callers.
pub fn reveal_in_finder(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(path).status().map(|_| ())
}
