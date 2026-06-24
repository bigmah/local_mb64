//! Mario Builder 64 — macOS launcher (Dioxus desktop).
//!
//! A small GUI that owns the three things the game can't configure itself:
//!   • the ROM — verify the user's `mb64.z64` against the game's exact hash and
//!     provision it into the data dir,
//!   • the working directory — the game reads/writes everything (ROM, virtual SD
//!     card, saves) relative to its CWD,
//!   • launch + window options — passed to the game as env vars on spawn.
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

use std::process::Child;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const APP_CSS: &str = include_str!("app.css");

fn main() {
    let window = WindowBuilder::new()
        .with_title("Mario Builder 64 — Launcher")
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(760.0, 760.0));
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

#[component]
fn App() -> Element {
    let mut settings = use_signal(Settings::load);
    let mut rom_status =
        use_signal(|| rom::data_dir_rom_status(&Settings::load().data_dir));
    let mut game_status = use_signal(|| GameStatus::Idle);
    let mut notice = use_signal(|| Option::<String>::None);

    // Build-from-base-ROM state.
    let repo = use_hook(paths::find_repo_root);
    let mut base_rom_ok = use_signal({
        let repo = repo.clone();
        move || repo.as_deref().map(build::baserom_in_place).unwrap_or(false)
    });
    let mut building = use_signal(|| false);
    let mut build_log = use_signal(Vec::<String>::new);
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

    // Poll the build child: stream its output into the log, detect completion.
    {
        let build_handle = build_handle.clone();
        use_future(move || {
            let build_handle = build_handle.clone();
            async move {
                loop {
                    futures_timer::Delay::new(Duration::from_millis(250)).await;
                    let mut guard = build_handle.lock().unwrap();
                    let Some(b) = guard.as_mut() else { continue };
                    while let Ok(line) = b.output.try_recv() {
                        build_log.write().push(line);
                    }
                    if let Ok(Some(status)) = b.child.try_wait() {
                        while let Ok(line) = b.output.try_recv() {
                            build_log.write().push(line);
                        }
                        let code = status.code().unwrap_or(-1);
                        build_log.write().push(if code == 0 {
                            "✅ finished (exit 0).".to_string()
                        } else {
                            format!("✗ failed (exit {code}). See the log above.")
                        });
                        *guard = None;
                        drop(guard);
                        building.set(false);
                    }
                }
            }
        });
    }

    let s = settings.read().clone();
    let binary_found = s.game_binary.is_file();
    let running = matches!(game_status(), GameStatus::Running(_));

    // Build panel display state.
    let toolchain_ok = build::toolchain_present();
    let baserom_label = if base_rom_ok() { "Base ROM ✓ — re-select…" } else { "Select base ROM…" };
    let build_label = if building() { "Building…" } else { "Build game" };
    let build_lines = build_log();
    let has_build_log = !build_lines.is_empty();
    let build_tail = build_lines[build_lines.len().saturating_sub(16)..].join("\n");

    // ── handlers ──────────────────────────────────────────────────────────────
    let on_pick_rom = move |_| {
        spawn(async move {
            let picked = rfd::AsyncFileDialog::new()
                .add_filter("N64 ROM", &["z64", "n64", "v64"])
                .set_title("Select your Mario Builder 64 ROM")
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
                    notice.set(Some(format!("ROM verified and provisioned ✓  ({})", src.display())));
                }
                Err(e) => notice.set(Some(format!("ROM not accepted: {e}"))),
            }
        });
    };

    let on_pick_baserom = {
        let repo = repo.clone();
        move |_| {
            let repo = repo.clone();
            spawn(async move {
                let Some(repo) = repo else {
                    notice.set(Some("Can't find the repo root — run the launcher from inside the repo.".into()));
                    return;
                };
                let picked = rfd::AsyncFileDialog::new()
                    .add_filter("N64 ROM", &["z64", "n64", "v64"])
                    .set_title("Select your US Super Mario 64 ROM (base ROM)")
                    .pick_file()
                    .await;
                let Some(file) = picked else { return };
                match build::place_baserom(file.path(), &repo) {
                    Ok(()) => {
                        base_rom_ok.set(true);
                        notice.set(Some("Base ROM verified ✓ — press “Build game”.".into()));
                    }
                    Err(e) => notice.set(Some(format!("Base ROM not accepted: {e}"))),
                }
            });
        }
    };

    let on_build = {
        let repo = repo.clone();
        let build_handle = build_handle.clone();
        move |_| {
            let Some(repo) = repo.clone() else {
                notice.set(Some("Can't find the repo root.".into()));
                return;
            };
            match build::start(&repo, &["all"]) {
                Ok(b) => {
                    build_log.set(vec![
                        "Starting build — the first run compiles a lot and can take several minutes…".to_string(),
                    ]);
                    *build_handle.lock().unwrap() = Some(b);
                    building.set(true);
                    notice.set(None);
                }
                Err(e) => notice.set(Some(format!("Couldn't start the build: {e}"))),
            }
        }
    };

    let on_install_toolchain = {
        let repo = repo.clone();
        let build_handle = build_handle.clone();
        move |_| {
            let Some(repo) = repo.clone() else {
                notice.set(Some("Can't find the repo root.".into()));
                return;
            };
            match build::start(&repo, &["install-toolchain"]) {
                Ok(b) => {
                    build_log.set(vec![
                        "Building the MIPS cross toolchain from source — this can take ~30–40 minutes…".to_string(),
                    ]);
                    *build_handle.lock().unwrap() = Some(b);
                    building.set(true);
                    notice.set(None);
                }
                Err(e) => notice.set(Some(format!("Couldn't start the install: {e}"))),
            }
        }
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
                    notice.set(Some("Game binary not found — build it first (see below).".into()))
                }
                Preflight::MissingRom => {
                    notice.set(Some("No ROM yet — click “Select ROM…”.".into()))
                }
                Preflight::InvalidRom => {
                    notice.set(Some("The ROM in the data folder is invalid — re-select it.".into()))
                }
            }
        }
    };

    let on_stop = move |_| {
        if let GameStatus::Running(pid) = game_status() {
            game::request_stop(pid);
        }
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

            // Status row: binary + ROM.
            div { class: "cards",
                StatusCard {
                    label: "Game",
                    ok: binary_found,
                    value: if binary_found { "Ready".to_string() } else { "Not built".to_string() },
                }
                StatusCard {
                    label: "ROM",
                    ok: rom_status() == DataDirRom::Ready,
                    value: match rom_status() {
                        DataDirRom::Ready => "Verified".to_string(),
                        DataDirRom::Missing => "Not selected".to_string(),
                        DataDirRom::Invalid => "Invalid".to_string(),
                    },
                }
            }

            // Build the game from the base ROM.
            section { class: "panel",
                h2 { "Build from your ROM" }
                p { class: "muted",
                    "Provide your US Super Mario 64 ROM and the launcher builds the decomp, recompiles it, and compiles the app. The first build needs the MIPS toolchain (mips64-elf-gcc) and can take several minutes."
                }
                div { class: "row",
                    button {
                        class: "btn",
                        disabled: building() || running,
                        onclick: on_pick_baserom,
                        "{baserom_label}"
                    }
                    button {
                        class: "btn play-btn",
                        disabled: building() || running || !base_rom_ok(),
                        onclick: on_build,
                        "{build_label}"
                    }
                }
                if !toolchain_ok {
                    div { class: "row",
                        span { class: "muted", "MIPS toolchain not found — required for the decomp build." }
                        button {
                            class: "btn",
                            disabled: building() || running,
                            onclick: on_install_toolchain,
                            "Install toolchain…"
                        }
                    }
                }
                if has_build_log {
                    pre { class: "buildlog", "{build_tail}" }
                }
            }

            // ROM provisioning (advanced: provide an already-built mb64.z64 directly).
            section { class: "panel",
                h2 { "ROM (already built)" }
                p { class: "muted",
                    "Already have a built Mario Builder 64 ROM? The launcher verifies it against the exact hash the game checks and places it in the data folder."
                }
                if let Some(src) = s.rom_source.as_ref() {
                    p { class: "path", "Last selected: {src.display()}" }
                }
                button {
                    class: "btn",
                    disabled: running,
                    onclick: on_pick_rom,
                    "Select ROM…"
                }
            }

            // Window settings.
            section { class: "panel",
                h2 { "Window" }
                p { class: "muted", "Applied on the next launch." }
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

            // Play / Stop.
            section { class: "panel play",
                if running {
                    button { class: "btn stop", onclick: on_stop, "Stop" }
                } else {
                    button {
                        class: "btn play-btn",
                        disabled: !binary_found,
                        onclick: on_play,
                        "▶  Play"
                    }
                }
                div { class: "run-status",
                    match game_status() {
                        GameStatus::Idle => rsx! { span { class: "muted", "Not running" } },
                        GameStatus::Running(pid) => rsx! { span { class: "ok", "Running (pid {pid})" } },
                        GameStatus::Exited(code) => rsx! {
                            span { class: if code == 0 { "ok" } else { "warn" },
                                "Exited (code {code})"
                            }
                        },
                        GameStatus::Failed(msg) => rsx! { span { class: "warn", "Launch failed: {msg}" } },
                    }
                }
            }

            if let Some(msg) = notice() {
                div { class: "notice", "{msg}" }
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

            if !binary_found {
                div { class: "build-hint",
                    "No game binary yet — use “Build from your ROM” above (CLI equivalent: cargo run -p mb64-build -- all)."
                }
            }
        }
    }
}

#[component]
fn StatusCard(label: &'static str, ok: bool, value: String) -> Element {
    rsx! {
        div { class: "card",
            span { class: "card-label", "{label}" }
            span { class: if ok { "card-value ok" } else { "card-value warn" },
                "{value}"
            }
        }
    }
}
