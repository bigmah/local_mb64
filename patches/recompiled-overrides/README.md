# Recompiled-function overrides

These are hand-written replacements for specific **recompiled** functions, applied
by `mb64-build recompile` (after N64Recomp + the libc renames, before the
preemption pass).

They exist because a few libultra/libcart functions can't run as-is under the
recompiler and need to call into our native glue instead:

| function | does | glue |
|----------|------|------|
| `osContGetReadDataEx` | fill `OSContPadEx[]` from the host input snapshot (the PIF buffer is never populated) | `mb64_get_pad_ex` (mb64_si/mb64_input.cpp) |
| `cart_card_init` | open/format the host-backed virtual SD card | `mb64_sd_init` (mb64_sdcard.cpp) |
| `cart_init` | ensure the virtual SD card is ready | `mb64_sd_init` |
| `cart_card_rd_dram` | SD read | `mb64_sd_read` |
| `cart_card_wr_dram` | SD write | `mb64_sd_write` |

## Why name-keyed (not a file/line patch)

N64Recomp splits the ~5,100 functions across `funcs_0.c … funcs_55.c`, and the
**split shifts between tool versions** (a single added function cascades every
later function into a different file). So a `funcs_28.c`-pinned patch silently
rots on a regen. Instead, the orchestrator matches each override **by function
name** (which is derived from the ELF address and is stable) and replaces that
function wherever it landed.

Each `<name>.c` here is a complete `RECOMP_FUNC` definition; preemption checks are
omitted on purpose (the preemption pass re-adds them downstream). To add one:
drop a `RECOMP_FUNC <ret> <name>(...) { ... }` file named exactly `<name>.c`.

> **Build-verify note:** these were captured from a working build that used a
> different N64Recomp version than `tools/bin/N64Recomp`. The overrides are
> self-contained (no carried version-specific code), so they should compile
> against any regen — but confirm with a full build on a real machine.
