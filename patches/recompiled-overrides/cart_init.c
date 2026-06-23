RECOMP_FUNC void cart_init(uint8_t* rdram, recomp_context* ctx) {
    // MB64-FIX: ensure the host-backed virtual SD card is ready. cart_init()
    // returns the cart type (ignored by mb64_file_init); 0 is fine.
    extern int mb64_sd_init(void);
    mb64_sd_init();
    ctx->r2 = 0;
}
