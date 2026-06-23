RECOMP_FUNC void cart_card_rd_dram(uint8_t* rdram, recomp_context* ctx) {
    // MB64-FIX: host-backed virtual SD card read (see mb64_sdcard.cpp).
    // cart_card_rd_dram(void* dram, u32 lba, u32 count) → r4=dram, r5=lba, r6=count.
    extern int mb64_sd_read(uint8_t*, uint32_t, uint32_t, uint32_t);
    ctx->r2 = mb64_sd_read(rdram, (uint32_t)ctx->r4, (uint32_t)ctx->r5, (uint32_t)ctx->r6);
}
