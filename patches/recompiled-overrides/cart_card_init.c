RECOMP_FUNC void cart_card_init(uint8_t* rdram, recomp_context* ctx) {
    // MB64-FIX: open/format the host-backed virtual SD card. Returns 0 = success.
    extern int mb64_sd_init(void);
    ctx->r2 = mb64_sd_init();
}
