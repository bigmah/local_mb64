//! Launching and stopping the game process.
//!
//! The game is driven entirely by its working directory (CWD == librecomp's
//! config path) plus, for window options, a small set of environment variables
//! that our glue reads in `create_window`. The launcher owns all of these.

use crate::core::rom;
use crate::core::settings::Settings;
use std::path::Path;
use std::process::{Child, Command};

/// A blocking issue that would prevent (or spoil) a launch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Preflight {
    Ok,
    MissingBinary,
    MissingRom,
    InvalidRom,
}

/// Check that everything needed to launch is in place. Creates the data dir if
/// it's absent (harmless, and the game would create it anyway).
pub fn preflight(settings: &Settings) -> Preflight {
    if !settings.game_binary.is_file() {
        return Preflight::MissingBinary;
    }
    let _ = std::fs::create_dir_all(&settings.data_dir);
    match rom::data_dir_rom_status(&settings.data_dir) {
        rom::DataDirRom::Ready => Preflight::Ok,
        rom::DataDirRom::Missing => Preflight::MissingRom,
        rom::DataDirRom::Invalid => Preflight::InvalidRom,
    }
}

/// Spawn the game with the launcher's working directory + window env vars.
pub fn spawn(settings: &Settings) -> anyhow::Result<Child> {
    let w = &settings.window;
    let child = Command::new(&settings.game_binary)
        .current_dir(&settings.data_dir)
        .env("MB64_WINDOW_WIDTH", w.width.to_string())
        .env("MB64_WINDOW_HEIGHT", w.height.to_string())
        .env("MB64_FULLSCREEN", if w.fullscreen { "1" } else { "0" })
        .spawn()?;
    Ok(child)
}

/// Ask the game to quit gracefully (SIGTERM → SDL_QUIT → flush + quick_exit).
/// Save data is durable either way (the SD card flushes per write), but SIGTERM
/// avoids leaving the process in the OS's "force quit" state.
pub fn request_stop(pid: u32) {
    // Safety: kill(2) with a known pid and a standard signal; no memory involved.
    unsafe {
        libc::kill(pid as libc::pid_t, libc::SIGTERM);
    }
}

/// Human-readable description of where save data and the virtual SD card live, for
/// the UI to surface "Open …" affordances.
pub fn saves_dir(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("saves")
}
