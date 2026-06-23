// Host-file-backed virtual SD card for Mario Builder 64's level saves.
//
// MB64 stores levels as .mb64 files on the flashcart's SD card via FatFS +
// libcart (src/libcart/ff). On real hardware the FatFS disk layer (diskio.c)
// calls cart_card_init / cart_card_rd_dram / cart_card_wr_dram, which do
// flashcart MMIO. Those are stubbed to no-ops in the recomp (can't do raw MMIO
// on PC), so `f_mount` failed (gMountSuccess = FR_DISK_ERR) and the editor
// crashed the moment it touched the filesystem ("Create").
//
// Here we back the "SD card" with a real host file (mb64_sd.img in the working
// directory), formatted FAT16 on first run. The recompiled cart_card_* stubs are
// overridden (in RecompiledFuncs) to call these helpers. Levels now persist
// across runs in that image (mountable on the host too — it's a plain FAT16 fs).
//
// Endianness: the recomp stores RDRAM byte-swizzled (a byte at physical offset p
// lives at rdram[p ^ 3], per the MEM_B macro). A real cart DMA lands raw bytes in
// ascending N64 address order, so we copy with the same ^3 swizzle.

#include <cstdint>
#include <cstdio>
#include <cstring>
#include <string>
#include <vector>

#include <unistd.h>  // ftruncate

namespace {
    constexpr uint32_t SECTOR_SIZE = 512;
    constexpr uint32_t IMG_SECTORS = 65536;          // 32 MiB virtual card
    constexpr uint32_t SEC_PER_CLUS = 4;
    constexpr uint32_t RSVD_SECTORS = 1;
    constexpr uint32_t NUM_FATS = 2;
    constexpr uint32_t ROOT_ENT_CNT = 512;
    constexpr uint32_t ROOT_SECTORS = (ROOT_ENT_CNT * 32) / SECTOR_SIZE;  // 32
    constexpr uint32_t FAT_SECTORS = 64;             // holds ~16k clusters (FAT16)

    FILE* g_img = nullptr;

    const char* img_path() { return "mb64_sd.img"; }

    void put16(uint8_t* p, uint16_t v) { p[0] = v & 0xFF; p[1] = (v >> 8) & 0xFF; }
    void put32(uint8_t* p, uint32_t v) { p[0]=v&0xFF; p[1]=(v>>8)&0xFF; p[2]=(v>>16)&0xFF; p[3]=(v>>24)&0xFF; }

    void format_fat16(FILE* f) {
        // Size the file (sparse zero-fill).
        if (ftruncate(fileno(f), (off_t)IMG_SECTORS * SECTOR_SIZE) != 0) {
            fprintf(stderr, "[sdcard] ftruncate failed\n");
        }

        // Boot sector (BPB).
        uint8_t bs[SECTOR_SIZE];
        memset(bs, 0, sizeof(bs));
        bs[0] = 0xEB; bs[1] = 0x3C; bs[2] = 0x90;            // jmp
        memcpy(bs + 3, "MSDOS5.0", 8);                       // OEM
        put16(bs + 11, SECTOR_SIZE);                          // BytsPerSec
        bs[13] = SEC_PER_CLUS;                                // SecPerClus
        put16(bs + 14, RSVD_SECTORS);                         // RsvdSecCnt
        bs[16] = NUM_FATS;                                    // NumFATs
        put16(bs + 17, ROOT_ENT_CNT);                         // RootEntCnt
        put16(bs + 19, 0);                                    // TotSec16 (0 -> use TotSec32)
        bs[21] = 0xF8;                                        // Media
        put16(bs + 22, FAT_SECTORS);                          // FATSz16
        put16(bs + 24, 32);                                   // SecPerTrk
        put16(bs + 26, 8);                                    // NumHeads
        put32(bs + 28, 0);                                    // HiddSec
        put32(bs + 32, IMG_SECTORS);                          // TotSec32
        bs[36] = 0x80;                                        // DrvNum
        bs[38] = 0x29;                                        // BootSig
        put32(bs + 39, 0x4D423634);                           // VolID
        memcpy(bs + 43, "MARIOBLD64 ", 11);                  // VolLab
        memcpy(bs + 54, "FAT16   ", 8);                       // FilSysType
        bs[510] = 0x55; bs[511] = 0xAA;                       // boot signature
        fseek(f, 0, SEEK_SET);
        fwrite(bs, 1, SECTOR_SIZE, f);

        // FAT: first two reserved entries (F8 FF / FF FF = media + EOC), rest free.
        uint8_t fat0[SECTOR_SIZE];
        memset(fat0, 0, sizeof(fat0));
        fat0[0] = 0xF8; fat0[1] = 0xFF; fat0[2] = 0xFF; fat0[3] = 0xFF;
        for (uint32_t i = 0; i < NUM_FATS; i++) {
            long off = (long)(RSVD_SECTORS + i * FAT_SECTORS) * SECTOR_SIZE;
            fseek(f, off, SEEK_SET);
            fwrite(fat0, 1, SECTOR_SIZE, f);
        }
        // Root directory + data region remain zero (empty).
        fflush(f);
    }

    bool ensure_open() {
        if (g_img) return true;
        FILE* f = fopen(img_path(), "r+b");
        if (!f) {
            f = fopen(img_path(), "w+b");
            if (!f) {
                fprintf(stderr, "[sdcard] cannot create %s\n", img_path());
                return false;
            }
            fprintf(stderr, "[sdcard] formatting new virtual SD card: %s\n", img_path());
            format_fat16(f);
        }
        g_img = f;
        return true;
    }
}

extern "C" {

// Returns 0 on success (matching libcart's cart_card_init convention).
int mb64_sd_init() {
    return ensure_open() ? 0 : 1;
}

// Read `count` 512-byte sectors at `lba` into RDRAM at the (KSEG0) `dram_vaddr`.
int mb64_sd_read(uint8_t* rdram, uint32_t dram_vaddr, uint32_t lba, uint32_t count) {
    if (!ensure_open()) return 1;
    const uint32_t phys = dram_vaddr & 0x1FFFFFFF;
    const uint32_t n = count * SECTOR_SIZE;
    std::vector<uint8_t> buf(n);
    if (fseek(g_img, (long)lba * SECTOR_SIZE, SEEK_SET) != 0) return 1;
    size_t got = fread(buf.data(), 1, n, g_img);
    if (got < n) memset(buf.data() + got, 0, n - got);  // past EOF reads as zeros
    for (uint32_t i = 0; i < n; i++) rdram[(phys + i) ^ 3] = buf[i];
    return 0;
}

// Write `count` 512-byte sectors at `lba` from RDRAM at the (KSEG0) `dram_vaddr`.
int mb64_sd_write(uint8_t* rdram, uint32_t dram_vaddr, uint32_t lba, uint32_t count) {
    if (!ensure_open()) return 1;
    const uint32_t phys = dram_vaddr & 0x1FFFFFFF;
    const uint32_t n = count * SECTOR_SIZE;
    std::vector<uint8_t> buf(n);
    for (uint32_t i = 0; i < n; i++) buf[i] = rdram[(phys + i) ^ 3];
    if (fseek(g_img, (long)lba * SECTOR_SIZE, SEEK_SET) != 0) return 1;
    if (fwrite(buf.data(), 1, n, g_img) < n) return 1;
    fflush(g_img);
    return 0;
}

}  // extern "C"
