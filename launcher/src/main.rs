//! Mario Builder 64 — macOS launcher (Dioxus desktop).
//!
//! A small GUI that owns the three things the game can't configure itself:
//!   • the ROM — verify the user's `mb64.z64` against the game's exact hash and
//!     provision it into the data dir,
//!   • the working directory — the game reads/writes everything (ROM, virtual SD
//!     card, saves) relative to its CWD,
//!   • launch + window options — passed to the game as env vars on spawn.
//!
//! The UX is deliberately two steps: **add your ROM**, then **Play**. Dropping in
//! a US Super Mario 64 ROM kicks off the whole build automatically (installing the
//! MIPS toolchain first if it's missing); the raw build log is tucked behind a
//! "Show build details" disclosure so a non-programmer never has to read it.
//!
//! The game runs as a separate process (a WebView can't host the Metal render
//! loop), which the launcher spawns, monitors, and can stop.

mod core;

use crate::core::build::{self, Build};
use crate::core::game::{self, Preflight};
use crate::core::paths;
use crate::core::rom::{self, DataDirRom};
use crate::core::settings::Settings;

use dioxus::desktop::tao::dpi::LogicalSize;
use dioxus::desktop::{Config, WindowBuilder};
use dioxus::prelude::*;

use std::path::Path;
use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const APP_CSS: &str = include_str!("app.css");

fn main() {
    let window = WindowBuilder::new()
        .with_title("Mario Builder 64 — Launcher")
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(640.0, 680.0));
    let cfg = Config::new().with_window(window);
    dioxus::LaunchBuilder::new().with_cfg(cfg).launch(App);
}

/// Lifecycle of the spawned game process, for the UI.
#[derive(Clone, PartialEq)]
enum GameStatus {
    Idle,
    Running(u32),
    Exited(i32),
    Failed(String),
}

/// Where the (possibly multi-step) build pipeline currently is. Drives the
/// friendly progress text; refined live as the build's banner lines stream in.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Idle,
    InstallingTools,
    BuildingRom,
    Recompiling,
    CompilingApp,
    Done,
    Failed,
}

impl Phase {
    /// The phase a freshly-started step begins in (refined later by banners).
    fn for_args(args: &[&str]) -> Phase {
        match args.first().copied() {
            Some("install-toolchain") => Phase::InstallingTools,
            _ => Phase::BuildingRom,
        }
    }

    /// Friendly, non-technical description of the current step.
    fn label(self) -> &'static str {
        match self {
            Phase::Idle => "Getting ready…",
            Phase::InstallingTools => "Installing build tools (one-time setup)…",
            Phase::BuildingRom => "Step 1 of 3 — Building the game from your ROM…",
            Phase::Recompiling => "Step 2 of 3 — Recompiling for macOS…",
            Phase::CompilingApp => "Step 3 of 3 — Compiling the app…",
            Phase::Done => "All done!",
            Phase::Failed => "Build stopped",
        }
    }

    /// Rough progress for the bar: `(percent, indeterminate)`.
    fn progress(self) -> (u32, bool) {
        match self {
            Phase::InstallingTools => (15, true),
            Phase::BuildingRom => (25, false),
            Phase::Recompiling => (55, false),
            Phase::CompilingApp => (82, false),
            Phase::Done => (100, false),
            _ => (8, false),
        }
    }
}

/// Start one `mb64-build` step and reflect it in the UI signals. On failure to
/// even spawn, flips into the failed state. (The step *running* to completion is
/// handled by the poll loop.)
#[allow(clippy::too_many_arguments)]
fn start_step(
    repo: &Path,
    build_handle: &Arc<Mutex<Option<Build>>>,
    args: &[&str],
    mut building: Signal<bool>,
    mut phase: Signal<Phase>,
    mut build_log: Signal<Vec<String>>,
    mut build_failed: Signal<bool>,
    mut notice: Signal<Option<String>>,
) {
    match build::start(repo, args) {
        Ok(b) => {
            *build_handle.lock().unwrap() = Some(b);
            phase.set(Phase::for_args(args));
            build_failed.set(false);
            building.set(true);
            notice.set(None);
        }
        Err(e) => {
            build_log.write().push(format!("✗ couldn't start: {e}"));
            phase.set(Phase::Failed);
            building.set(false);
            build_failed.set(true);
            notice.set(Some(format!("Couldn't start the build: {e}")));
        }
    }
}

/// Kick off the full "from your ROM" pipeline: install the toolchain first (only
/// if it's missing), then run `all`. The remaining steps are queued in `pending`
/// and advanced by the poll loop as each one finishes.
#[allow(clippy::too_many_arguments)]
fn begin_build_chain(
    repo: &Path,
    build_handle: &Arc<Mutex<Option<Build>>>,
    building: Signal<bool>,
    phase: Signal<Phase>,
    mut build_log: Signal<Vec<String>>,
    build_failed: Signal<bool>,
    mut pending: Signal<Vec<Vec<String>>>,
    notice: Signal<Option<String>>,
) {
    let mut steps: Vec<Vec<String>> = Vec::new();
    if !build::toolchain_present() {
        steps.push(vec!["install-toolchain".to_string()]);
    }
    steps.push(vec!["all".to_string()]);

    build_log.set(vec!["Setting up — you can leave this running.".to_string()]);
    let first = steps.remove(0);
    pending.set(steps);
    let argv: Vec<&str> = first.iter().map(String::as_str).collect();
    start_step(
        repo, build_handle, &argv, building, phase, build_log, build_failed, notice,
    );
}

#[component]
fn App() -> Element {
    let mut settings = use_signal(Settings::load);
    let mut rom_status = use_signal(|| rom::data_dir_rom_status(&Settings::load().data_dir));
    let mut game_status = use_signal(|| GameStatus::Idle);
    let mut notice = use_signal(|| Option::<String>::None);

    // Build-from-base-ROM state.
    let repo = use_hook(paths::find_repo_root);
    let mut base_rom_ok = use_signal({
        let repo = repo.clone();
        move || repo.as_deref().map(build::baserom_in_place).unwrap_or(false)
    });
    let building = use_signal(|| false);
    let build_log = use_signal(Vec::<String>::new);
    let phase = use_signal(|| Phase::Idle);
    let build_failed = use_signal(|| false);
    // Build steps still queued behind the running one (e.g. `all` after the
    // toolchain install). The poll loop pops the next as each finishes.
    let pending = use_signal(Vec::<Vec<String>>::new);
    let build_handle = use_hook(|| Arc::new(Mutex::new(None::<Build>)));

    // Shared handle to the child process. Held across the UI and the poll loop;
    // try_wait() (non-blocking) reaps it without consuming, so the Stop button can
    // still signal it by pid.
    let proc = use_hook(|| Arc::new(Mutex::new(None::<Child>)));

    // Poll the child for exit and reflect it in the UI.
    {
        let proc = proc.clone();
        use_future(move || {
            let proc = proc.clone();
            async move {
                loop {
                    futures_timer::Delay::new(Duration::from_millis(400)).await;
                    let mut guard = proc.lock().unwrap();
                    if let Some(child) = guard.as_mut() {
                        if let Ok(Some(status)) = child.try_wait() {
                            let code = status.code().unwrap_or(-1);
                            *guard = None;
                            drop(guard);
                            game_status.set(GameStatus::Exited(code));
                        }
                    }
                }
            }
        });
    }

    // Poll the build child: stream its output into the log (hidden behind a
    // disclosure), refine the phase from its banner lines, and on success advance
    // to the next queued step — so an upload drives toolchain → build → done with
    // no further clicks.
    {
        let build_handle = build_handle.clone();
        let repo = repo.clone();
        let mut building = building;
        let mut phase = phase;
        let mut build_log = build_log;
        let mut build_failed = build_failed;
        let mut pending = pending;
        let mut rom_status = rom_status;
        use_future(move || {
            let build_handle = build_handle.clone();
            let repo = repo.clone();
            async move {
                loop {
                    futures_timer::Delay::new(Duration::from_millis(250)).await;
                    let mut guard = build_handle.lock().unwrap();
                    let Some(b) = guard.as_mut() else { continue };
                    while let Ok(line) = b.output.try_recv() {
                        if line.contains("━━ build-rom") {
                            phase.set(Phase::BuildingRom);
                        } else if line.contains("━━ recompile") {
                            phase.set(Phase::Recompiling);
                        } else if line.contains("━━ build-app") {
                            phase.set(Phase::CompilingApp);
                        }
                        build_log.write().push(line);
                    }
                    if let Ok(Some(status)) = b.child.try_wait() {
                        while let Ok(line) = b.output.try_recv() {
                            build_log.write().push(line);
                        }
                        let code = status.code().unwrap_or(-1);
                        *guard = None;
                        drop(guard);

                        if code != 0 {
                            build_log.write().push(format!("✗ failed (exit {code})."));
                            phase.set(Phase::Failed);
                            building.set(false);
                            build_failed.set(true);
                            pending.set(Vec::new());
                            notice.set(Some(
                                "The build hit a problem. Open “Show build details” to see what happened."
                                    .to_string(),
                            ));
                            continue;
                        }

                        // Success — start the next queued step, or finish.
                        let mut steps = pending();
                        if steps.is_empty() {
                            build_log.write().push("✅ All done — press Play.".to_string());
                            phase.set(Phase::Done);
                            building.set(false);
                            rom_status
                                .set(rom::data_dir_rom_status(&settings.read().data_dir));
                        } else {
                            let next = steps.remove(0);
                            pending.set(steps);
                            if let Some(repo) = repo.as_deref() {
                                build_log.write().push(format!("→ {}", next.join(" ")));
                                let argv: Vec<&str> = next.iter().map(String::as_str).collect();
                                start_step(
                                    repo,
                                    &build_handle,
                                    &argv,
                                    building,
                                    phase,
                                    build_log,
                                    build_failed,
                                    notice,
                                );
                            }
                        }
                    }
                }
            }
        });
    }

    let s = settings.read().clone();
    let binary_found = s.game_binary.is_file();
    let rom_ready = rom_status() == DataDirRom::Ready;
    let running = matches!(game_status(), GameStatus::Running(_));
    let is_building = building();
    let ready_to_play = binary_found && rom_ready;
    let failed = build_failed();
    let toolchain_ok = build::toolchain_present();

    // Progress display state.
    let cur_phase = phase();
    let (pct, indet) = cur_phase.progress();
    let fill_class = if indet { "progress-fill indet" } else { "progress-fill" };
    let lines = build_log();
    let has_log = !lines.is_empty();
    let log_text = lines[lines.len().saturating_sub(400)..].join("\n");
    let last_line = lines.last().cloned().unwrap_or_default();

    // Two-step indicator state.
    let step1_done = base_rom_ok() || is_building || ready_to_play || running;
    let step1_class = if step1_done { "step done" } else { "step active" };
    let step2_active = ready_to_play || running;
    let step2_class = if step2_active { "step active" } else { "step" };

    // ── handlers ──────────────────────────────────────────────────────────────
    let on_pick_baserom = {
        let repo = repo.clone();
        let build_handle = build_handle.clone();
        move |_| {
            let repo = repo.clone();
            let build_handle = build_handle.clone();
            spawn(async move {
                let Some(repo) = repo else {
                    notice.set(Some(
                        "Can't find the project folder — run the launcher from inside the repo."
                            .into(),
                    ));
                    return;
                };
                let picked = rfd::AsyncFileDialog::new()
                    .add_filter("N64 ROM", &["z64", "n64", "v64"])
                    .set_title("Select your US Super Mario 64 ROM")
                    .pick_file()
                    .await;
                let Some(file) = picked else { return };
                match build::place_baserom(file.path(), &repo) {
                    Ok(()) => {
                        base_rom_ok.set(true);
                        begin_build_chain(
                            &repo,
                            &build_handle,
                            building,
                            phase,
                            build_log,
                            build_failed,
                            pending,
                            notice,
                        );
                    }
                    Err(e) => notice.set(Some(format!("That ROM wasn't accepted: {e}"))),
                }
            });
        }
    };

    // Used by both "Try again" and "Rebuild", so it's a Copy Callback.
    let rebuild = {
        let repo = repo.clone();
        let build_handle = build_handle.clone();
        use_callback(move |_: ()| {
            let Some(repo) = repo.clone() else {
                notice.set(Some("Can't find the project folder.".into()));
                return;
            };
            begin_build_chain(
                &repo,
                &build_handle,
                building,
                phase,
                build_log,
                build_failed,
                pending,
                notice,
            );
        })
    };

    let on_install_toolchain = {
        let repo = repo.clone();
        let build_handle = build_handle.clone();
        move |_| {
            let Some(repo) = repo.clone() else {
                notice.set(Some("Can't find the project folder.".into()));
                return;
            };
            let mut build_log = build_log;
            let mut pending = pending;
            build_log.set(vec!["Installing build tools…".to_string()]);
            pending.set(Vec::new());
            start_step(
                &repo,
                &build_handle,
                &["install-toolchain"],
                building,
                phase,
                build_log,
                build_failed,
                notice,
            );
        }
    };

    // Advanced: provide an already-built mb64.z64 directly (skips the build).
    let on_pick_rom = move |_| {
        spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("N64 ROM", &["z64", "n64", "v64"])
                .set_title("Select an already-built Mario Builder 64 ROM")
                .pick_file()
                .await;
            let Some(file) = picked else { return };
            let src = file.path().to_path_buf();
            let data_dir = settings.read().data_dir.clone();
            match rom::provision(&src, &data_dir) {
                Ok(()) => {
                    settings.write().rom_source = Some(src.clone());
                    let _ = settings.read().save();
                    rom_status.set(rom::data_dir_rom_status(&data_dir));
                    notice.set(Some("ROM verified and ready ✓".to_string()));
                }
                Err(e) => notice.set(Some(format!("ROM not accepted: {e}"))),
            }
        });
    };

    let on_play = {
        let proc = proc.clone();
        move |_| {
            let s = settings.read().clone();
            match game::preflight(&s) {
                Preflight::Ok => match game::spawn(&s) {
                    Ok(child) => {
                        let pid = child.id();
                        *proc.lock().unwrap() = Some(child);
                        game_status.set(GameStatus::Running(pid));
                        notice.set(None);
                    }
                    Err(e) => game_status.set(GameStatus::Failed(e.to_string())),
                },
                Preflight::MissingBinary => {
                    notice.set(Some("The game isn't built yet — add your ROM first.".into()))
                }
                Preflight::MissingRom => {
                    notice.set(Some("No ROM yet — add your Super Mario 64 ROM.".into()))
                }
                Preflight::InvalidRom => {
                    notice.set(Some("The ROM in the data folder is invalid — re-add it.".into()))
                }
            }
        }
    };

    let on_stop = move |_| {
        if let GameStatus::Running(pid) = game_status() {
            game::request_stop(pid);
        }
    };

    // A small note under the Play button about the last session, if notable.
    let run_note = match game_status() {
        GameStatus::Exited(code) if code != 0 => Some(format!("Last session exited (code {code}).")),
        GameStatus::Failed(msg) => Some(format!("Couldn't launch: {msg}")),
        _ => None,
    };

    // ── render ────────────────────────────────────────────────────────────────
    rsx! {
        style { dangerous_inner_html: APP_CSS }
        div { class: "app",
            header { class: "hero",
                div { class: "logo", "MB" }
                div {
                    h1 { "Mario Builder 64" }
                    p { class: "subtitle", "macOS native launcher" }
                }
            }

            // Two-step path: add your ROM → play.
            div { class: "stepper",
                div { class: "{step1_class}",
                    span { class: "dot", if step1_done { "✓" } else { "1" } }
                    span { "Add your ROM" }
                }
                div { class: "sep" }
                div { class: "{step2_class}",
                    span { class: "dot", if ready_to_play { "✓" } else { "2" } }
                    span { "Play" }
                }
            }

            // The one thing to do right now, adapted to where we are.
            section { class: "stage",
                if running {
                    div { class: "stage-body",
                        div { class: "pulse" }
                        h2 { "Playing" }
                        p { class: "muted", "Mario Builder 64 is running." }
                        button { class: "btn stop", onclick: on_stop, "Stop" }
                    }
                } else if is_building {
                    div { class: "stage-body",
                        div { class: "spinner" }
                        h2 { "{cur_phase.label()}" }
                        p { class: "muted",
                            "Building the game from your ROM. This can take several minutes — you can leave it running."
                        }
                        div { class: "progress",
                            div { class: "{fill_class}", style: "width: {pct}%" }
                        }
                        if has_log {
                            p { class: "ticker", "{last_line}" }
                            details { class: "logwrap",
                                summary { "Show build details" }
                                pre { class: "buildlog", "{log_text}" }
                            }
                        }
                    }
                } else if ready_to_play {
                    div { class: "stage-body",
                        div { class: "big-check", "✓" }
                        h2 { "Ready to play" }
                        p { class: "muted", "Your ROM is verified and the game is built." }
                        button { class: "btn play-btn", onclick: on_play, "▶  Play" }
                        if let Some(note) = run_note {
                            p { class: "muted small", "{note}" }
                        }
                    }
                } else if failed {
                    div { class: "stage-body",
                        div { class: "big-x", "!" }
                        h2 { "Something went wrong" }
                        p { class: "muted",
                            "The build didn't finish. You can try again, or open the details to see what happened."
                        }
                        button { class: "btn play-btn", onclick: move |_| rebuild.call(()), "Try again" }
                        if has_log {
                            details { class: "logwrap",
                                summary { "Show build details" }
                                pre { class: "buildlog", "{log_text}" }
                            }
                        }
                    }
                } else {
                    div { class: "stage-body",
                        div { class: "big-icon", "🎮" }
                        h2 { "Add your Super Mario 64 ROM" }
                        p { class: "muted",
                            "Choose your legally-owned US Super Mario 64 ROM (.z64). The launcher builds Mario Builder 64 from it automatically — then just press Play."
                        }
                        button { class: "btn play-btn", onclick: on_pick_baserom, "Select your ROM…" }
                        if !toolchain_ok {
                            p { class: "muted small",
                                "First time on this Mac? Setup also installs the build tools it needs — a one-time step that can take a while."
                            }
                        }
                    }
                }
            }

            if let Some(msg) = notice() {
                div { class: "notice", "{msg}" }
            }

            // Everything a non-programmer never needs to touch.
            details { class: "advanced",
                summary { "Advanced" }
                div { class: "adv-body",
                    div { class: "adv-item",
                        div {
                            div { class: "adv-title", "Rebuild from your ROM" }
                            p { class: "muted small", "Run the full build again." }
                        }
                        button {
                            class: "btn",
                            disabled: is_building || running,
                            onclick: move |_| rebuild.call(()),
                            "Rebuild"
                        }
                    }
                    div { class: "adv-item",
                        div {
                            div { class: "adv-title", "Install build tools" }
                            p { class: "muted small", "MIPS toolchain + dependencies. Normally handled automatically." }
                        }
                        button {
                            class: "btn",
                            disabled: is_building || running,
                            onclick: on_install_toolchain,
                            if toolchain_ok { "Reinstall" } else { "Install" }
                        }
                    }
                    div { class: "adv-item",
                        div {
                            div { class: "adv-title", "Use an already-built ROM" }
                            p { class: "muted small", "Skip building if you already have a Mario Builder 64 ROM." }
                        }
                        button {
                            class: "btn",
                            disabled: running,
                            onclick: on_pick_rom,
                            "Select…"
                        }
                    }
                    if let Some(src) = s.rom_source.as_ref() {
                        p { class: "path", "Last ROM: {src.display()}" }
                    }
                    div { class: "adv-item col",
                        div { class: "adv-title", "Window" }
                        div { class: "row",
                            label { "Width"
                                input {
                                    r#type: "number", min: "320", step: "16",
                                    value: "{s.window.width}",
                                    disabled: running,
                                    oninput: move |e| {
                                        if let Ok(v) = e.value().parse::<u32>() {
                                            settings.write().window.width = v.max(1);
                                            let _ = settings.read().save();
                                        }
                                    }
                                }
                            }
                            label { "Height"
                                input {
                                    r#type: "number", min: "240", step: "16",
                                    value: "{s.window.height}",
                                    disabled: running,
                                    oninput: move |e| {
                                        if let Ok(v) = e.value().parse::<u32>() {
                                            settings.write().window.height = v.max(1);
                                            let _ = settings.read().save();
                                        }
                                    }
                                }
                            }
                            label { class: "check",
                                input {
                                    r#type: "checkbox",
                                    checked: s.window.fullscreen,
                                    disabled: running,
                                    onchange: move |e| {
                                        settings.write().window.fullscreen = e.checked();
                                        let _ = settings.read().save();
                                    }
                                }
                                "Fullscreen"
                            }
                        }
                    }
                }
            }

            footer { class: "footer",
                span { class: "path", "Data: {s.data_dir.display()}" }
                div { class: "links",
                    button {
                        class: "link",
                        onclick: move |_| { let _ = paths::reveal_in_finder(&settings.read().data_dir); },
                        "Open data folder"
                    }
                    button {
                        class: "link",
                        onclick: move |_| { let _ = paths::reveal_in_finder(&game::saves_dir(&settings.read().data_dir)); },
                        "Open saves"
                    }
                }
            }
        }
    }
}
