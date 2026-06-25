# Legal & ROM posture

This is a **source-available** project, published under the same well-established
conventions as the SM64 decompilation, the SM64 PC port, and the *N64: Recompiled*
family (Zelda64Recomp, drmario64_recomp). We publish **source you build yourself**,
plus our own launcher and build tooling — never game code, assets, or ROMs.

## You supply your own ROM. We ship no Nintendo content.

- Building requires a **legally-owned US Super Mario 64 ROM**, placed as
  `baserom.us.z64` (big-endian `.z64`, SHA-1
  `9bef1128717f958171a4afac3ed78ee2bb4e86ce`).
- The ROM is used **at build time only** — `extract_assets.py` pulls media
  (textures, sounds, sequences, skyboxes, etc.) out of it and bakes them into the
  build. The recompiled app also reads game data from a ROM image at runtime.
- This repository contains **no Nintendo ROM and no assets extracted from one**.
  `.gitignore` blocks `*.z64`/`baserom*` and the extracted-asset folders so they
  can never be committed by accident.

## What is whose

- **Nintendo** owns Super Mario 64 and its assets. We never redistribute them.
- **arthurtilly et al.** own the Mario Builder 64 code. It is referenced here as a
  **git submodule pointer** (`vendor/Mario-Builder-64`) — a URL plus a commit hash,
  **not a copy**. MB64 ships with **no license file → all rights reserved**, so
  cloning this repo fetches MB64 directly from its own upstream; we redistribute
  none of it. Out of courtesy, a public port should credit the MB64 authors and
  ideally seek their blessing.
- **N64Recomp / N64ModernRuntime / RT64** and the bundled UI/utility libraries are
  third-party projects we build against, each under its own license (see the README
  credits). Where this project carries local patches to a GPLv3 dependency
  (e.g. the runtime), those patches are published as source per GPLv3.
- The **new code in this repo** (the native glue in `app/src/mb64_*`, the Rust
  launcher and tooling, and the recomp config) is ours, under **GPLv3**
  (see [`../app/COPYING`](../app/COPYING)).

## What we publish — and what we never publish

**Published (source-available, GPLv3):** the native glue code, the recomp
configuration (`recomp/*.toml`), the Rust launcher and build tooling — including a
**prebuilt launcher binary** (it contains no Nintendo content and no game data; it
only provisions a build and launches the game you build locally) — the docs, and
**submodule pointers** to the game and the runtime/renderer libraries.

**Never published / never distributed** — and blocked by `.gitignore`:

- any **ROM** (`*.z64`, `baserom*`) or **assets extracted** from one;
- the **recompiled game C** output (`RecompiledFuncs/`) — it is derived from
  Nintendo/MB64 code and is regenerated locally from *your* ROM at build time.

In short: sharing the **recipe** (and our own launcher, which carries no Nintendo
content) is fine; sharing the **ROM or assets extracted from it** is not. This is the
same line every reputable decomp/recomp project draws. Not affiliated with or endorsed
by Nintendo.
