# Mario Builder 64 → macOS — Agent Handoff

Read this top-to-bottom before touching anything. It is self-contained.

---

## 0. TL;DR

We are building a **standalone native macOS app** of [Mario Builder 64](https://github.com/arthurtilly/Mario-Builder-64)
(an SM64 ROM hack / in-game level editor) via **static recompilation** (N64Recomp +
RT64 + N64ModernRuntime — the *Majora's Mask Recompiled* tech). **Not an emulator.**

**Current state: the game RENDERS, RUNS its main loop, takes INPUT, and plays AUDIO —
functionally playable.** The native arm64 executable boots SDL+Metal, runs the recompiled
MB64 code into its per-frame game loop (`gGlobalTimer` advances steadily), RT64 renders its
display lists (real geometry — hundreds of verts / thousands of tris per frame), keyboard +
SDL game controller drive the N64 controller, and the recompiled `aspMain` synthesizes
audio out through SDL (verified non-silent). Stable for 25s+ of play (killed with SIGKILL to
avoid the teardown crash below). **Visual + audio QA still needs a human** — this dev
environment can't `screencapture` ("could not create image from display") and has no
speakers wired to me; functional correctness was verified by probing game memory
(`gGlobalTimer`, `gControllerPads[0].button`) and audio sample peaks. **Run it on a machine
with a display and play it.**

Four blocking bugs were fixed this session, in order: (1) the Yay0 decompressor
(`slidstart`) was split into broken fragments → all compressed assets were zero → RT64
crashed on the first textured draw; (2) once rendering, the game was FROZEN at
`gGlobalTimer=1` because `osContStartReadDataEx`'s controller read waited forever on an SI
completion the stubbed `__osSiRawStartDma` never posted; (3) no input plumbing; (4) silent
(no audio ucode). Details below and in §6/§7.

**Keyboard map** (no remap UI yet; the Dioxus launcher will own this): stick = arrow keys;
A = X; B = C; Z = Z/Shift; L = A; R = S; Start = Enter; C-buttons = I/J/K/L; D-pad = T/F/G/H.
Xbox-style game controllers work too (left stick, A→A, X→B, triggers→Z, shoulders→L/R,
right stick→C, dpad→dpad).

**✅ RESOLVED — the game-freeze (frozen at `gGlobalTimer=1`).** After rendering was fixed,
the game ran exactly one frame then stalled. `thread5_game_loop` → `osContStartReadDataEx`
(MB64 has GameCube-controller support, so it uses the `…Ex` controller API ultramodern does
NOT provide → the game's own PIF-based version is recompiled and runs) → blocked in
`osRecvMesg` waiting for the SI (controller) read to complete. Our `mb64_ultra_stubs.cpp`
stubbed `__osSiRawStartDma` as a no-op, so the SI completion message was never posted and
the read hung forever. **Fix:** `__osSiRawStartDma_recomp` now calls
`ultramodern::send_si_message(rdram)` — every SI DMA completes with an SI interrupt, and
MB64's read does exactly one `osRecvMesg` per `__osSiRawStartDma` (WRITE then READ), so this
is balanced. The game's main loop now advances.

**✅ INPUT (M3b done).** `app/src/mb64_input.cpp` snapshots SDL keyboard + game-controller
state (on the gfx thread, in `update_gfx`) into atomics; `mb64_si.cpp::mb64_get_pad_ex`
reads that snapshot. Because MB64 reads via `osContGetReadDataEx` (PIF-based, which we don't
emulate), the recompiled `osContGetReadDataEx` in `funcs_28.c` is OVERRIDDEN (early-return
injection at the top of the function, original body left as dead code) to fill
`OSContPadEx[]` directly from `mb64_get_pad_ex` — `data` ptr in `r4`, stride 10
(`gControllerPads[4]` spans 0x28), 4 controllers, port 0 connected. Verified:
`gControllerPads[0].button` reflects injected input. (Hand-edit to a generated file → wiped
on recomp regen; re-apply, or move to a librecomp function-override.)

**✅ AUDIO (M3a done).** MB64's audio microcode is the standard SDK `aspMain` (text 0xE20 @
ROM 0x1E4E0 / VMA 0x80141810 — same size as drmario's, identical ucode). `app/aspMain.us.toml`
+ `tools/bin/RSPRecomp` → `app/rsp/aspMain.cpp` (reused drmario's `extra_indirect_branch_targets`
— RSPRecomp ran clean, confirming identical ucode). `get_rsp_microcode` returns `aspMain`
for `M_AUDTASK`; `app/src/mb64_audio.cpp` (adapted from the template) opens the SDL audio
device and resamples/queues samples. Verified non-silent (sample peaks climb to ~13k once
the game's sequence player runs). NOTE: audio was silent while the game was frozen — it only
produces sound now that the main loop advances.

**✅ EDITOR SAVES / virtual SD card (M3c).** Menu nav worked but **"Create" (enter the level
editor) crashed**: MB64 saves `.mb64` levels on the flashcart SD via FatFS + libcart
(`src/mb64/file.c` `cart_init()`+`f_mount`; `src/libcart/ff/diskio.c` →
`cart_card_init`/`cart_card_rd_dram`/`cart_card_wr_dram`), and those `cart_*` were no-op
stubs, so `f_mount` failed (`gMountSuccess`=FR_DISK_ERR) and the editor crashed on its first
file op. **Fix:** `app/src/mb64_sdcard.cpp` backs a real host file `mb64_sd.img` (cwd),
created + FAT16-formatted on first run (self-contained formatter — `newfs_msdos` only does
devices, `FF_USE_MKFS`=0); the recompiled `cart_card_init/rd_dram/wr_dram` + `cart_init`
stubs (`funcs_14/12/30/16.c`) are hand-overridden to call `mb64_sd_init/read/write`. Sector
copies use the `^3` RDRAM byte-swizzle (cart DMA is raw big-endian order). Verified
`gMountSuccess`=FR_OK and the game created its `MarioBuilder64Levels` directory in the
image — levels persist in `app/mb64_sd.img` (a real FAT16 fs). The SD card mount was
necessary but NOT sufficient — "Create" still crashed afterward; see the next entry.

**✅ RESOLVED — "Create" crashed on a MISSING INDIRECT-CALL FUNCTION (`proutSprintf`),
not graphics/filesystem. The level editor now OPENS (user-confirmed 2026-06-23).** After the
SD fix, "Create" still aborted with `Failed to find function at 0x80132358` →
`libc++abi: terminating` (SIGABRT) on thread `mb64_game`. KEY DEBUGGING NOTE: the `.ips`
crash reports were useless here — every quit triggers the teardown race (below), whose
`objc_release` fault MASKS the real cause; you only see the true fault by running under lldb
(`gtimeout 300 lldb -b -o run -o "thread backtrace all" ./build/mario_builder_64`) so a hard
fault stops the debugger before the clean-shutdown teardown runs. `0x80132358` is
**`sprintf`+0x58 = libultra's static `proutSprintf` output callback**, which comes from the
prebuilt **libnustd** with its symbol stripped — so N64Recomp folded it into `sprintf`
(0x80132300..0x80132380) and registered NO function at its address. `sprintf` passes
`&proutSprintf` to `_Printf`, which calls it INDIRECTLY through the lookup table →
`get_function(0x80132358)` fails. It only surfaces in the editor because that's the first
`sprintf()` on a fresh save (`show_tip`/toolbar format text; the pre-Create menus don't call
it). `_Printf`'s only real caller is `sprintf` (`osSyncPrintf`/`__osSyncVPrintf`/`rmonPrintf`
are no-op stubs in this build), so there's no sibling-prout bug. **Fix (surgical, no recomp
regen):** reimplemented `proutSprintf_recomp` (`memcpy(dst,src,count); $v0=dst+count`) in
`app/src/mb64_ultra_stubs.cpp` and registered it at `0x80132358` via
`recomp::overlays::add_loaded_function(...)` in
`mb64_overlays.cpp::register_resident_function_addresses` (same API/timing as the
resident-section registration). **Lesson:** prebuilt-library static helpers used as function
pointers (printf prouts, comparators, etc.) have no symbol → N64Recomp absorbs them → an
indirect call aborts with "Failed to find function at <addr>". Fix = reimplement the function
+ `add_loaded_function` at that vram (or, the proper long-term fix, give it a sized symbol /
config entry and regenerate).

**✅ RESOLVED — the first-frame RT64 crash was a BROKEN Yay0 DECOMPRESSOR, not an RT64
HLE bug.** The recomp TOML had given each *internal branch label* of the `slidstart`
(slidec, `src/boot/slidec.s`) Yay0 decompressor its own `function_sizes` entry
(`slidemain2`/`codecheck2`/`pressdata2`/`pressloop2`/`loopend2`/`pressloop3`). N64Recomp
therefore chopped the one decompressor function into 7 fragments, turning its internal
loop branches into (broken, would-be-infinitely-recursive) function calls and leaving
`slidstart` itself a SETUP-ONLY STUB that returned without decompressing anything. Result:
every Yay0-compressed asset (segment 2, textures, geometry) decompressed to ZEROS. The
game's *logic* ran fine (uncompressed code DMAs in normally) but it had no art, so the
first frame that drew real content — a `gsSPDisplayList(0x02001600)` = `dl_shadow_circle`
in segment 2 — pointed RT64 at a region of all-zeros; RT64 walked off the end of the
"display list" into garbage and crashed in `drawIndexedTri`. **Fix:** `slidstart` must be
ONE function (0x80125ee0..0x80125f90, size 0xB0) with the internal labels as goto targets.
Done two ways: (1) `recomp/mb64.us.toml` now has a single `slidstart` size=0xB0 entry (the
6 label entries removed) — validated by regenerating into a temp dir: N64Recomp emits the
correct single function with internal `goto`s; (2) the live `app/RecompiledFuncs/funcs_51.c`
has a hand-reassembled single-function `slidstart` (faithful transcription, matches the
regenerated output) so the current build is correct without a full regen. The dead
`slidemain2_*`/`codecheck2_*`/... fragments still sit in `funcs_0.c` (unused; a regen
removes them). **How it was found:** instrumented RT64's HLE `processDisplayLists` +
`runDl` to dump every command, saw RT64 walk linearly through ~7000 zero/garbage commands
after a `seg=2 → 0x43e0f0` jump; dumped RDRAM (`State::dumpRDRAM`) and confirmed the whole
segment-2 region was zero; `nm` resolved `0x02001600` → `dl_shadow_circle`; traced the
load path to `load_segment_decompress` → `slidstart`; read the recompiled `slidstart` and
saw it was a setup-only stub. All instrumentation has been reverted.

**Earlier framing (now obsolete):** this was previously thought to be an RT64
display-list-parsing / microcode bug. It was not — RT64's F3DZEX2 HLE is fine; it was
faithfully parsing a display list that pointed at zeroed (un-decompressed) memory.

**✅ RESOLVED — the audio busy-wait deadlock (scheduler preemption).** Level loading
busy-spun in `wait_for_audio_frames` (`while (gAudioFrameCount <= 0) {}`) waiting on the
audio thread, but the recomp's COOPERATIVE scheduler (switches only at syscalls) could
never run the audio thread. Fixed by adding **timer-driven preemption** — see §7
"Scheduler preemption". The game now runs its main loop.

**(Former blocker — the "RT64 crashes on the first real-frame DL / garbage `cnt=128`
vertex at a zeroed `0x25c8c4`" symptom — is the Yay0-decompressor bug resolved above.
RT64 was faithfully parsing a display list that pointed at un-decompressed (zero) memory;
once `slidstart` decompresses correctly, segment 2 holds the real `dl_shadow_circle` and
the geometry renders. No RT64 changes were needed.)**

**Other known issues:**
- **✅ FIXED — crash on process teardown** in RT64's "RT64 Workload"/Present/Idle Metal
  threads (`objc_release`/`objc_autoreleasePoolPop` during `_pthread_exit`). It fired on
  every clean shutdown (an RT64 Metal object-lifetime race; a per-iteration
  `NS::AutoreleasePool` attempt failed earlier). Sidestepped: `update_gfx` (mb64_main.cpp)
  now exits immediately on `SDL_QUIT` via `ultramodern::error_handling::quick_exit(...,
  EXIT_SUCCESS)` (`std::_Exit(0)` on macOS) after `fflush(nullptr)`, skipping RT64's racy
  Metal teardown — verified exit 0 with no `.ips` (was exit 139 SIGSEGV before). SD-card
  saves are durable (flushed per write). Masks rather than fixes the RT64 race — fine for a
  single-window app.
- Audio: `thread4_sound` runs; `get_rsp_microcode` returns a no-op silent ucode (real
  synthesis = M3a).

**What it took to get from "nullptr render stub" to a (first) rendered frame:** the
render context was a clean adaptation of the template (§5). The hard part was getting
the game to BOOT far enough to render — a cascade of fixes, the biggest being a
**stale ROM** (the provisioned `mb64.z64` was from an older decomp build than the
recomp). See §7 "Boot bring-up fixes (M2)".

Constraints from the user: **no Docker**, **no emulator**, and **new code we author
should be Rust** (the launcher + tooling) — the C/C++ engine+runtime are existing code
we *use*. The Rust **launcher (Dioxus)** comes after the game renders.

---

## 1. Environment (what's installed, what's ephemeral)

Machine: **Apple Silicon (arm64), macOS 26.5, full Xcode 26.2 + CLT 26.5**, Homebrew.
Installed: `clang`(Apple)+`clang++`, **Homebrew LLVM** at `/opt/homebrew/opt/llvm/bin`
(has the MIPS backend; Apple clang does NOT), `cmake` 4.3, `ninja`, GNU **`make` 4.4.1**
at `/opt/homebrew/opt/make/libexec/gnubin/make` (Apple's 3.81 is too old), `sdl2`,
`glew`, `mips-linux-gnu-binutils`, `crosstool-ng`, coreutils (`gtimeout`, etc.).

### ⚠️ EPHEMERAL — lives in `/tmp`, will vanish on reboot. Rebuild if missing:
- **`/tmp/n64tc/`** — the **MIPS cross toolchain** (`mips64-elf-gcc` 14.2 + binutils 2.43),
  built from source. Needed only to **rebuild `mb64.elf`** (Section 4). Rebuild recipe
  is in `~/.claude/.../memory/n64recomp-macos-feasibility.md` and Section 7 below.
- **`/tmp/drmario-spike/`** — clone of `AngheloAlf/drmario64_recomp` (the recomp app
  TEMPLATE). NOTE: `app/` is already a copy of this, so you usually don't need it.
- **`/tmp/n64recomp-build/`** — where `N64Recomp`/`RSPRecomp` were built. **Already
  copied** to `tools/bin/` (persistent), so you don't need this.

### Persistent (in the repo):
- `tools/bin/{N64Recomp,RSPRecomp}` — the recompiler tools (arm64 binaries).
- `vendor/Mario-Builder-64/` — MB64 source (git submodule, pinned `69f83d7`), **with two
  edits** (see Section 7). Its `build/us_n64/mb64.{elf,z64}` is the decomp output.
- `build/rom/{mb64.elf, mb64.z64, mb64.us.toml, RecompiledFuncs/}` — recomp inputs+output.
- `recomp/mb64.us.toml` — the canonical N64Recomp config (copy of the above).
- `app/` — the application (CMake project). `app/RecompiledFuncs/` = the recompiled C
  (post-processed, see §7). `app/src/mb64_*.cpp` = OUR glue. `app/lib/` = rt64 +
  N64ModernRuntime (runtime). `app/src/main/`, `app/src/game/`, `app/include/`,
  `app/src/ui/` = the drmario TEMPLATE (reference for adapting; `src/ui` is RmlUi which
  we are NOT using — Dioxus instead).
- `launcher/`, `tools/mb64-build/` — Rust workspace (the `mb64-build doctor` CLI exists;
  the Dioxus launcher is a later milestone).

Nothing is committed to git yet (11 untracked entries; user hasn't asked to commit).

---

## 2. The pipeline (how it all fits)

```
your US baserom.us.z64 ─┐
                        ├─► decomp build (Section 4) ─► mb64.elf (+ mb64.z64)
vendor/Mario-Builder-64 ┘        (mips64-elf-gcc, GRUCODE=f3dzex)
                                      │
                                      ▼
            N64Recomp recomp/mb64.us.toml  ─►  app/RecompiledFuncs/*.c  (470k lines)
                                      │            + funcs.h, lookup.cpp, recomp_overlays.inl
                                      ▼
   post-process renames (§7) ─► CMake app (app/CMakeLists.txt) links:
       RecompiledFuncs + librecomp + ultramodern + rt64(Metal) + SDL2 + our glue
                                      │
                                      ▼
                       app/build/mario_builder_64  (native arm64)
```

---

## 3. Build & run the app (this works TODAY)

```bash
cd ~/dev/smb/app
cmake -B build -G Ninja -DCMAKE_POLICY_VERSION_MINIMUM=3.5 -DCMAKE_PREFIX_PATH=/opt/homebrew
cmake --build build -j8 --target mario_builder_64      # ~10 min first time (rt64 is big)
cp ~/dev/smb/build/rom/mb64.z64 .                       # the app looks for the ROM in cwd
./build/mario_builder_64                                # segfaults in gfx_thread_func (null render ctx)
```
Backtrace the crash: `gtimeout 30 lldb -b -o run -o bt ./build/mario_builder_64`.

---

## 4. Rebuild the recomp inputs (only if you change the decomp)

You usually DON'T need this — `app/RecompiledFuncs/` is already generated. Do it only if
you modify `vendor/Mario-Builder-64` or the recomp config.

```bash
# (a) build the ELF — needs /tmp/n64tc (rebuild it per §7 if gone) + a US SM64 ROM
cd ~/dev/smb/vendor/Mario-Builder-64
ln -sf ../../baserom.us.z64 baserom.us.z64              # user supplies baserom.us.z64 at repo root
export PATH=/tmp/n64tc/bin:/opt/homebrew/opt/make/libexec/gnubin:$PATH; unset LIBRARY_PATH
make VERSION=us COMPILER=gcc -j8                        # → build/us_n64/mb64.elf + mb64.z64
cp build/us_n64/mb64.elf build/us_n64/mb64.z64 ~/dev/smb/build/rom/

# (b) recompile  (config addresses are tied to the ELF — see §7 if you rebuilt the ELF)
cd ~/dev/smb/build/rom
../../tools/bin/N64Recomp mb64.us.toml                 # → RecompiledFuncs/  (expect rc=0)

# (c) post-process for macOS, then stage into the app  (see §7 for the renames)
#   sed the libc-name collisions, then: cp -R RecompiledFuncs ../../app/RecompiledFuncs
```

---

## 5. ⭐ YOUR TASK: M2 — the RT64 render context (→ first frame)

The app crashes only because `mb64::renderer::create_render_context` (in
`app/src/mb64_render_stub.cpp`) returns `nullptr`. Replace it with a real `RT64Context`.

**Reference implementation is already in the repo** (the drmario template, copied into `app/`):
- `app/src/main/rt64_render_context.cpp` (~450 lines, defines `zelda64::renderer::RT64Context`)
- `app/include/zelda_render.h` (the `RT64Context` class decl + `create_render_context`)

**Plan:**
1. Copy those two files to `app/src/mb64_render.cpp` + `app/include/mb64_render.h`,
   rename the `zelda64::renderer` namespace → `mb64::renderer`.
2. **Strip the coupling** to things we don't have: `recomp_ui.h` (the RmlUi UI),
   `recomp::mods` / texture-pack code (`enable_texture_pack`, `texture_pack_action_queue`,
   `secondary_*`), and any `zelda64::`-specific config. Keep the core RT64 init +
   the per-task `loadUCodeGBI`/draw path. **RT64 auto-detects the microcode from the
   task (`app->interpreter->loadUCodeGBI(task->t.ucode...)`), so F3DZEX2 needs NO
   game-specific config** — do not hardcode a ucode.
3. Delete `app/src/mb64_render_stub.cpp`; add `mb64_render.cpp` to `app/CMakeLists.txt`'s
   exe sources. You will likely need extra `target_include_directories` (rt64 `src/hle`,
   `src/render`, `src/rhi`) and possibly more rt64 contrib paths — add as the compile errors
   demand (same loop we used all session).
4. Build, run, **iterate to a visible frame.** Expect: undefined symbols (add the rt64
   render hooks / link bits), then runtime issues (the gfx thread now has a real context).
   When a frame appears, that's M2 done.

**Watch items:** the S2DEX 2D editor UI may render wrong first (fidelity bug, not a
blocker — note it and move on); Retina/scaling can cause jitter (target native res).

---

## 6. Roadmap after M2

- **M3a — audio:** `get_rsp_microcode` currently returns `nullptr` (silent). Recompile
  MB64's audio microcode with `tools/bin/RSPRecomp` (mirror the template's
  `aspMain.*.toml` / `app/rsp/aspMain.cpp`), wire it into `get_rsp_microcode` + the audio
  callbacks (`queue_samples`/etc. in `mb64_main.cpp`, currently stubbed).
- **M3b — input:** `get_n64_input` is a zeroed stub. Adapt the template's
  `src/game/input.cpp` (SDL game controller → N64 buttons/stick).
- **M3c — editor + saves:** verify the in-game level editor works and that `.mb64`
  level files load. The `mb64_ultra_stubs.cpp` PI/SI/PFS stubs (return-0) will need real
  implementations for saving levels (SRAM/Flash). Set `GameEntry.save_type` correctly
  (currently `AllowAll`; MB64 likely Sram/Flashram). Compute `rom_hash` =
  `XXH3_64bits(mb64.z64)` (currently `0x0` placeholder; validated at runtime).
- **M4 — Dioxus launcher (Rust):** the user wants the launcher in Rust+Dioxus — ROM
  provisioning (file picker + SHA-1), settings, controller config, Launch button that
  spawns the game as a child process. Scaffold in `launcher/`. Install `dx` via
  `cargo install dioxus-cli`. Keep it a SEPARATE process (a WebView can't host the
  Metal render loop). Extend `tools/mb64-build` (Rust) to automate the §7 post-process.
- **M5 — packaging:** `.app` bundle + signing (`xattr -cr` for local). The template's
  `.github/macos/apple_bundle.cmake` + `ld64` are references; `app/` currently has a stub
  `.github/macos/apple_bundle.cmake` (not wired into our minimal CMakeLists).

---

## 7. Gotchas already solved (DO NOT re-discover these)

**Decomp build (vendor/Mario-Builder-64):**
- Must use **GNU make 4.x** (gnubin), not Apple make 3.81 (misparses `!=` assignments).
- Build with **`COMPILER=gcc`** + the **`/tmp/n64tc` `mips64-elf-` toolchain** (auto-detected).
  clang can't build it (GCC-only inline asm `asm("f10")` + asm flags `-mno-shared`/`-mdivide-breaks`).
- **DO NOT pass `CC=`/`CXX=`** on the make cmdline (breaks the host-tool `armips` C++ link).
- **Makefile edit already applied:** added `-fno-ipa-cp -fno-ipa-sra -fno-partial-inlining`
  to the gcc CFLAGS — else GCC-14 emits dotted clone names (`foo.constprop.0`) that are
  illegal C identifiers and the recompiled C won't compile. **Keep this edit.**
  (A second edit to `src/engine/math_util.c` was REVERTED — gcc handles the original asm;
  the clang version is saved in `patches/clang-math_util-fp-asm.patch`, unused.)
- `extract_assets.py` overwrites many tracked asset files — ignore that churn.

**N64Recomp config (`recomp/mb64.us.toml`)** — what it took to reach `rc=0`:
- entrypoint `0x80124330`, `elf_path = mb64.elf`.
- 19 `[[input.function_sizes]]` (size-0 hand-asm stubs: entry_point, __n64Assert, fcr_*, slidec).
- 15 `[[patches.instruction]]` nop-ing GCC-14 `teq $zero,$zero` unreachable-traps.
- 19 `[patches] stubs` = libcart (`cart_*`/`ci_*`/`ed_*`/`sc64_*`/`sd_*`).
- 1 `ignored = ["ipl3_entry"]` (IPL3 boot ROM — the runtime handles boot).
- ⚠️ **The `patches.instruction` vram addresses are tied to a specific ELF build.** If you
  rebuild `mb64.elf`, code shifts and you must REGENERATE them (the function_sizes/stubs/
  ignored are name-based and stable). Script: objdump for `teq` addrs → map each to its
  containing FUNC via readelf → emit `func`/`vram`/`value=0x0`. (We did this in Python.)
- `stubs`/`ignored` only accept functions N64Recomp actually tracks (sized FUNCs) — NOT
  size-0 symbols and NOT libultra `__osPi*`.

**macOS post-process of the recomp C** (reapply after every recomp; the Rust orchestrator
should automate this) — N64Recomp auto-renames most libc/libultra collisions to `_recomp`,
but two macOS-specific stragglers remain. In `app/RecompiledFuncs/`:
```bash
# plain sed (BSD sed/grep do NOT support \b or [[:<:]] reliably — use plain patterns)
grep -rl __fpclassifyf . | while read f; do sed -i '' 's/__fpclassifyf/mb64_fpclassifyf/g' "$f"; done
grep -rl strncpy       . | while read f; do sed -i '' 's/strncpy/mb64_strncpy/g'           "$f"; done
```
(If a NEW libc collision appears as `error: functions that differ only in their return
type` or `cannot initialize ... recomp_func_t*`, find the offending name and rename it the
same way. There were only these two.)

**App build / link (macOS-specific, all already in `app/CMakeLists.txt`):**
- Configure needs `-DCMAKE_POLICY_VERSION_MINIMUM=3.5` (bundled libs declare cmake_min < 3.5).
- `target_compile_options(rt64 PRIVATE -include stdlib.h)` on APPLE — hlslpp's HLSL_CPU
  scalar path uses `labs` without including a stdlib header. **stdlib.h not cstdlib** (rt64
  has C sources like miniz.c).
- The zstd source dir `app/lib/rt64/src/contrib/zstd/build/` must exist (an earlier
  `rsync --exclude build*` wrongly dropped it; it's restored now — don't re-drop it).
- Exe sources compile with `-Wno-everything -fno-strict-aliasing`; **do NOT use
  `-fms-extensions` on macOS** (breaks the SDK `_string.h` overloads).
- In `mb64_overlays.cpp`, `#include "librecomp/overlays.hpp"` MUST come **before** the
  recomp `.inl` (recomp.h macros otherwise poison the macOS SDK `<string>` strchr overloads).
- `app/src/mb64_ultra_stubs.cpp` provides 11 libultra funcs the runtime lacks (TLB, PI/SI
  raw IO, controller-pak/PFS, `recomp_syscall_handler`) as no-op/return-0 — MB64 calls
  these, drmario didn't. Fine for first frame; revisit for real save/controller-pak.

**Boot bring-up fixes (M2)** — what it took to get the recompiled game to boot to a
rendered frame, in the order discovered (each was a distinct crash; all are in OUR
code or are documented hand-stubs, the runtime libs are unmodified):

1. **⚠️ STALE ROM (the big one).** `build/rom/mb64.z64` was from an OLDER decomp build
   than `build/rom/mb64.elf` (which the recomp was generated from). The recomp runs
   native code from the ELF but reads game DATA from the ROM by ELF address — a
   mismatched ROM → garbage data → cascading crashes (the first visible one was a
   bogus DMA-table count in `setup_dma_table_list`). The matching ROM is the one built
   alongside the ELF: `vendor/Mario-Builder-64/build/us_n64/mb64.z64`. **Always provision
   the ROM that matches the ELF the recomp came from.** Sanity check: ELF `0x80124330`
   and ROM `0x1000` must contain identical bytes; the recomp's entry immediate
   (`funcs_35.c` `ADD32(ctx->r8, -0X73F0)`) must match the ROM. Current matching pair:
   ELF/ROM md5 `0a3fe5e0…` / `4b2dd85a…`; `rom_hash = 0xd82b295c5a4d30f5`.
2. **Entrypoint sign-extension** (`mb64_main.cpp` `get_entrypoint_address`): KSEG0
   addresses must be stored **sign-extended** in the 64-bit `gpr` (`(gpr)(int32_t)0x80124330`),
   matching the recomp's `MEM_*` model (it subtracts the sign-extended `0xFFFFFFFF80000000`).
   The bare `0x80124330u` zero-extends → the boot ROM→RDRAM copy writes 2GB past RDRAM.
3. **Resident section function registration** (`mb64_overlays.cpp`
   `register_resident_function_addresses`, wired as the GameEntry `on_init_callback`):
   librecomp's `init()` registers functions with ONE linear `load_overlays` call, which
   only maps the first contiguous section. SM64's later sections (section_6 "engine"
   etc.) have a different rom→ram offset, so their funcs land at the wrong vram and every
   indirect call (thread entries, the game loop) fails `get_function()`. Fix: re-register
   every resident function at `section.ram_addr + func.offset` via `add_loaded_function`.
4. **Deferred `start_game`** (`mb64_main.cpp` `start_game_deferred`, called from
   `mb64_render.cpp::update_screen` after a few frames): no in-engine UI means we start
   the game ourselves, but starting before ultramodern's VI thread has run `set_dummy_vi`
   once makes `vi_thread_func` deref a null `OSViMode`. Gating on the first few presented
   (dummy) frames guarantees the VI mode is set first.
5. **`detect_emulator` stub** (hand-edit in `RecompiledFuncs/funcs_25.c`): probes Parallel
   Launcher libpl (`0x1FFB0000`) and raw DPC regs (`0xA4100018`) — raw MMIO that can't work
   under recomp. Stubbed to return "no emulator" (r2=0). TODO: formalize as a `[patches]`
   stub in `recomp/mb64.us.toml` + regenerate.
6. **Audio NOT disabled (it can't be).** First attempt stubbed `thread4_sound`, but the
   game's LEVEL LOADING touches the sound banks (`stop_sounds_in_continuous_banks`) and
   hangs if audio is uninitialized — so the audio system must run. The earlier KSEG1
   `audio_init` crash turned out to be ROM-mismatch garbage; with the correct ROM,
   `audio_init`/`sound_init` run fine. The audio RSP task is handled by a no-op silent
   ucode in `mb64_main.cpp` `get_rsp_microcode` (returning `nullptr` makes `run_task`
   abort the app). Real synthesis (RSPRecomp'd aspMain) is M3a. This is what leads to the
   current deadlock blocker (see §0): the audio system runs but the game busy-waits on it.

  NOTE: stub (5) is a hand-edit to a GENERATED file — wiped if you regenerate the recomp.
  Re-apply it, or better, move it into the TOML `[patches] stubs`.

**Scheduler preemption (breaks SM64 audio busy-waits).** The recomp runs N64 threads
cooperatively (only one green thread runs at a time; it switches only at syscalls via
`ultramodern::check_running_queue`). SM64 busy-spins in places (`wait_for_audio_frames`,
audio DMA waits) expecting hardware priority preemption, which deadlocked the spinner
against the higher-priority thread it waits on. The fix emulates preemption:
- **`ultramodern/src/scheduling.cpp`**: `volatile int recomp_should_preempt` + a 1ms
  timer thread (`start_preemption_timer`, called from `ultramodern::preinit`) that sets
  it. `recomp_preempt(rdram)` (extern "C") clears the flag, **drains external messages**,
  then `check_running_queue`. The drain (`dequeue_external_messages`) is ESSENTIAL and
  was the subtle bug: `osSendMesg` from a NATIVE thread (VI/SP/DP) only enqueues to
  `external_messages`, which is normally drained by a game thread at its next syscall — a
  busy-spinner never syscalls, so without draining here the VI retrace never wakes the
  main/audio threads and preemption has nothing to switch to.
- **`recomp_interrupts_disabled`** counter (`librecomp/.../ultra_translation.cpp`
  `__osDisableInt_recomp`/`__osRestoreInt_recomp`): `recomp_preempt` skips yielding while
  >0, so we don't preempt inside the game's `osDisableInt` critical sections (we emulate
  *involuntary* preemption, which HW also suppresses while interrupts are masked).
- **Declarations** in `N64Recomp/include/recomp.h`; `start_preemption_timer` in
  `ultramodern.hpp`.
- **The actual yield checks** (`if (recomp_should_preempt) recomp_preempt(rdram);`) are
  inserted at every BACKWARD `goto` (loop back-edge) in the recompiled C — currently via
  a **POST-PROCESS** (`tools/insert_preempt.py`, run from `app/`: tracks labels per
  function, inserts before gotos whose target was already seen; 4876 insertions). ⚠️ This
  is a transform on
  GENERATED files — **wiped if you regenerate the recomp.** Proper home: emit it in
  N64Recomp's CGenerator (`recompilation.cpp`, the `cpu_b`/`cpu_j` + conditional-branch
  cases) for any backward branch, gated on `recomp_should_preempt`. Until then, re-run the
  post-process after every recomp regeneration. (Backward-only keeps overhead low and
  avoids most critical sections.)

**Rebuild `/tmp/n64tc` if it's gone** (full recipe in
`memory/n64recomp-macos-feasibility.md`): build binutils 2.43 then gcc 14.2 from source
with `--with-system-zlib --without-zstd`, `MAKEINFO=true`, `--with-gmp/mpfr/mpc=/opt/homebrew/opt/*`,
`unset LIBRARY_PATH`, prefix `/tmp/n64tc`. The decomp links its OWN libgcc (`lib/gcclib/`)
so the gcc build's `all-target-libgcc` failing is harmless.

---

## 8. Key references
- `docs/ARCHITECTURE.md`, `docs/BUILD.md`, `docs/LEGAL.md` — design, status, ROM/legal.
- `~/.claude/projects/-Users-tonyradtke-dev-smb/memory/*.md` — the durable project memory
  (auto-loaded for the Claude Code agent; same facts as here, more granular).
- Template app: `AngheloAlf/drmario64_recomp`; runtime: `N64Recomp/N64ModernRuntime`;
  renderer: `rt64/rt64`; the reference full app: `Zelda64Recomp/Zelda64Recomp`.

**Legal:** the user supplies their own US SM64 ROM (`baserom.us.z64`, build-time only,
SHA-1 `9bef1128717f958171a4afac3ed78ee2bb4e86ce`). Never commit ROMs or extracted Nintendo
assets (`.gitignore` blocks them). Personal use only.
