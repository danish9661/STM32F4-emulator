// Emulator-visible ABI shared between the F407 platform glue and the JS driver
// (site/emulator.js / site/doom.js). Fixed addresses, see AGENTS.md §16.
#ifndef DOOM_PLATFORM_H
#define DOOM_PLATFORM_H

#include <stddef.h>
#include <stdint.h>

// WAD image (doom1.wad, 4,196,020 bytes) preloaded by the JS driver via the
// extra_ram option.  Read-only from the guest's point of view.
#define DOOM_FB_ADDR   0xB8000000u
#define DOOM_FB_SIZE   4196020u

// Zone + newlib heap live in the emulated external SDRAM at 0xC0000000.
// The engine's .data/.bss occupy the first ~0x50000 bytes there, so the
// zone starts at 0xC0100000 (see link.ld).
#define DOOM_ZONE_ADDR 0xC0100000u
#define DOOM_ZONE_SIZE (6u * 1024 * 1024)
#define DOOM_HEAP_ADDR (DOOM_ZONE_ADDR + DOOM_ZONE_SIZE)
#define DOOM_HEAP_SIZE (9u * 1024 * 1024)

// Guest-visible SRAM ABI region (fixed addresses; see link.ld .abi @
// 0x20002000, low SRAM so the top-of-RAM stack has ~112K of headroom):
//   0x20002000  struct doom_abi_t (see platform.c):
//     0x00  u32 key write index  (written by JS)
//     0x04  u32 key read index   (written by guest)
//     0x08  key ring, 256 bytes, 2 bytes per event (keycode, 0x80|pressed)
//     0x110 palette, 1024 bytes, 256 * (b,g,r,a) u8  (written by guest)
//     0x510 u32 DG_ScreenBuffer value (written by guest at DG_Init)
//     0x514 u32 frame counter (written by guest per DG_DrawFrame; the JS
//            driver paces the game to realtime 35 fps against this)
//     0x518 u32 guest ms clock (written by guest per DG_SleepMs)
#define DOOM_ABI_ADDR     0x20002000u
#define KEYQ_SIZE         256u
#define KEYQ_INDEX_ADDR   (DOOM_ABI_ADDR + 0x00u)
#define KEYQ_RD_ADDR      (DOOM_ABI_ADDR + 0x04u)
#define KEYQ_RING_ADDR    (DOOM_ABI_ADDR + 0x08u)
#define PALETTE_ADDR      (DOOM_ABI_ADDR + 0x110u)
#define PALETTE_SIZE      1024u
#define DGSB_ADDR         (DOOM_ABI_ADDR + 0x510u)
#define FRAMECOUNT_ADDR   (DOOM_ABI_ADDR + 0x514u)
#define CLOCKMS_ADDR      (DOOM_ABI_ADDR + 0x518u)
// Savegame virtual files (no filesystem): 2 slots of 256 KB in the free
// EXTRAM gap between .bss (ends ~0xC004C000) and the zone (0xC0100000).
// The guest writes "doomsavN.dsg" blobs (engine temp-write + rename-commit);
// the JS driver mirrors them to localStorage via the flags below.
//     0x51C u32 saveFlag  (guest→driver) 1 = save written, 2 = load request
//     0x520 u32 saveSize  (guest→driver on save; driver→guest on load)
//     0x524 u32 saveReady (driver→guest) 1 = requested slot restored
//     0x528 u32 saveSlot  (guest→driver) slot index for flag operations
//     0x52C u32 saveMap   (driver→guest) bit N = slot N has a saved game
#define DOOM_SAVE_ADDR       0xC0080000u
#define DOOM_SAVE_SLOT_SIZE  0x40000u    // 256 KB (vanilla SAVEGAMESIZE cap)
#define DOOM_SAVE_SLOTS      2
#define SAVEFLAG_ADDR  (DOOM_ABI_ADDR + 0x51Cu)
#define SAVESIZE_ADDR  (DOOM_ABI_ADDR + 0x520u)
#define SAVEREADY_ADDR (DOOM_ABI_ADDR + 0x524u)
#define SAVESLOT_ADDR  (DOOM_ABI_ADDR + 0x528u)
#define SAVEMAP_ADDR   (DOOM_ABI_ADDR + 0x52Cu)
// Low-detail render request (driver→guest): 0 = high, 1 = low (every other
// column, ~12% fewer guest instructions per frame).  The guest applies it
// through the engine's own R_SetViewSize() — poking the `detailLevel`
// global from JS does NOT work: detailshift is only recomputed inside
// R_ExecuteSetViewSize(), which runs only when R_SetViewSize() has set
// setsizeneeded (measured: writing detailLevel alone leaves inst/frame
// bit-identical).
#define DETAIL_ADDR    (DOOM_ABI_ADDR + 0x530u)

// wad lookup + file-exists shim (platform.c)
const char *doom_wad_name(const char *path);
int doom_file_exists(const char *filename);

// frame audio mixer (i_sound_f407.c): mixes one frame (11025/35 samples) of
// active sfx channels into the I2S1 TX FIFO; called from DG_DrawFrame.
void DOOM_SubmitAudio(void);

#endif
