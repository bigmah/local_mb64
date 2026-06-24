# Mario Builder 64 — native macOS port

A project to build a **standalone native macOS application** that plays
[Mario Builder 64](https://github.com/arthurtilly/Mario-Builder-64) (arthurtilly's
Super Mario 64 ROM hack with a full in-game level editor) on Apple Silicon — **not**
an emulator, and not "load a `.z64` into mupen64". A real app you double-click.

> **Status: early / in active bring-up.** This repo currently contains the project
> scaffold, vendored sources, and tooling. It does not yet produce a playable build.
> See [docs/BUILD.md](docs/BUILD.md) for the current milestone state.

## Approach

Mario Builder 64 is a pure **N64-target** decompilation (built on the
HackerSM64 lineage) with no existing PC build path. Rather than hand-porting a
platform layer onto a heavily-diverged engine, this project uses **static
recompilation** via [N64: Recompiled](https://github.com/N64Recomp/N64Recomp) —
the same technology behind the standalone *Majora's Mask Recompiled* app:

```
your US SM64 ROM ─┐
                  ├─► build mb64.elf + mb64.z64 (decomp build, MIPS toolchain)
HackerSM64/MB64 ──┘            │
                              ▼
                  N64Recomp (mb64.elf → C)  +  RSPRecomp (audio microcode)
                              │
                              ▼
            N64ModernRuntime (libultra reimpl) + RT64 (Vulkan→MoltenVK)
                              │
                              ▼
                   MarioBuilder64.app  (native macOS, Apple Silicon)
```

The recompiled game code and the RT64 renderer / runtime are existing C/C++
projects we *use*. The **new code written for this project is in Rust**: the
**Dioxus launcher** (ROM provisioning, settings, controller config, launch) and
the build-orchestration **tooling**.

## You must supply your own ROM

This project bundles **zero Nintendo assets**. Building requires a legally-owned
**US Super Mario 64 ROM** (`baserom.us.z64`, big-endian, SHA-1
`9bef1128717f958171a4afac3ed78ee2bb4e86ce`), used at build time only to extract
the game's media. This is a **source-available** project: you build it yourself
and bring your own ROM. No ROMs, extracted assets, recompiled game code, or
prebuilt app binaries are distributed. See [docs/LEGAL.md](docs/LEGAL.md).

## Download the launcher (easiest — no command line)

Don't want to touch a terminal? Grab the prebuilt **launcher** from the
[Releases page](https://github.com/bigmah/mb64_mac.git/releases):

1. Download `MarioBuilder64-Launcher-*.dmg`, open it, and drag the app to **Applications**.
2. The app is unsigned, so the **first** time: right-click it → **Open** → **Open**
   (this clears macOS Gatekeeper; afterwards you can launch it normally).
3. In the launcher: **Set up** (it checks for Apple's Command Line Tools + Homebrew,
   offering to install anything missing, then clones this open-source project) →
   **Add your ROM** (your own US SM64 `.z64`) → it builds itself → **Play**.

The download contains **only our own launcher + build tooling** — no ROM, no game
code, no Nintendo assets. The game is built locally on your Mac from *your* ROM the
first time you run it (the first build compiles a MIPS toolchain and can take a
while). Building from source still requires Xcode Command Line Tools and Homebrew;
the launcher will walk you through installing them.

## Layout

| Path | What |
|------|------|
| `vendor/Mario-Builder-64/` | MB64 decomp source (git submodule, upstream) |
| `app/` | the native macOS app — our glue (`src/mb64_*`), recomp config, CMake |
| `app/lib/` | third-party runtime/renderer/UI libraries (**git submodules**, upstream) |
| `recomp/` | N64Recomp config for MB64 (TOML, symbols, overlays) |
| `launcher/` | Rust + Dioxus launcher app |
| `tools/` | Rust build-orchestration tooling |
| `patches/` | local patches applied to dependencies at build time |
| `docs/` | Architecture, build, and legal notes |

## Building from source

```bash
# 1. Clone WITH submodules (the game, runtime, renderer, and UI libraries)
git clone --recurse-submodules https://github.com/bigmah/mb64_mac.git
cd mb64_mac
#    (already cloned without --recurse-submodules? run:)
git submodule update --init --recursive

# 2. Check the environment (compilers, cmake/ninja/sdl2, MIPS toolchain, ROM)
cargo run -p mb64-build -- doctor

# 3. One-time: install the MIPS cross toolchain (binutils + gcc, built from source
#    into ~/.mb64/toolchain, ~30–40 min) plus the Homebrew build deps. Skips
#    anything already present.
cargo run -p mb64-build -- install-toolchain

# 4. Provide your own US SM64 ROM as baserom.us.z64 at the repo root, then build:
cargo run -p mb64-build -- all     # build-rom → recompile → build-app
cargo run -p mb64-build -- play    # …or just use the Dioxus launcher:
cargo run -p mb64-launcher         # drop in your SM64 ROM → it builds itself → Play
```

Nothing third-party is committed into this repo: `app/lib/*` are pinned git
submodules, and local changes to dependencies / generated code are kept as
reviewable patches under `patches/`, applied automatically by the `mb64-build`
orchestrator at the right pipeline stage (the `N64ModernRuntime` scheduler-preemption
fix before the app build; the decomp IPA-clone CFLAGS fix before `make`). The MIPS
cross toolchain is built from source on demand — never committed — into a persistent
per-user location the orchestrator and launcher auto-detect.

## Docs

- [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — the recompilation pipeline and where Rust fits
- [docs/BUILD.md](docs/BUILD.md) — toolchain, dependencies, and milestone status
- [docs/LEGAL.md](docs/LEGAL.md) — ROM/asset/licensing posture

## Credits & acknowledgements

This project is a small layer of original glue (the `app/src/mb64_*` native code,
the Rust launcher, and the build tooling) on top of a large body of existing
open-source work. Nearly everything that makes a native build possible was written
by other people — huge thanks to all of them.

**The game**

- [**Mario Builder 64**](https://github.com/arthurtilly/Mario-Builder-64) — by
  *arthurtilly* and contributors. The SM64 ROM hack / in-game level editor this
  project plays. Vendored as a git submodule (`vendor/Mario-Builder-64`).
- [**HackerSM64**](https://github.com/HackerN64/HackerSM64) — the modernized SM64
  decompilation base that Mario Builder 64 is built on.

**The recompilation + runtime stack** (the *Majora's Mask Recompiled* technology
that lets an N64 ROM run as a native program)

- [**N64: Recompiled**](https://github.com/N64Recomp/N64Recomp) (`N64Recomp` /
  `RSPRecomp`) — by *Wiseguy* and the N64Recomp project. Statically recompiles the
  game's MIPS code (and audio microcode) into C.
- [**RT64**](https://github.com/rt64/rt64) — by *Dario Sanfilippo (DarioSamo)* et al.
  The high-accuracy N64 renderer (native Metal on macOS). *MIT.*
- [**N64ModernRuntime**](https://github.com/N64Recomp/N64ModernRuntime)
  (`librecomp` + `ultramodern`) — the libultra reimplementation and recomp runtime.
  *GPLv3.* In turn builds on [xxHash](https://github.com/Cyan4973/xxHash),
  [miniz](https://github.com/richgel999/miniz), and
  [o1heap](https://github.com/N64Recomp/o1heap).
- [**drmario64_recomp**](https://github.com/AngheloAlf/drmario64_recomp) — by
  *AngheloAlf*. The recomp app template our `app/` scaffold is adapted from.
- [**Zelda64Recomp**](https://github.com/Zelda64Recomp/Zelda64Recomp) — the
  reference recomp application that template descends from.

**Bundled libraries** (vendored as submodules under `app/lib/`)

- [**RmlUi**](https://github.com/mikke89/RmlUi) — *mikke89* — HTML/CSS UI library. *MIT.*
- [**moodycamel::ConcurrentQueue**](https://github.com/cameron314/concurrentqueue) — *cameron314*. *zlib.*
- [**lunasvg**](https://github.com/sammycage/lunasvg) — *sammycage* — SVG rendering. *MIT.*
- [**sse2neon**](https://github.com/DLTcollab/sse2neon) — *DLTcollab* — SSE→NEON shim for arm64. *MIT.*
- [**GamepadMotionHelpers**](https://github.com/JibbSmart/GamepadMotionHelpers) — *JibbSmart* — gyro/motion input. *MIT.*
- [**SlotMap**](https://github.com/SergeyMakeev/SlotMap) — *SergeyMakeev*. *MIT.*
- [**FreeType**](https://freetype.org) (via [freetype-windows-binaries](https://github.com/ubawurinna/freetype-windows-binaries)) — font rasterization. *FreeType License.*

**Licensing note.** Because it incorporates N64ModernRuntime and the
Zelda64Recomp/drmario64_recomp app scaffold (both **GPLv3**), this project as a
whole is distributed under the **GPLv3** (see [`app/COPYING`](app/COPYING)). RT64 is
MIT and the bundled UI/utility libraries are MIT/zlib as noted above. This repository
contains **no Nintendo code or assets** — you supply your own ROM at build time. See
[docs/LEGAL.md](docs/LEGAL.md).
