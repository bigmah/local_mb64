RECOMP_FUNC void detect_emulator(uint8_t* rdram, recomp_context* ctx) {
    // MB64-FIX: the original detect_emulator() probes real N64 hardware — PI/DPC/PIF
    // registers (osPiReadIo(0x1ffb0000,…), IO_READ(DPC_*_REG)) and raw KSEG1 derefs
    // like *(volatile u16*)0xbfd00106 — none of which exist in the recomp runtime.
    // Recompiled, those become wild rdram reads and crash at boot (EXC_BAD_ACCESS in
    // detect_emulator). Report EMU_CONSOLE so the game takes the console path
    // (gBorderHeight = BORDER_HEIGHT_CONSOLE, skips the VC/RCVI hacks). Return value
    // is v0 = ctx->r2; EMU_CONSOLE = (1 << 0).
    ctx->r2 = 0x1; // EMU_CONSOLE
}
