//! Build orchestrator for the Mario Builder 64 macOS native port.
//!
//! The port is a multi-stage pipeline (build the decomp ROM/ELF → statically
//! recompile with N64Recomp → build the native macOS app). This CLI exists to
//! make that brittle sequence reproducible. Today it implements `doctor` (verify
//! the environment) and stubs the pipeline stages that follow.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// SHA-1 of the only ROM we support extracting from: US Super Mario 64 (`.z64`).
const US_ROM_SHA1: &str = "9bef1128717f958171a4afac3ed78ee2bb4e86ce";

/// MIPS cross-compiler prefixes the MB64 Makefile will accept, best first.
const MIPS_PREFIXES: &[&str] = &[
    "mips64-elf-",
    "mips-n64-",
    "mips64-",
    "mips-linux-gnu-",
    "mips64-linux-gnu-",
    "mips64-none-elf-",
];

#[derive(Parser)]
#[command(
    name = "mb64-build",
    version,
    about = "Build orchestrator for the Mario Builder 64 macOS native port"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check that the toolchain, dependencies, and ROM are ready.
    Doctor,
    /// (planned) Build mb64.us.elf + mb64.us.z64 from the vendored decomp.
    BuildRom,
    /// (planned) Statically recompile mb64.us.elf with N64Recomp.
    Recompile,
    /// (planned) Configure + build the native macOS .app.
    BuildApp,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Doctor => doctor(),
        Cmd::BuildRom => not_yet("build-rom", &[
            "locate vendor/Mario-Builder-64 + your baserom.us.z64",
            "run `make VERSION=us GRUCODE=f3dex2` with a native MIPS toolchain",
            "collect build/us/mb64.us.elf and build/us/mb64.us.z64",
        ]),
        Cmd::Recompile => not_yet("recompile", &[
            "emit recomp/mb64.us.toml (entrypoint, sections, overlays, symbols)",
            "run N64Recomp + RSPRecomp (audio ucode)",
            "iterate manual_funcs/function_sizes until no functions are dropped",
        ]),
        Cmd::BuildApp => not_yet("build-app", &[
            "configure CMake with -DCMAKE_POLICY_VERSION_MINIMUM=3.5 (CMake 4 compat)",
            "link RecompiledFuncs + librecomp + ultramodern + rt64 (Metal)",
            "produce MarioBuilder64.app",
        ]),
    }
}

fn not_yet(stage: &str, steps: &[&str]) -> Result<()> {
    println!("`{stage}` is not implemented yet. Planned steps:");
    for (i, s) in steps.iter().enumerate() {
        println!("  {}. {s}", i + 1);
    }
    Ok(())
}

// ── doctor ───────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq)]
enum Level {
    Ok,
    Warn,
    Fail,
}

impl Level {
    fn glyph(self) -> &'static str {
        match self {
            Level::Ok => "\u{2713}",   // ✓
            Level::Warn => "\u{26a0}", // ⚠
            Level::Fail => "\u{2717}", // ✗
        }
    }
}

struct Check {
    level: Level,
    name: String,
    detail: String,
}

fn doctor() -> Result<()> {
    let root = find_project_root().context("could not locate the project root")?;
    println!("Mario Builder 64 — environment check");
    println!("project root: {}\n", root.display());

    let mut checks: Vec<Check> = Vec::new();

    // Host compilers (used for the recompiled C and the C++ runtime/renderer).
    checks.push(check_tool("clang", "C compiler (recompiled output + N64Recomp)"));
    checks.push(check_tool("clang++", "C++ compiler (RT64 / runtime)"));

    // CMake — flag the 4.x compatibility gotcha we already hit.
    checks.push(check_cmake());
    checks.push(check_tool("ninja", "build driver for the C++ app"));

    // SDL2 (windowing/input for the runtime) via pkg-config.
    checks.push(check_sdl2());

    // The MIPS cross-compiler for the decomp build (native, no Docker).
    checks.push(check_mips_toolchain());

    // Vendored game source.
    checks.push(check_submodule(&root));

    // The user's ROM.
    checks.push(check_rom(&root));

    // Render.
    let mut out = String::new();
    for c in &checks {
        let _ = writeln!(out, "  {} {:<26} {}", c.level.glyph(), c.name, c.detail);
    }
    print!("{out}");

    let fails = checks.iter().filter(|c| c.level == Level::Fail).count();
    let warns = checks.iter().filter(|c| c.level == Level::Warn).count();
    println!();
    if fails == 0 {
        println!("Ready: {} checks passed, {warns} warning(s).", checks.len() - warns - fails);
        Ok(())
    } else {
        bail!("{fails} blocking issue(s) — resolve the ✗ items above, then re-run `doctor`.");
    }
}

fn check_tool(bin: &str, detail: &str) -> Check {
    match on_path(bin) {
        true => Check { level: Level::Ok, name: bin.into(), detail: detail.into() },
        false => Check { level: Level::Fail, name: bin.into(), detail: format!("NOT FOUND — {detail}") },
    }
}

fn check_cmake() -> Check {
    let Some(v) = first_line(&run("cmake", &["--version"])) else {
        return Check { level: Level::Fail, name: "cmake".into(), detail: "NOT FOUND (need >= 3.20)".into() };
    };
    // v looks like "cmake version 4.3.0"
    let ver = v.split_whitespace().last().unwrap_or("").to_string();
    let major = ver.split('.').next().and_then(|s| s.parse::<u32>().ok()).unwrap_or(0);
    if major >= 4 {
        Check {
            level: Level::Warn,
            name: "cmake".into(),
            detail: format!("{ver} — pass -DCMAKE_POLICY_VERSION_MINIMUM=3.5 (handled by build-app)"),
        }
    } else {
        Check { level: Level::Ok, name: "cmake".into(), detail: ver }
    }
}

fn check_sdl2() -> Check {
    if !on_path("pkg-config") {
        return Check { level: Level::Warn, name: "sdl2".into(), detail: "pkg-config missing — can't verify".into() };
    }
    let ok = Command::new("pkg-config").args(["--exists", "sdl2"]).status().map(|s| s.success()).unwrap_or(false);
    if ok {
        let ver = first_line(&run("pkg-config", &["--modversion", "sdl2"])).unwrap_or_default();
        Check { level: Level::Ok, name: "sdl2".into(), detail: format!("{ver} (pkg-config)") }
    } else {
        Check { level: Level::Fail, name: "sdl2".into(), detail: "not found — `brew install sdl2`".into() }
    }
}

fn check_mips_toolchain() -> Check {
    for p in MIPS_PREFIXES {
        let gcc = format!("{p}gcc");
        let asm = format!("{p}as");
        if on_path(&gcc) {
            return Check { level: Level::Ok, name: "mips toolchain".into(), detail: format!("{gcc} (gcc)") };
        }
        if on_path(&asm) {
            return Check {
                level: Level::Warn,
                name: "mips toolchain".into(),
                detail: format!("{asm} present (binutils) — C built via clang -target mips"),
            };
        }
    }
    Check {
        level: Level::Fail,
        name: "mips toolchain".into(),
        detail: "none — install mips64-elf gcc (crosstool-ng) or mips-linux-gnu-binutils".into(),
    }
}

fn check_submodule(root: &Path) -> Check {
    let p = root.join("vendor/Mario-Builder-64/Makefile");
    if p.is_file() {
        Check { level: Level::Ok, name: "MB64 source".into(), detail: "vendor/Mario-Builder-64".into() }
    } else {
        Check {
            level: Level::Fail,
            name: "MB64 source".into(),
            detail: "missing — `git submodule update --init`".into(),
        }
    }
}

fn check_rom(root: &Path) -> Check {
    // Accept the ROM at the project root or inside the decomp tree.
    let candidates = [root.join("baserom.us.z64"), root.join("vendor/Mario-Builder-64/baserom.us.z64")];
    let Some(path) = candidates.iter().find(|p| p.is_file()) else {
        return Check {
            level: Level::Warn,
            name: "baserom.us.z64".into(),
            detail: "not provided yet — drop your legal US SM64 ROM at the project root".into(),
        };
    };
    match sha1_of(path) {
        Ok(hash) if hash == US_ROM_SHA1 => {
            Check { level: Level::Ok, name: "baserom.us.z64".into(), detail: "US ROM, SHA-1 verified".into() }
        }
        Ok(hash) => Check {
            level: Level::Fail,
            name: "baserom.us.z64".into(),
            detail: format!("SHA-1 {hash} != US {US_ROM_SHA1} (wrong region/format?)"),
        },
        Err(e) => Check { level: Level::Fail, name: "baserom.us.z64".into(), detail: format!("unreadable: {e}") },
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────

/// Walk up from CWD to the directory that holds `vendor/Mario-Builder-64`.
fn find_project_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("vendor/Mario-Builder-64").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
            // Fall back to CWD rather than failing hard.
            return std::env::current_dir().map_err(Into::into);
        }
    }
}

fn on_path(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Run a command, returning combined stdout+stderr (empty on failure to spawn).
fn run(bin: &str, args: &[&str]) -> String {
    match Command::new(bin).args(args).output() {
        Ok(o) => {
            let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
            s.push_str(&String::from_utf8_lossy(&o.stderr));
            s
        }
        Err(_) => String::new(),
    }
}

fn first_line(s: &str) -> Option<String> {
    s.lines().next().map(|l| l.trim().to_string()).filter(|l| !l.is_empty())
}

fn sha1_of(path: &Path) -> Result<String> {
    use sha1::{Digest, Sha1};
    let bytes = fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let mut hasher = Sha1::new();
    hasher.update(&bytes);
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}
