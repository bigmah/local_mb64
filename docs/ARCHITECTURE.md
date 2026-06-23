# Architecture

## The big picture

Mario Builder 64 is a pure **N64-target** decompilation (HackerSM64 lineage). It
has no PC/native build path, and its engine has diverged too far from the SM64
PC-port family to cheaply graft a hand-written platform layer onto it. So instead
of porting the engine, we **statically recompile** the game binary into native
code with [N64: Recompiled](https://github.com/N64Recomp/N64Recomp) and run it on
the **RT64** renderer + **N64ModernRuntime** — the same stack behind *Majora's
Mask Recompiled* and *Dr. Mario 64 Recompiled*.

This is **not an emulator**. N64Recomp translates the game's MIPS code into C
ahead-of-time; the result is an ordinary native program. RT64 is a high-accuracy
renderer that HLE-interprets the game's display lists onto the GPU.

```
  baserom.us.z64 (your ROM) ──┐
                              ├─►  decomp build (native MIPS toolchain, no Docker)
  vendor/Mario-Builder-64 ────┘        │   GRUCODE=f3dzex/f3dex2 (F3DEX2 fam) + S2DEX
                                       ▼
                          build/us/mb64.us.elf  (symbols)  +  mb64.us.z64 (ROM image)
                                       │
                 ┌─────────────────────┼───────────────────────────┐
                 ▼                      ▼                           ▼
         N64Recomp(elf)         RSPRecomp(audio ucode)      RT64 HLEs the graphics
         → RecompiledFuncs/*.c   → aspMain.c                ucode at RUNTIME (no recomp)
                 │                      │
                 └──────────┬───────────┘
                            ▼
   CMake app  =  RecompiledFuncs + PatchesLib + librecomp + ultramodern + rt64 (+ SDL2)
                            │
                            ▼
                  MarioBuilder64.app  (native arm64 macOS, Metal)
                            ▲
   Rust + Dioxus launcher ──┘   (ROM provisioning, settings, launch — separate process)
   Rust build-orchestrator CLI  (drives the whole left-hand pipeline reproducibly)
```

## Components

| Component | Language | Ours? | Role |
|-----------|----------|-------|------|
| `vendor/Mario-Builder-64` | C / MIPS | no (submodule) | the game; built to an ELF+ROM |
| **N64Recomp** | C++ | no | MIPS ELF → C source |
| **RSPRecomp** | C++ | no | audio/signal RSP microcode → C |
| **RT64** | C++ / Metal | no | runtime HLE renderer (native Metal on macOS) |
| **N64ModernRuntime** (`librecomp` + `ultramodern`) | C++ | no | libultra reimpl + recomp runtime |
| **recomp config** (`recomp/*.toml`, syms, overlays, patches) | TOML/C | **yes** | tells N64Recomp how to recompile MB64 |
| **launcher** | **Rust + Dioxus** | **yes** | ROM provisioning, settings, launch |
| **tools / orchestrator** | **Rust** | **yes** | automates the build pipeline |

## Key design decisions

- **Microcode is already friendly — MB64 uses the F3DEX2 family, not F3DEX3.**
  Verified directly in the source: MB64 v2.3.0's `Makefile` sets `GRUCODE ?= f3dzex`
  (Fast3DZEX2) and offers only `f3dex/f3dex2/f3dex2pl/f3dzex/super3d/l3dex2` — every
  option defines `F3DEX_GBI_2` (the F3DEX2 command set), plus `S2DEX_GBI_2` for text.
  **There is no F3DEX3 in MB64.** F3DZEX2 is the same family Majora's Mask uses —
  RT64's flagship-tested path. If RT64 is ever fussy about the ZEX variant, we pin
  `GRUCODE=f3dex2` (plain Fast3DEX2) — a one-line change. This was the research's
  top "project-defining" risk; verifying the actual config demoted it to minor.
- **Native toolchain, no Docker.** HackerSM64's wiki only documents a Docker/VM path
  for macOS, but MB64 already ships a native arm64 macOS MIPS linker
  (`tools/mips64-elf-ld-arm`, a Mach-O arm64 binary) and its Makefile accepts a
  `mips-linux-gnu`/`mips64-elf` prefix. So we get a native MIPS cross-compiler
  (crosstool-ng `mips64-elf`, or clang targeting mips — N64Recomp explicitly supports
  "modern clang targeting mips" output) and build the decomp directly on macOS.
  Docker stays as a last-resort fallback only.
- **Graphics microcode is HLE'd, not recompiled.** Only CPU code and *audio*
  microcode go through N64Recomp/RSPRecomp. RT64 interprets graphics display lists
  at runtime — that's why microcode support (F3DEX2 vs F3DEX3) is a renderer concern.
- **Rust is the glue and the face, not the engine.** The hot path (recompiled code,
  RDRAM simulation, Metal rendering) is C++ we consume. Rust handles everything
  *around* it: the launcher UI and the build orchestration. The launcher runs the
  game as a **separate child process** (a WebView can't host the native render loop).
- **CMake template: `drmario64_recomp`** (AngheloAlf) — the closest non-Zelda
  reference app; smaller and simpler than Zelda64Recomp.

## Risk map

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| ~~RT64 F3DEX3 unsupported~~ (debunked: MB64 is F3DEX2-family) | low | low | pin `GRUCODE=f3dex2` if the f3dzex variant misbehaves |
| S2DEX / 2D-UI render bugs | med | medium | fidelity bug, not a blocker; per-call workarounds / RT64 fixes |
| Custom audio ucode + editor/save patches | medium | medium | defer audio (boot silent first); `RSPRecomp` overlays are proven |
| Zero SM64-recomp prior art | certain | medium | the *class* of game (F3DEX2 + std audio) recompiles cleanly elsewhere |
| macOS Metal per-game bugs | low–med | medium | validate RT64-Metal on known-good content first |

See [BUILD.md](BUILD.md) for the milestone plan and the fail-fast spike that
attacks the top risks first.
