//! Build orchestrator for the Mario Builder 64 macOS native port.
//!
//! The port is a multi-stage pipeline. This CLI makes that brittle sequence
//! reproducible from a fresh clone + a user-supplied US SM64 ROM:
//!
//!   build-rom   decomp (make) -> build/rom/mb64.{elf,z64}
//!   recompile   N64Recomp + post-process + RSPRecomp -> app/RecompiledFuncs + app/rsp
//!   build-app   cmake (Ninja) + the N64ModernRuntime patch -> app/build/mario_builder_64
//!   all         the three above, in order
//!   play        launch the built game
//!   doctor      verify the environment
//!
//! Local modifications to pinned submodules / generated code live as patch files
//! under `patches/` and are applied here (so a fresh checkout stays buildable):
//!   patches/Mario-Builder-64-*.patch   -> applied to the decomp before `make`
//!   patches/recompiled-*.patch         -> applied to the recompiled C after N64Recomp
//!   patches/N64ModernRuntime-*.patch   -> applied to the runtime submodule before cmake

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// SHA-1 of the only base ROM we support extracting from: US Super Mario 64 (`.z64`).
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

/// GNU make 4.x shimmed into PATH (Apple's bundled make 3.81 misparses the decomp Makefile).
const GNU_MAKE_GNUBIN: &str = "/opt/homebrew/opt/make/libexec/gnubin";
/// Homebrew prefix (SDL2 etc.) handed to CMake.
const HOMEBREW_PREFIX: &str = "/opt/homebrew";

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
    /// Build mb64.us.elf + mb64.us.z64 from the vendored decomp (needs a MIPS toolchain).
    BuildRom,
    /// Statically recompile mb64.us.elf with N64Recomp (+ post-process + audio ucode).
    Recompile,
    /// Configure + build the native macOS app with CMake/Ninja.
    BuildApp,
    /// Run the whole pipeline: build-rom -> recompile -> build-app.
    All,
    /// Launch the built game.
    Play,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Doctor => doctor(),
        Cmd::BuildRom => build_rom(&Ctx::discover()?),
        Cmd::Recompile => recompile(&Ctx::discover()?),
        Cmd::BuildApp => build_app(&Ctx::discover()?),
        Cmd::All => {
            let c = Ctx::discover()?;
            build_rom(&c)?;
            recompile(&c)?;
            build_app(&c)?;
            println!("\n✅ pipeline complete — run `mb64-build play`");
            Ok(())
        }
        Cmd::Play => play(&Ctx::discover()?),
    }
}

// ── pipeline context ───────────────────────────────────────────────────────────

struct Ctx {
    root: PathBuf,
    jobs: usize,
}

impl Ctx {
    fn discover() -> Result<Self> {
        let root = find_project_root().context("could not locate the project root")?;
        let jobs = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        Ok(Ctx { root, jobs })
    }
    fn app(&self) -> PathBuf {
        self.root.join("app")
    }
    fn decomp(&self) -> PathBuf {
        self.root.join("vendor/Mario-Builder-64")
    }
    fn rom_out(&self) -> PathBuf {
        self.root.join("build/rom")
    }
}

// ── stage: build-rom ───────────────────────────────────────────────────────────

fn build_rom(c: &Ctx) -> Result<()> {
    banner("build-rom", "decomp → build/rom/mb64.{elf,z64}");

    let baserom = c.root.join("baserom.us.z64");
    if !baserom.is_file() {
        bail!(
            "baserom.us.z64 not found at {} — drop your legally-owned US SM64 ROM there",
            baserom.display()
        );
    }
    verify_us_rom(&baserom)?;

    let decomp = c.decomp();
    if !decomp.join("Makefile").is_file() {
        bail!("decomp source missing — run: git submodule update --init --recursive");
    }
    // The decomp's own build reads ./baserom.us.z64; mirror ours in.
    let inner = decomp.join("baserom.us.z64");
    if !inner.exists() {
        fs::copy(&baserom, &inner).with_context(|| "placing baserom into the decomp tree")?;
    }

    let mips = find_mips_prefix().ok_or_else(|| {
        anyhow::anyhow!(
            "no MIPS cross-compiler on PATH (need e.g. `mips64-elf-gcc`).\n  \
             install: brew install make coreutils && brew install tehzz/n64-dev/mips64-elf-gcc"
        )
    })?;
    println!("  MIPS toolchain: {mips}gcc");

    // Local decomp patches (e.g. the GCC IPA-clone CFLAGS fix) must be applied first.
    apply_patches(c, &decomp, "Mario-Builder-64")?;

    // GNU make 4.x must shadow Apple make; unset LIBRARY_PATH (breaks the host-tool link).
    let mut path = String::new();
    if Path::new(GNU_MAKE_GNUBIN).is_dir() {
        let _ = write!(path, "{GNU_MAKE_GNUBIN}:");
    }
    path.push_str(&std::env::var("PATH").unwrap_or_default());

    run(
        Command::new("make")
            .current_dir(&decomp)
            .env("PATH", &path)
            .env_remove("LIBRARY_PATH")
            .args(["VERSION=us", "COMPILER=gcc", &format!("-j{}", c.jobs)]),
        "make (decomp)",
    )?;

    let out = decomp.join("build/us_n64");
    let rom_out = c.rom_out();
    fs::create_dir_all(&rom_out)?;
    for f in ["mb64.elf", "mb64.z64"] {
        let src = out.join(f);
        if !src.is_file() {
            bail!("expected decomp output {} was not produced", src.display());
        }
        fs::copy(&src, rom_out.join(f)).with_context(|| format!("staging {f}"))?;
    }
    println!("  ✅ staged build/rom/mb64.elf + mb64.z64");
    Ok(())
}

// ── stage: recompile ───────────────────────────────────────────────────────────

fn recompile(c: &Ctx) -> Result<()> {
    banner("recompile", "N64Recomp → post-process → RSPRecomp");

    let rom_out = c.rom_out();
    let elf = rom_out.join("mb64.elf");
    if !elf.is_file() {
        bail!("{} missing — run `mb64-build build-rom` first", elf.display());
    }

    // N64Recomp reads its config + mb64.elf from its working dir and writes RecompiledFuncs/.
    let cfg = c.root.join("recomp/mb64.us.toml");
    fs::copy(&cfg, rom_out.join("mb64.us.toml")).with_context(|| "staging recomp config")?;
    let n64recomp = c.root.join("tools/bin/N64Recomp");
    run(
        Command::new(&n64recomp).current_dir(&rom_out).arg("mb64.us.toml"),
        "N64Recomp",
    )?;

    let funcs = rom_out.join("RecompiledFuncs");
    // macOS libc-collision renames the recompiler doesn't already handle.
    postprocess_renames(&funcs)?;
    // Re-apply hand-edits to generated functions (input / SD card) as name-keyed
    // overrides — regen-stable, since N64Recomp's function→file layout shifts.
    apply_overrides(&funcs, &c.root.join("patches/recompiled-overrides"))?;
    // Insert scheduler-preemption checks at loop back-edges (globs RecompiledFuncs/*.c in CWD).
    run(
        Command::new("python3")
            .current_dir(&rom_out)
            .arg(c.root.join("tools/insert_preempt.py")),
        "insert_preempt.py",
    )?;
    // Any additional captured line-patches to the recompiled C.
    apply_patches(c, &rom_out, "recompiled")?;

    // Audio microcode -> app/rsp/aspMain.cpp (RSPRecomp reads app/aspMain.us.toml in app/).
    let rsprecomp = c.root.join("tools/bin/RSPRecomp");
    run(
        Command::new(&rsprecomp).current_dir(c.app()).arg("aspMain.us.toml"),
        "RSPRecomp (audio ucode)",
    )?;

    // Stage the recompiled C into the app.
    stage_dir(&funcs, &c.app().join("RecompiledFuncs"))?;
    println!("  ✅ recompiled C → app/RecompiledFuncs, audio ucode → app/rsp/aspMain.cpp");
    Ok(())
}

/// Rename recompiled symbols that collide with the macOS C library.
fn postprocess_renames(funcs: &Path) -> Result<()> {
    const RENAMES: &[(&str, &str)] = &[
        ("__fpclassifyf", "mb64_fpclassifyf"),
        ("strncpy", "mb64_strncpy"),
    ];
    if !funcs.is_dir() {
        bail!("N64Recomp produced no {} directory", funcs.display());
    }
    let mut touched = 0;
    for entry in fs::read_dir(funcs).with_context(|| format!("reading {}", funcs.display()))? {
        let path = entry?.path();
        let is_src = path
            .extension()
            .map(|e| e == "c" || e == "h")
            .unwrap_or(false);
        if !is_src {
            continue;
        }
        let mut text = fs::read_to_string(&path)?;
        let mut changed = false;
        for (from, to) in RENAMES {
            if text.contains(from) {
                text = text.replace(from, to);
                changed = true;
            }
        }
        if changed {
            fs::write(&path, &text)?;
            touched += 1;
        }
    }
    println!("  post-process: renamed libc collisions in {touched} file(s)");
    Ok(())
}

/// Replace each name-keyed override (`patches/recompiled-overrides/<name>.c`) in the
/// freshly recompiled C. Matches by function name (ELF-derived, stable), not by file
/// position — N64Recomp's function→file split shifts between tool versions, so a
/// file/line patch would not survive a regen.
fn apply_overrides(funcs: &Path, overrides_dir: &Path) -> Result<()> {
    if !overrides_dir.is_dir() {
        return Ok(());
    }
    let mut entries: Vec<PathBuf> = fs::read_dir(overrides_dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().map(|e| e == "c").unwrap_or(false))
        .collect();
    entries.sort();
    let mut applied = 0;
    for op in &entries {
        let name = op.file_stem().and_then(|s| s.to_str()).unwrap_or_default();
        let body = fs::read_to_string(op)?;
        if replace_function(funcs, name, &body)? {
            applied += 1;
        } else {
            bail!("override target function `{name}` not found in recompiled output");
        }
    }
    if applied > 0 {
        println!("  applied {applied} regen-stable function override(s)");
    }
    Ok(())
}

/// The function name in a `RECOMP_FUNC <ret> <name>(...)` definition line, if any.
fn recomp_func_name(line: &str) -> Option<&str> {
    if !line.trim_start().starts_with("RECOMP_FUNC ") {
        return None;
    }
    line.split('(').next()?.split_whitespace().last()
}

/// Find `name` in any `funcs_*.c` under `funcs`, brace-match its body, replace it with
/// `body`. Returns whether a replacement was made.
fn replace_function(funcs: &Path, name: &str, body: &str) -> Result<bool> {
    for entry in fs::read_dir(funcs)? {
        let path = entry?.path();
        if path.extension().map(|e| e != "c").unwrap_or(true) {
            continue;
        }
        let text = fs::read_to_string(&path)?;
        let lines: Vec<&str> = text.lines().collect();
        let Some(start) = lines.iter().position(|l| recomp_func_name(l) == Some(name)) else {
            continue;
        };
        // Brace-match the function body from its opening line.
        let (mut depth, mut started, mut end) = (0i32, false, start);
        for (j, l) in lines.iter().enumerate().skip(start) {
            depth += l.matches('{').count() as i32 - l.matches('}').count() as i32;
            started |= l.contains('{');
            if started && depth == 0 {
                end = j;
                break;
            }
        }
        if !started {
            continue; // a bodyless declaration — keep looking for the definition
        }
        let mut out = String::new();
        for l in &lines[..start] {
            out.push_str(l);
            out.push('\n');
        }
        out.push_str(body.trim_end());
        out.push('\n');
        for l in &lines[end + 1..] {
            out.push_str(l);
            out.push('\n');
        }
        fs::write(&path, out)?;
        return Ok(true);
    }
    Ok(false)
}

// ── stage: build-app ───────────────────────────────────────────────────────────

fn build_app(c: &Ctx) -> Result<()> {
    banner("build-app", "cmake (Ninja) → app/build/mario_builder_64");

    let app = c.app();
    if !app.join("RecompiledFuncs/funcs.h").is_file() {
        bail!("app/RecompiledFuncs missing — run `mb64-build recompile` first");
    }
    let nmr = app.join("lib/N64ModernRuntime");
    if !nmr.join("CMakeLists.txt").is_file() {
        bail!("submodules not initialized — run: git submodule update --init --recursive");
    }
    // Re-apply the runtime's scheduler-preemption patch (idempotent).
    let patch = c.root.join("patches/N64ModernRuntime-preemption.patch");
    if patch.is_file() {
        apply_patch_file(&nmr, &patch, "N64ModernRuntime-preemption")?;
    }

    // CMake 4 dropped policy < 3.5 compat that bundled libs still declare.
    run(
        Command::new("cmake").current_dir(&app).args([
            "-B",
            "build",
            "-G",
            "Ninja",
            "-DCMAKE_POLICY_VERSION_MINIMUM=3.5",
            &format!("-DCMAKE_PREFIX_PATH={HOMEBREW_PREFIX}"),
        ]),
        "cmake configure",
    )?;
    run(
        Command::new("cmake").current_dir(&app).args([
            "--build",
            "build",
            &format!("-j{}", c.jobs),
            "--target",
            "mario_builder_64",
        ]),
        "cmake build",
    )?;

    // The game reads its ROM from the working dir; provision the matching one.
    let z = c.rom_out().join("mb64.z64");
    if z.is_file() {
        let _ = fs::copy(&z, app.join("mb64.z64"));
    }
    println!("  ✅ built app/build/mario_builder_64");
    Ok(())
}

// ── stage: play ────────────────────────────────────────────────────────────────

fn play(c: &Ctx) -> Result<()> {
    let bin = c.app().join("build/mario_builder_64");
    if !bin.is_file() {
        bail!("game not built — run `mb64-build all` (or build-app) first");
    }
    println!("launching {}", bin.display());
    let status = Command::new(&bin)
        .current_dir(c.app())
        .status()
        .with_context(|| "launching the game")?;
    std::process::exit(status.code().unwrap_or(0));
}

// ── patch application ──────────────────────────────────────────────────────────

/// Apply every `patches/<prefix>*.patch` to `target` (idempotent).
fn apply_patches(c: &Ctx, target: &Path, prefix: &str) -> Result<()> {
    let dir = c.root.join("patches");
    if !dir.is_dir() {
        return Ok(());
    }
    let mut patches: Vec<PathBuf> = fs::read_dir(&dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("");
            name.starts_with(prefix) && name.ends_with(".patch")
        })
        .collect();
    patches.sort();
    for p in &patches {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("patch");
        apply_patch_file(target, p, name)?;
    }
    Ok(())
}

/// Apply a single patch with `patch -p1`, skipping if already applied.
fn apply_patch_file(target: &Path, patch: &Path, name: &str) -> Result<()> {
    // If it reverse-applies cleanly, it is already in place.
    let already = Command::new("patch")
        .current_dir(target)
        .args(["-p1", "-R", "-s", "-f", "--dry-run", "-i"])
        .arg(patch)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if already {
        println!("  patch already applied: {name}");
        return Ok(());
    }
    run(
        Command::new("patch")
            .current_dir(target)
            .args(["-p1", "--forward", "-s", "-i"])
            .arg(patch),
        &format!("patch {name}"),
    )
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
    checks.push(check_tool("clang", "C compiler (recompiled output + N64Recomp)"));
    checks.push(check_tool("clang++", "C++ compiler (RT64 / runtime)"));
    checks.push(check_cmake());
    checks.push(check_tool("ninja", "build driver for the C++ app"));
    checks.push(check_sdl2());
    checks.push(check_mips_toolchain());
    checks.push(check_submodule(&root));
    checks.push(check_rom(&root));

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
    let Some(v) = first_line(&capture("cmake", &["--version"])) else {
        return Check { level: Level::Fail, name: "cmake".into(), detail: "NOT FOUND (need >= 3.20)".into() };
    };
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
        let ver = first_line(&capture("pkg-config", &["--modversion", "sdl2"])).unwrap_or_default();
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
        detail: "none — `brew install tehzz/n64-dev/mips64-elf-gcc` (or crosstool-ng)".into(),
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
            detail: "missing — `git submodule update --init --recursive`".into(),
        }
    }
}

fn check_rom(root: &Path) -> Check {
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

fn banner(stage: &str, desc: &str) {
    println!("\n\u{2501}\u{2501} {stage} — {desc} \u{2501}\u{2501}");
}

/// Run a command with inherited stdio (the user sees make/cmake output live).
fn run(cmd: &mut Command, label: &str) -> Result<()> {
    println!("  → {label}");
    let status = cmd.status().with_context(|| format!("spawning: {label}"))?;
    if !status.success() {
        bail!("{label} failed (exit {})", status.code().unwrap_or(-1));
    }
    Ok(())
}

fn verify_us_rom(path: &Path) -> Result<()> {
    let h = sha1_of(path)?;
    if h != US_ROM_SHA1 {
        bail!("{} has SHA-1 {h}, not the US SM64 ROM ({US_ROM_SHA1})", path.display());
    }
    Ok(())
}

fn find_mips_prefix() -> Option<String> {
    MIPS_PREFIXES
        .iter()
        .find(|p| on_path(&format!("{p}gcc")))
        .map(|p| p.to_string())
}

/// Replace `dst` with a recursive copy of `src`.
fn stage_dir(src: &Path, dst: &Path) -> Result<()> {
    if !src.is_dir() {
        bail!("nothing to stage: {} is not a directory", src.display());
    }
    if dst.exists() {
        fs::remove_dir_all(dst).ok();
    }
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).ok();
    }
    run(Command::new("cp").arg("-R").arg(src).arg(dst), "stage RecompiledFuncs")
}

/// Walk up from CWD to the directory that holds `vendor/Mario-Builder-64`.
fn find_project_root() -> Result<PathBuf> {
    let mut dir = std::env::current_dir()?;
    loop {
        if dir.join("vendor/Mario-Builder-64").exists() {
            return Ok(dir);
        }
        if !dir.pop() {
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
fn capture(bin: &str, args: &[&str]) -> String {
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
