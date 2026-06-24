# Build & milestone status

> Living document. Reflects the **plan of record** and where we actually are.

## Where we are

**Phase: bring-up / de-risking spike.** No playable build yet. The work so far has
established the approach (static recompilation via N64Recomp + RT64), vendored the
sources, scaffolded the Rust side, and is validating that the C++ renderer stack
compiles natively on this Apple Silicon machine.

| What | Status |
|------|--------|
| Approach + architecture decided | ✅ (see [ARCHITECTURE.md](ARCHITECTURE.md)) |
| MB64 vendored (`vendor/Mario-Builder-64`, pinned) | ✅ |
| MB64 microcode verified = F3DEX2 family (not F3DEX3) | ✅ top risk debunked |
| Rust workspace + `mb64-build doctor` | ✅ first cut |
| RT64 (Metal) compiles natively here | ✅ `rt64.a` + 56 Metal shaders |
| N64Recomp + RSPRecomp tools build | ✅ |
| `baserom.us.z64` provided + SHA-1 verified | ✅ |
| Native MIPS toolchain (no Docker) | ✅ binutils 2.43 + gcc 14.2 (`mips64-elf-`, from source via `mb64-build install-toolchain` → `~/.mb64/toolchain`) |
| M0: build `mb64.elf` + `mb64.z64` | ✅ clean build, microcode f3dzex |
| M1a: **N64Recomp → native C** | ✅ **rc=0, 5106 funcs → 56 C files (470k lines)** |
| M1b: build the app (link recomp C + runtime + RT64) | ⬜ next |
| M2: first frame on screen | ⬜ |
| M3: playable editor + Dioxus launcher | ⬜ |

### The recomp config (`recomp/mb64.us.toml`) — what it took to reach rc=0
- `entrypoint = 0x80124330`, `elf_path = mb64.elf`
- **19 `function_sizes`**: hand-asm stubs that are NOTYPE/size-0 (entry_point, __n64Assert, roundf, fcr_*, slidec decompressor) — N64Recomp skips size-0 symbols.
- **15 `patches.instruction`**: GCC-14 emits `teq $zero,$zero` unreachable-traps that N64Recomp can't recompile → nop them (error paths, don't run in normal play).
- **19 `stubs`**: libcart flashcart/SD driver (`cart_*`/`ci_*`/`ed_*`/`sc64_*`/`sd_*`) — jump tables N64Recomp can't size, and irrelevant on PC.
- **1 `ignored`**: `ipl3_entry` (IPL3 boot ROM — N64ModernRuntime handles boot itself).

### M1b — build the app (in progress)
The app scaffold is copied from the `drmario64_recomp` template into `app/` (runtime
+ RT64 + the glue we'll adapt). What's done / known:
- **Recomp output staged** into `app/RecompiledFuncs/` — N64Recomp emits `funcs_*.c`,
  `funcs.h`, `lookup.cpp`, and **`recomp_overlays.inl`** (the section/overlay table),
  so `register_overlays.cpp` is a trivial wrapper around the generated `.inl`.
- **IPA-clone fix:** GCC-14's `-fipa-cp`/`-fipa-sra`/partial-inlining emit dotted clone
  names (`foo.constprop.0`, `foo.isra.0`) that aren't valid C identifiers, so the
  recompiled C wouldn't compile. Fixed by adding `-fno-ipa-cp -fno-ipa-sra
  -fno-partial-inlining` to the decomp GCC CFLAGS (Makefile), then rebuild + re-recomp.
- **librecomp does the heavy lifting:** `recomp::register_game(GameEntry{...})` +
  `recomp::start(rsp_callbacks, gfx_callbacks, ...)`. `GameEntry` for MB64 =
  internal_name "MARIO BUILDER 64", `entrypoint_address = 0x80124330`, our function
  lookup, save type TBD.

**Remaining M1b glue to author** (adapt from drmario, **drop the RmlUi `src/ui/` tree** —
we use Dioxus instead): a minimal `main.cpp` (GameEntry + callbacks), the RT64 gfx
callbacks (mostly generic, from `rt64_render_context.cpp`), SDL input callbacks, a
silent-audio stub (defer real audio), `register_overlays.cpp`, and a minimal CMakeLists.
Recompile the **audio microcode** (`RSPRecomp` → `rsp/aspMain.cpp`) only once video works.
Then iterate undefined symbols → link → **M2: first RT64/Metal frame.**

## Toolchain (native macOS, no Docker)

The C++ runtime/renderer build natively. The only cross-compiler we need is for the
**MIPS decomp build**, and we get it natively (HackerSM64's wiki only documents
Docker, but MB64 ships a native arm64 MIPS linker and accepts a standard prefix).

Already present on this machine: Xcode + `clang` 17, `cmake` 4.3, `ninja`, `sdl2`,
`glew`, `pkg-config`, Rust 1.93.

MIPS cross-compiler — **must be real GCC** (we tried clang; HackerSM64 uses
GCC-only inline asm like `asm("f10")` and assembler flags like `-mdivide-breaks`
that clang rejects). **Do not use the `tehzz/n64-dev` tap** — it pins gcc 10.2.0 /
binutils 2.37, whose bundled zlib no longer compiles against the macOS 26 SDK
(`_stdio.h` parse errors). Instead let the orchestrator build it from source:
```bash
cargo run -p mb64-build -- install-toolchain
```
This installs the Homebrew build deps (`make coreutils gmp mpfr libmpc cmake ninja
sdl2 pkg-config`) and builds **binutils 2.43 + gcc 14.2** for `mips64-elf` from
source (`--with-system-zlib`, `MAKEINFO=true`, `LIBRARY_PATH` unset, gmp/mpfr/mpc
from Homebrew) into a persistent prefix:

    ~/.mb64/toolchain

(a deliberately space-free path — GNU `configure` rejects build/source/prefix
paths that contain spaces, so `~/Library/Application Support/…` can't be used.)
`build-rom`, `doctor`, and the launcher auto-detect it there (no PATH export
needed), and the decomp builds with `COMPILER=gcc` (default) — no overrides, no
source patches. The build tree is left under `~/.mb64/toolchain-build` (safe to
delete to reclaim ~2 GB).

> Dioxus CLI is only needed to scaffold the launcher UI (later): `cargo install dioxus-cli`.
> Running the launcher itself is just `cargo run -p mb64-launcher`.

> **CMake 4 gotcha (already hit):** this machine has CMake 4.3, which dropped
> compatibility with the old `cmake_minimum_required(<3.5)` that bundled libs
> declare. Always configure the C++ app with `-DCMAKE_POLICY_VERSION_MINIMUM=3.5`.
> The `mb64-build` orchestrator does this for you.

## The orchestrator

`tools/mb64-build` is the Rust CLI that drives the whole pipeline:
```bash
cargo run -p mb64-build -- doctor             # verify toolchain + deps + ROM
cargo run -p mb64-build -- install-toolchain  # one-time: build the MIPS cross gcc
cargo run -p mb64-build -- build-rom          # decomp → build/rom/mb64.{elf,z64}
cargo run -p mb64-build -- recompile          # N64Recomp + post-process + RSPRecomp
cargo run -p mb64-build -- build-app          # cmake/ninja → app/build/mario_builder_64
cargo run -p mb64-build -- all                # build-rom → recompile → build-app
cargo run -p mb64-build -- play               # launch the built game
```
Local patches under `patches/` are applied automatically at the right stage
(`Mario-Builder-64-*` before the decomp `make`, `recompiled-*` after N64Recomp,
`N64ModernRuntime-*` before the app cmake).

## Releases — the one-line installer

Users install with:
```sh
curl -fsSL https://raw.githubusercontent.com/bigmah/local_mb64/main/install.sh | bash
```
`install.sh` downloads the prebuilt binaries from the latest release, assembles
`Mario Builder 64 Launcher.app` (both binaries side by side in `Contents/MacOS/`, so
the launcher finds its orchestrator as a sibling — see `bootstrap::orchestrator_path`),
and opens it. Overridable via `MB64_VERSION` (pin a tag), `MB64_TARBALL` (offline
install from a local file), `MB64_APP_DIR` (install location).

`.github/workflows/release.yml` produces what the installer downloads, on every `v*`
tag (or manual `workflow_dispatch`):
- Builds **only our two Rust binaries** — `mb64-launcher` + `mb64-build` — on a
  `macos-14` (Apple Silicon) runner. It does **not** check out the game submodules,
  build the game, or touch a ROM, so nothing copyrighted is fetched or shipped.
- `.github/scripts/package-macos.sh` tars the two binaries into
  `mb64-macos-arm64.tar.gz` (+ a `.sha256`) and attaches them to the Release.
- **Version pin:** CI stamps `MB64_SOURCE_REF=${{ github.ref_name }}` into the
  launcher (read via `option_env!` in `bootstrap::source_ref`; `launcher/build.rs`
  forces a rebuild when it changes). On first run the launcher clones this repo at
  exactly that ref, so the bundled `mb64-build` always matches the source it builds.
  Unset (dev builds) → tracks `main`.

On the user's Mac the launcher does the rest: detect/offer to install the host
prereqs (Command Line Tools, Homebrew), `git clone --recurse-submodules` into
`~/.mb64/src`, then the normal `install-toolchain` → `all` build from their ROM.

> **No code signing, by design.** The binaries carry only the toolchain's anonymous
> ad-hoc signature (the minimum the arm64 kernel needs to run a binary — no identity,
> no name). We don't use a Developer ID or notarize: a Developer ID certificate would
> embed the account's legal/entity name in the app, and it isn't needed here. Because
> the installer fetches the binaries with `curl` (which doesn't set the
> `com.apple.quarantine` flag, unlike a browser download), Gatekeeper doesn't prompt —
> the app just opens. This is why the install path is a `curl | bash` one-liner rather
> than a `.dmg`.

## The de-risking spike (do before sinking time in)

1. **RT64-Metal compiles here?** — build the RT64 renderer from source on this
   machine. *(in progress)*
2. **F3DEX2 MB64 builds?** — build the decomp forced to a known-good microcode and
   confirm the ROM is sane. *(needs the ROM)*

If both pass, the path is green. If RT64-Metal can't build/run here, we'd pivot to
the SM64 decomp PC-port route (mature Apple Silicon support) instead.

## Milestones

- **M0 — build `mb64.us.elf` + `mb64.us.z64`** from the decomp (native MIPS
  toolchain, `GRUCODE=f3dex2`, your US ROM). Gates everything. *MVP deliverable.*
- **M1 — first recompiled native binary.** Author `recomp/mb64.us.toml`; iterate
  `manual_funcs`/`function_sizes` until N64Recomp emits clean C; link with
  `librecomp`/`ultramodern`/`rt64`; *it launches.* (big)
- **M2 — first frame.** RT64 HLE draws one frame of MB64 over Metal. The true
  renderer go/no-go. Boot silent (audio deferred). (big)
- **M3 — playable editor.** Input wired, editor reachable, save format patched,
  audio via RSPRecomp, Dioxus launcher does real ROM provisioning + settings.

## What we need from you

- Your legal **US Super Mario 64 ROM** as `baserom.us.z64` at the project root
  (build-time only; SHA-1 `9bef1128...`; `mb64-build doctor` verifies it).
