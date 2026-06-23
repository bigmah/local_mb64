// Stubs for libultra functions that the recompiled MB64 calls but that
// N64ModernRuntime doesn't implement (uncommon ones: TLB mapping, raw PI/SI IO,
// controller-pak/PFS). drmario never calls these so the runtime omits them; MB64
// does. No-op / return-0 is fine for reaching a first frame (these are
// save/controller-pak/hardware paths, not the render path). Revisit for real
// save + controller-pak support later.
#include "recomp.h"

#include "ultramodern/ultramodern.hpp"

// In the recomp ABI the return value goes in $v0 (== r2); 0 means success/empty.
#define RETURN0() do { ctx->r2 = 0; } while (0)

extern "C" {

void osMapTLB_recomp(uint8_t* rdram, recomp_context* ctx)            { (void)rdram; (void)ctx; }
void osUnmapTLB_recomp(uint8_t* rdram, recomp_context* ctx)          { (void)rdram; (void)ctx; }
void osPiReadIo_recomp(uint8_t* rdram, recomp_context* ctx)          { (void)rdram; RETURN0(); }
void osPiWriteIo_recomp(uint8_t* rdram, recomp_context* ctx)         { (void)rdram; RETURN0(); }
void __osSiGetAccess_recomp(uint8_t* rdram, recomp_context* ctx)     { (void)rdram; (void)ctx; }
// Every SI DMA completes with an SI interrupt that posts to the game's SI event
// queue. MB64's blocking controller read (osContStartReadDataEx) does one
// osRecvMesg per __osSiRawStartDma (WRITE then READ), so post exactly one SI
// message per call to unblock it — without this the game FREEZES on its first
// controller read (thread5_game_loop never advances). The controller response
// bytes in the PIF buffer are filled by mb64_si_fill_pifram() (see mb64_si.cpp),
// invoked from the recompiled osContGetReadDataEx. send_si_message is NOBLOCK.
void __osSiRawStartDma_recomp(uint8_t* rdram, recomp_context* ctx)   { ultramodern::send_si_message(rdram); RETURN0(); }
void __osSiRelAccess_recomp(uint8_t* rdram, recomp_context* ctx)     { (void)rdram; (void)ctx; }
void __osContAddressCrc_recomp(uint8_t* rdram, recomp_context* ctx)  { (void)rdram; RETURN0(); }
void __osContRamRead_recomp(uint8_t* rdram, recomp_context* ctx)     { (void)rdram; RETURN0(); }
void __osPfsSelectBank_recomp(uint8_t* rdram, recomp_context* ctx)   { (void)rdram; RETURN0(); }

// libultra's static output callback for sprintf(), at ROM vaddr 0x80132358. It
// lives inside the prebuilt libnustd sprintf object with NO symbol, so N64Recomp
// folded it into sprintf() and registered no function at its address. sprintf()
// passes &proutSprintf to _Printf(), which calls it INDIRECTLY through the
// function-pointer lookup table — so without a lookup entry at 0x80132358 the
// first sprintf() at runtime (e.g. the level editor formatting text via
// show_tip/toolbar) aborts with "Failed to find function at 0x80132358". We
// reimplement it here and register it at that address in
// register_resident_function_addresses() (mb64_overlays.cpp). Original:
//   char *proutSprintf(char *dst, const char *src, size_t count) {
//       return (char *)memcpy(dst, src, count) + count;   // == dst + count
//   }
void proutSprintf_recomp(uint8_t* rdram, recomp_context* ctx) {
    const gpr dst = ctx->r4, src = ctx->r5, count = ctx->r6;
    for (gpr i = 0; i < count; i++) {
        MEM_B(i, dst) = MEM_BU(i, src);
    }
    ctx->r2 = ADD32(dst, count); // $v0 = dst + count
}

void recomp_syscall_handler(uint8_t* rdram, recomp_context* ctx, int32_t instruction_vram) {
    // Minimal: ignore. The game's syscall use is mainly __n64Assert; a real impl
    // would decode and dispatch. No-op lets boot proceed.
    (void)rdram; (void)ctx; (void)instruction_vram;
}

} // extern "C"
