RECOMP_FUNC void osContGetReadDataEx(uint8_t* rdram, recomp_context* ctx) {
    // MB64-FIX: fill OSContPadEx[] from the host input snapshot instead of the
    // (un-emulated) PIF buffer. ultramodern provides no osContGetReadDataEx, so
    // the game's PIF-based version runs but the PIF is never filled. data ptr is
    // in r4; OSContPadEx stride is 10 (gControllerPads[4] spans 0x28); loop the
    // 4 controllers (__osMaxControllers). errno 0 = present, 8 = no controller.
    // See mb64_si.cpp + mb64_input.cpp. The original body below is dead code.
    {
        extern int mb64_get_pad_ex(int port, uint16_t* button, int* sx, int* sy);
        gpr pad = ctx->r4;
        for (int i = 0; i < 4; i++) {
            uint16_t button = 0; int sx = 0, sy = 0;
            if (mb64_get_pad_ex(i, &button, &sx, &sy)) {
                MEM_HU(0X0, pad) = button;
                MEM_B(0X2, pad) = (int8_t)sx;
                MEM_B(0X3, pad) = (int8_t)sy;
                MEM_B(0X4, pad) = 0;   // c_stick_x (N64: none)
                MEM_B(0X5, pad) = 0;   // c_stick_y
                MEM_B(0X6, pad) = 0;   // l_trig
                MEM_B(0X7, pad) = 0;   // r_trig
                MEM_B(0X8, pad) = 0;   // errno: present
            } else {
                MEM_B(0X8, pad) = 8;   // CONT_NO_RESPONSE_ERROR
            }
            pad = ADD32(pad, 0XA);
        }
        return;
    }
}
