//! Self-bootstrap: turn a freshly-downloaded launcher into a working build.
//!
//! A downloaded `.app` ships only our two Rust binaries (the launcher + the
//! `mb64-build` orchestrator). Everything else it needs it fetches at runtime:
//!   • the game source — `git clone --recurse-submodules` of the public repo,
//!   • the host build tools — Xcode Command Line Tools (git/clang/make) and
//!     Homebrew (cmake/ninja/sdl2 + the cross-toolchain's math libs).
//!
//! No Nintendo assets are involved: the user still supplies their own ROM, and
//! the recompiled game code is produced locally at build time — nothing
//! copyrighted is downloaded or shipped.
//!
//! Kept Dioxus-free so it stays unit-testable (`cargo test -p mb64-launcher`).

use crate::core::build::{spawn_streamed, Build};
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The public source repo the launcher clones (HTTPS so no SSH keys are needed).
pub const SOURCE_URL: &str = "https://github.com/bigmah/local_mb64.git";

/// The exact source ref to check out. CI stamps the release's git SHA in via
/// `MB64_SOURCE_REF` so the bundled `mb64-build` always matches the cloned tree;
/// dev builds (unset) track `main`.
pub fn source_ref() -> &'static str {
    match option_env!("MB64_SOURCE_REF") {
        Some(r) if !r.is_empty() => r,
        _ => "main",
    }
}

// ── source checkout ─────────────────────────────────────────────────────────────

/// Whether a directory holds a usable source checkout.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceStatus {
    /// Nothing there yet — needs an initial clone.
    Missing,
    /// Cloned, but submodules aren't fully populated — needs `submodule update`.
    Incomplete,
    /// Source + submodules present; ready to build.
    Ready,
}

/// Inspect `dir` and report whether the game source is ready to build there.
pub fn source_status(dir: &Path) -> SourceStatus {
    let cloned = dir.join(".git").exists();
    let decomp_ok = dir_non_empty(&dir.join("vendor/Mario-Builder-64"));
    // A populated runtime submodule proves `submodule update --recursive` ran.
    let submodules_ok = dir.join("app/lib/N64ModernRuntime/CMakeLists.txt").is_file();
    if !cloned && !decomp_ok {
        SourceStatus::Missing
    } else if decomp_ok && submodules_ok {
        SourceStatus::Ready
    } else {
        SourceStatus::Incomplete
    }
}

fn dir_non_empty(p: &Path) -> bool {
    std::fs::read_dir(p).map(|mut it| it.next().is_some()).unwrap_or(false)
}

/// Clone (or, if already present, update) the source into `dest` at the pinned
/// ref, with all submodules. Returns a streamed child so the UI can show progress
/// and detect completion exactly like a build.
pub fn clone_source(dest: &Path) -> Result<Build> {
    if let Some(parent) = dest.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // One shell script handles both fresh-clone and update; paths/URLs go through
    // the environment to sidestep any quoting issues.
    let script = r#"
set -e
if [ -d "$MB64_DEST/.git" ]; then
  echo "Updating existing source in $MB64_DEST ..."
  git -C "$MB64_DEST" fetch --tags --force origin
  git -C "$MB64_DEST" checkout --force "$MB64_REF"
  git -C "$MB64_DEST" submodule sync --recursive
  git -C "$MB64_DEST" submodule update --init --recursive --progress
else
  echo "Cloning Mario Builder 64 source into $MB64_DEST ..."
  git clone --recurse-submodules --progress "$MB64_URL" "$MB64_DEST"
  git -C "$MB64_DEST" checkout --force "$MB64_REF"
  git -C "$MB64_DEST" submodule update --init --recursive --progress
fi
echo "Source ready."
"#;
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(script)
        .env("MB64_DEST", dest)
        .env("MB64_URL", SOURCE_URL)
        .env("MB64_REF", source_ref());
    spawn_streamed(cmd).with_context(|| "starting git clone")
}

// ── bundled orchestrator ────────────────────────────────────────────────────────

/// Locate the `mb64-build` orchestrator binary: first a sibling of the launcher
/// executable (the copy bundled in the `.app`), then `PATH`. `None` means there's
/// no prebuilt binary — callers fall back to `cargo run` (dev checkout).
pub fn orchestrator_path() -> Option<PathBuf> {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let cand = dir.join("mb64-build");
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let out = Command::new("sh").arg("-c").arg("command -v mb64-build").output().ok()?;
    if out.status.success() {
        let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !p.is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    None
}

// ── host prerequisites ──────────────────────────────────────────────────────────

/// A host build tool the launcher can't bundle but the build needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prereq {
    /// Xcode Command Line Tools — provides git, clang, and make.
    CommandLineTools,
    /// Homebrew — provides cmake/ninja/sdl2 and the cross-toolchain's math libs.
    Homebrew,
}

/// All host prerequisites, in install order (git/clang first, then Homebrew).
pub const PREREQS: [Prereq; 2] = [Prereq::CommandLineTools, Prereq::Homebrew];

impl Prereq {
    pub fn title(self) -> &'static str {
        match self {
            Prereq::CommandLineTools => "Command Line Tools",
            Prereq::Homebrew => "Homebrew",
        }
    }
    pub fn detail(self) -> &'static str {
        match self {
            Prereq::CommandLineTools => "git, clang, and make (from Apple)",
            Prereq::Homebrew => "cmake, ninja, sdl2, and build libraries",
        }
    }
    pub fn present(self) -> bool {
        match self {
            Prereq::CommandLineTools => clt_present(),
            Prereq::Homebrew => on_path("brew"),
        }
    }
    /// Begin installing this prerequisite. Both paths hand off to a process the
    /// user finishes interactively (Apple's CLT installer / Terminal for Homebrew's
    /// sudo prompt), so the caller should prompt the user to re-check afterwards.
    pub fn begin_install(self) -> Result<()> {
        match self {
            Prereq::CommandLineTools => install_command_line_tools(),
            Prereq::Homebrew => install_homebrew_in_terminal(),
        }
    }
}

/// A cheap snapshot of prerequisite presence, so the UI can hold it in a signal
/// instead of shelling out on every render.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prereqs {
    pub clt: bool,
    pub brew: bool,
}

impl Prereqs {
    pub fn snapshot() -> Self {
        Prereqs {
            clt: Prereq::CommandLineTools.present(),
            brew: Prereq::Homebrew.present(),
        }
    }
    pub fn ok(self) -> bool {
        self.clt && self.brew
    }
    pub fn present(self, p: Prereq) -> bool {
        match p {
            Prereq::CommandLineTools => self.clt,
            Prereq::Homebrew => self.brew,
        }
    }
}

fn clt_present() -> bool {
    // `xcode-select -p` prints the active developer dir and exits 0 once the CLT (or
    // full Xcode) are installed; also require `git` to actually resolve on PATH.
    Command::new("xcode-select")
        .arg("-p")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
        && on_path("git")
}

fn on_path(tool: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {tool}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Kick off Apple's Command Line Tools installer (it shows its own GUI progress).
/// The call returns immediately; the user completes the system dialog, then
/// re-checks. Idempotent — a no-op if the tools are already present.
fn install_command_line_tools() -> Result<()> {
    Command::new("xcode-select")
        .arg("--install")
        .status()
        .with_context(|| "running `xcode-select --install`")?;
    Ok(())
}

/// Open Terminal running Homebrew's official installer. Homebrew needs sudo and a
/// TTY (to create `/opt/homebrew`), which a GUI child process can't provide, so we
/// hand it to Terminal where the user can enter their password.
fn install_homebrew_in_terminal() -> Result<()> {
    let script = "#!/bin/bash\n\
        /bin/bash -c \"$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)\"\n\
        echo\n\
        echo 'Homebrew install finished — you can close this window and return to the launcher.'\n";
    let path = std::env::temp_dir().join("mb64-install-homebrew.command");
    std::fs::write(&path, script).with_context(|| "writing Homebrew install script")?;
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    Command::new("open")
        .arg("-a")
        .arg("Terminal")
        .arg(&path)
        .status()
        .with_context(|| "opening Terminal for the Homebrew install")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_dir_is_missing_source() {
        let dir = std::env::temp_dir().join("mb64-bootstrap-test-empty");
        let _ = std::fs::create_dir_all(&dir);
        assert_eq!(source_status(&dir), SourceStatus::Missing);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// If we're running inside the dev checkout with submodules populated, the repo
    /// root must read as Ready (ground-truth when present; skipped otherwise).
    #[test]
    fn dev_checkout_reads_ready_when_populated() {
        let Some(repo) = crate::core::paths::find_repo_root() else { return };
        if !repo.join("app/lib/N64ModernRuntime/CMakeLists.txt").is_file() {
            eprintln!("skipping: submodules not initialized");
            return;
        }
        assert_eq!(source_status(&repo), SourceStatus::Ready);
    }
}
