// Bridges the host input snapshot (mb64_input.cpp) into MB64's controller read.
//
// MB64 reads controllers via osContStartReadDataEx/osContGetReadDataEx (it has
// GameCube controller support), which ultramodern does not provide — so the
// game's own PIF-based versions run. We don't emulate the SI/PIF transfer, so
// instead: __osSiRawStartDma posts the SI completion (mb64_ultra_stubs.cpp) to
// unblock the blocking read, and the recompiled osContGetReadDataEx is overridden
// (funcs_28.c) to fill OSContPadEx[] directly from this helper.

#include <cstdint>

#include "mb64_input.h"

extern "C" int mb64_get_pad_ex(int port, uint16_t* button, int* sx, int* sy) {
    // Only port 0 is connected (keyboard + first game controller).
    if (port != 0) {
        return 0;
    }
    uint16_t b = 0;
    float x = 0.0f, y = 0.0f;
    mb64::input::get(port, &b, &x, &y);
    *button = b;
    // N64 analog stick hardware range is roughly -80..80; the game scales from there.
    *sx = (int)(x * 80.0f);
    *sy = (int)(y * 80.0f);
    return 1;
}
