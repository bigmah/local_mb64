//! Base-ROM verification + driving the full build pipeline from the UI.
//!
//! The "build everything" path: take the user's legally-owned **US Super Mario 64**
//! ROM, place it at the repo root as `baserom.us.z64`, then run `mb64-build all`
//! (decomp → recompile → app) as a child process, streaming its merged output
//! back to the UI line by line.
//!
//! Kept Dioxus-free so it stays unit-testable (`cargo test -p mb64-launcher`).

use anyhow::{bail, Context, Result};
use std::fmt::Write as _;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};

/// SHA-1 of the supported base ROM: US Super Mario 64 (`.z64`). The decomp build
/// extracts the game's assets from exactly this ROM (matches `mb64-build doctor`).
pub const SM64_US_SHA1: &str = "9bef1128717f958171a4afac3ed78ee2bb4e86ce";

/// The name the decomp + orchestrator expect the base ROM under, at the repo root.
pub const BASEROM_NAME: &str = "baserom.us.z64";

/// Verdict on a candidate base-ROM file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BaseRomCheck {
    /// SHA-1 matches the US SM64 ROM.
    Good,
    /// A readable file, but the wrong ROM (carries the offending SHA-1).
    WrongRom(String),
    /// Couldn't read the file.
    Unreadable(String),
}

fn sha1_hex(bytes: &[u8]) -> String {
    use sha1::{Digest, Sha1};
    let mut h = Sha1::new();
    h.update(bytes);
    let mut s = String::with_capacity(40);
    for b in h.finalize() {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Verify a candidate file is the US SM64 base ROM (by SHA-1).
pub fn check_baserom(path: &Path) -> BaseRomCheck {
    match std::fs::read(path) {
        Ok(bytes) => {
            let h = sha1_hex(&bytes);
            if h == SM64_US_SHA1 {
                BaseRomCheck::Good
            } else {
                BaseRomCheck::WrongRom(h)
            }
        }
        Err(e) => BaseRomCheck::Unreadable(e.to_string()),
    }
}

/// Is a valid base ROM already in place at the repo root?
pub fn baserom_in_place(repo: &Path) -> bool {
    matches!(check_baserom(&repo.join(BASEROM_NAME)), BaseRomCheck::Good)
}

/// Verify `src` and copy it to `<repo>/baserom.us.z64`.
pub fn place_baserom(src: &Path, repo: &Path) -> Result<()> {
    match check_baserom(src) {
        BaseRomCheck::Good => {}
        BaseRomCheck::WrongRom(h) => {
            bail!("that ROM's SHA-1 is {h}, not the US SM64 ROM ({SM64_US_SHA1})")
        }
        BaseRomCheck::Unreadable(e) => bail!("could not read the ROM: {e}"),
    }
    let dest = repo.join(BASEROM_NAME);
    std::fs::copy(src, &dest)
        .with_context(|| format!("copying base ROM to {}", dest.display()))?;
    Ok(())
}

/// A running build: the child process plus a stream of its merged output lines.
pub struct Build {
    pub child: Child,
    pub output: Receiver<String>,
}

/// MIPS cross-compiler prefixes the decomp build accepts (mirrors `mb64-build`).
const MIPS_PREFIXES: &[&str] = &[
    "mips64-elf-",
    "mips-n64-",
    "mips64-",
    "mips-linux-gnu-",
    "mips64-linux-gnu-",
    "mips64-none-elf-",
];

/// `bin` dirs that may hold an off-PATH `mips64-elf-gcc` (mirrors `mb64-build`):
/// the persistent install, then the legacy `/tmp/n64tc`.
fn toolchain_bin_dirs() -> Vec<std::path::PathBuf> {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    vec![
        home.join(".mb64/toolchain/bin"),
        std::path::PathBuf::from("/tmp/n64tc/bin"),
    ]
}

/// Is a MIPS cross-`gcc` (needed by the decomp build) available — on PATH, or at one
/// of the known install locations the orchestrator auto-detects?
pub fn toolchain_present() -> bool {
    let on_path = MIPS_PREFIXES.iter().any(|p| {
        Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {p}gcc"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    });
    on_path || toolchain_bin_dirs().iter().any(|d| d.join("mips64-elf-gcc").is_file())
}

/// Spawn `mb64-build <subcmd>` with the repo as its working directory, merging
/// stdout+stderr into a line stream. Prefers the prebuilt `mb64-build` binary
/// bundled next to the launcher (so a downloaded `.app` needs no Rust); falls back
/// to `cargo run` inside a dev checkout.
pub fn start(repo: &Path, subcmd: &[&str]) -> Result<Build> {
    let mut cmd = orchestrator_command(repo);
    cmd.args(subcmd);
    spawn_streamed(cmd).with_context(|| "spawning mb64-build")
}

/// Build the base `Command` for the orchestrator (without its subcommand args),
/// rooted at `repo`.
fn orchestrator_command(repo: &Path) -> Command {
    match crate::core::bootstrap::orchestrator_path() {
        Some(bin) => {
            let mut c = Command::new(bin);
            c.current_dir(repo);
            c
        }
        None => {
            let mut c = Command::new("cargo");
            c.current_dir(repo)
                .args(["run", "--quiet", "-p", "mb64-build", "--"]);
            c
        }
    }
}

/// Spawn a configured command with piped stdout+stderr merged into a single line
/// stream. Shared by the build orchestrator and the source-bootstrap (git) steps
/// so they show progress and complete through the same poll loop.
pub fn spawn_streamed(mut cmd: Command) -> Result<Build> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| "spawning child process")?;

    let (tx, rx) = mpsc::channel::<String>();
    if let Some(out) = child.stdout.take() {
        let tx = tx.clone();
        std::thread::spawn(move || pump(BufReader::new(out), tx));
    }
    if let Some(err) = child.stderr.take() {
        let tx = tx.clone();
        std::thread::spawn(move || pump(BufReader::new(err), tx));
    }
    drop(tx); // so the channel closes once both pumps finish
    Ok(Build { child, output: rx })
}

/// Forward each line of `reader` to `tx` until EOF or the receiver is gone.
fn pump<R: BufRead>(reader: R, tx: Sender<String>) {
    for line in reader.lines() {
        match line {
            Ok(l) => {
                if tx.send(l).is_err() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_sm64() {
        let dir = std::env::temp_dir();
        let p = dir.join("mb64-launcher-test-notrom.bin");
        std::fs::write(&p, b"not a rom").unwrap();
        assert!(matches!(check_baserom(&p), BaseRomCheck::WrongRom(_)));
        let _ = std::fs::remove_file(&p);
    }

    /// If the real base ROM is present at the repo root, it must verify.
    #[test]
    fn real_baserom_verifies_if_present() {
        let Some(repo) = crate::core::paths::find_repo_root() else { return };
        let rom = repo.join(BASEROM_NAME);
        if !rom.exists() {
            eprintln!("skipping: {} not present", rom.display());
            return;
        }
        assert_eq!(check_baserom(&rom), BaseRomCheck::Good);
    }
}
