// Bare-metal platform glue for the doomgeneric port (F407).
// DG_* callbacks, key ring (JS-poked SRAM), palette export, newlib syscalls.
#include "doomgeneric.h"
#include "doomkeys.h"
#include "doomplatform.h"
#include "i_video.h"

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <fcntl.h>
#include <string.h>
#include <strings.h>
#include <unistd.h>

// ── SRAM key ring + palette (produced by JS / consumed by guest) ───────────
// Fixed layout at KEYQ_INDEX_ADDR (0x2000FE00), see doomplatform.h:
//   0x00 u32 key write index   (written by JS)
//   0x04 u32 key read index    (written by guest)
//   0x08 key ring, 256 bytes, 2 bytes per event (keycode, 0x80 if pressed)
//   0x110 palette, 1024 bytes, 256 * (b,g,r,a) u8 (written by guest)
// A single struct guarantees member offsets regardless of linker section
// sorting; .abi starts exactly at KEYQ_INDEX_ADDR.
typedef struct {
    volatile uint32_t keyWr;
    volatile uint32_t keyRd;
    volatile uint8_t  keyRing[KEYQ_SIZE];
    volatile uint8_t  pad[8];
    volatile uint8_t  palette[PALETTE_SIZE];
    volatile uint32_t screenBuffer;   // DG_ScreenBuffer value (DG_Init)
    volatile uint32_t frameCount;     // DG_DrawFrame calls (1 per rendered frame)
    volatile uint32_t clockMs;        // guest ms clock (advanced by DG_SleepMs)
    volatile uint32_t saveFlag;       // save ABI, see doomplatform.h
    volatile uint32_t saveSize;
    volatile uint32_t saveReady;
    volatile uint32_t saveSlot;
    volatile uint32_t saveMap;
    volatile uint32_t detail;         // driver->guest: 0 = high, 1 = low detail
} doom_abi_t;

__attribute__((section(".abi"), aligned(16)))
volatile doom_abi_t g_abi;

// ── UART1 TX (polling) for printf output ────────────────────────────────────
#define USART1_SR (*(volatile uint32_t*)0x40011000)
#define USART1_DR (*(volatile uint32_t*)0x40011004)
#define SR_TXE     (1u << 7)

void uart_putchar(char c)
{
    if (c == '\n') {
        while (!(USART1_SR & SR_TXE)) ;
        USART1_DR = '\r';
    }
    while (!(USART1_SR & SR_TXE)) ;
    USART1_DR = c;
}

// ── WAD / file shims (no filesystem on bare metal) ──────────────────────────
// The WAD image is preloaded at DOOM_FB_ADDR by the JS driver (extra_ram).
// w_file_mem.c provides the stdc_wad_file replacement used by w_file.c;
// M_FileExists is patched in m_misc.c to call doom_file_exists first.

const char *doom_wad_name(const char *path)
{
    const char *p = path;
    const char *slash = NULL;
    while (*p) {
        if (*p == '/' || *p == '\\') slash = p;
        p++;
    }
    const char *base = slash ? slash + 1 : path;
    if (strcasecmp(base, "doom1.wad") == 0) return base;
    if (strcasecmp(base, "doom.wad") == 0) return base;
    return NULL;
}

static int is_savegame_name(const char *path);
static int save_slot_of(const char *path);

int doom_file_exists(const char *filename)
{
    if (doom_wad_name(filename) != NULL) return 1;
    if (is_savegame_name(filename)) {
        int s = save_slot_of(filename);
        return (s >= 0 && s < DOOM_SAVE_SLOTS && (g_abi.saveMap >> s) & 1u);
    }
    return 0;
}

// I_ZoneBase: the zone lives in the emulated external SDRAM
void *doom_zone_base(int *size)
{
    *size = (int)DOOM_ZONE_SIZE;
    return (void*)DOOM_ZONE_ADDR;
}

// ── Savegame virtual files (no filesystem) ───────────────────────────────────
// The engine writes a save via a temp file ("doom1/temp.dsg") and renames it
// onto "doom1/doomsavN.dsg" (newlib rename = _link + _unlink).  Blobs stage
// in slot 0's EXTRAM area; _link parses the target slot, copies the staging
// buffer there, and flags the JS driver (saveFlag=1 + saveSlot + saveSize)
// which mirrors it to localStorage.  Loads run the reverse handshake:
// fopen("rb") sets saveFlag=2 + saveSlot and busy-waits on saveReady — the
// driver restores the blob within one step burst.
#define DOOM_SAVE_FD 0x7f00
static uint32_t save_pos, save_len;
static int save_wr;

static int is_savegame_name(const char *path)
{
    return strstr(path, ".dsg") != NULL;
}

static int save_slot_of(const char *path)
{
    const char *p = strstr(path, "doomsav");
    if (p != NULL && p[7] >= '0' && p[7] <= '9') return p[7] - '0';
    return 0;   // temp.dsg / recovery.dsg stage and commit via rename
}

static void save_commit(const char *target)
{
    int slot = save_slot_of(target);
    if (slot < 0 || slot >= DOOM_SAVE_SLOTS || save_len == 0) return;
    if (slot != 0)
        memcpy((void*)(DOOM_SAVE_ADDR + slot * DOOM_SAVE_SLOT_SIZE),
               (const void*)DOOM_SAVE_ADDR, save_len);
    g_abi.saveSlot = (uint32_t)slot;
    g_abi.saveSize = save_len;
    g_abi.saveFlag = 1;
    printf("SAVE ok slot=%d bytes=%d\n", slot, (int)save_len);
}

// ── newlib syscalls ─────────────────────────────────────────────────────────

int _write(int fd, const char *buf, int len)
{
    if (fd == DOOM_SAVE_FD) {   // savegame stream -> EXTRAM staging area
        uint32_t n = len;
        if (save_pos + n > DOOM_SAVE_SLOT_SIZE) n = DOOM_SAVE_SLOT_SIZE - save_pos;
        memcpy((void*)(DOOM_SAVE_ADDR + save_pos), buf, n);
        save_pos += n;
        if (save_pos > save_len) save_len = save_pos;
        return n;
    }
    int i;
    for (i = 0; i < len; i++) uart_putchar(buf[i]);
    return len;
}

// Bump allocator over the EXTRAM region AFTER the doom zone. The linker's
// __heap_start lands at the end of .bss (0xC0100000), which is exactly
// DOOM_ZONE_ADDR — malloc must start above the zone or the zone and the
// heap (framebuffer!) clobber each other (W_CheckNumForName hash-chain
// corruption, framebuffer-over-zone-header garbage).
extern char __heap_start[];
void *_sbrk(ptrdiff_t incr)
{
    static char *cur = NULL;
    char *p;
    if (cur == NULL) cur = (char *)DOOM_HEAP_ADDR;
    p = cur;
    cur += incr;
    return p;
}

int _read(int fd, char *buf, int len)
{
    if (fd != DOOM_SAVE_FD) return 0;
    uint32_t n = len;
    if (save_pos + n > save_len) n = save_len - save_pos;
    memcpy(buf, (const void*)(DOOM_SAVE_ADDR + save_pos), n);
    save_pos += n;
    return n;
}
int _close(int fd) { return 0; }
int _lseek(int fd, int off, int whence)
{
    if (fd != DOOM_SAVE_FD) return -1;
    long pos = whence == SEEK_SET ? off
             : whence == SEEK_CUR ? (long)save_pos + off
             : (long)save_len + off;
    if (pos < 0) return -1;
    save_pos = (uint32_t)pos;
    return pos;
}
int _fstat(int fd, void *st) { return 0; }
int _isatty(int fd) { return 1; }
int _getpid(void) { return 1; }
int _kill(int pid, int sig) { return -1; }
int _open(const char *path, int flags, ...)
{
    if (!is_savegame_name(path)) return -1;
    int slot = save_slot_of(path);
    if (slot < 0 || slot >= DOOM_SAVE_SLOTS) return -1;
    if ((flags & O_ACCMODE) == O_RDONLY) {
        // No save in this slot: fail instantly, NO handshake — the engine
        // probes existence via open() (M_FileExists fallback) and must never
        // busy-wait for a restore that can't happen.
        if (!((g_abi.saveMap >> slot) & 1u)) return -1;
        g_abi.saveSlot = (uint32_t)slot;
        g_abi.saveFlag = 2;
        while (!g_abi.saveReady) ;   // driver restores between step bursts
        g_abi.saveReady = 0;
        if (g_abi.saveSize == 0) return -1;
        save_pos = 0; save_len = g_abi.saveSize; save_wr = 0;
        printf("LOAD ok slot=%d bytes=%d\n", slot, (int)save_len);
    } else {
        save_pos = 0; save_len = 0; save_wr = 1;
    }
    return DOOM_SAVE_FD;
}
int _unlink(const char *path) { return is_savegame_name(path) ? 0 : -1; }
int _link(const char *oldp, const char *newp)
{
    if (is_savegame_name(oldp) && is_savegame_name(newp)) {
        save_commit(newp);
        return 0;
    }
    return -1;
}
int mkdir(const char *path, mode_t mode) { return -1; }
void _exit(int code) { for (;;) ; }

// ── DG_* platform callbacks ─────────────────────────────────────────────────

void DG_Init()
{
    // Expose the engine's framebuffer pointer (SCREENWIDTH*SCREENHEIGHT
    // palette indices under CMAP256) to the JS driver.
    g_abi.screenBuffer = (uint32_t)(uintptr_t)DG_ScreenBuffer;
}

// Apply a driver-requested detail level through the engine's own API.
// detailshift (and the colfunc/spanfunc render pointers that actually make
// low detail cheaper) are recomputed ONLY inside R_ExecuteSetViewSize(), so
// poking the `detailLevel` global from JS does nothing — measured: it leaves
// inst/frame bit-identical.
//
// R_SetViewSize() only *requests* the change (setsizeneeded=1); D_Display()
// is supposed to consume it, but in this build that check never fires
// (probed: setsizeneeded stays 1 across frames while rendering continues),
// so we execute it here.  Safe: DG_DrawFrame runs at the end of a frame,
// right after I_FinishUpdate's blit — exactly the "takes effect next
// refresh" boundary R_SetViewSize's own comment describes.
extern void R_SetViewSize(int blocks, int detail);
extern void R_ExecuteSetViewSize(void);
extern int screenblocks;
extern int detailLevel;

static void apply_detail_request(void)
{
    int want = (int)(g_abi.detail ? 1 : 0);
    if (want == detailLevel)
        return;
    detailLevel = want;
    R_SetViewSize(screenblocks, detailLevel);
    R_ExecuteSetViewSize();
}

void DG_DrawFrame()
{
    // Export the current CMAP256 palette to the fixed ABI address so the JS
    // driver can map framebuffer indices -> RGB each frame, and count frames
    // (the driver paces the loop to realtime 35 tics/s using this counter).
    memcpy((void*)g_abi.palette, (const void*)colors, PALETTE_SIZE);
    g_abi.frameCount++;
    apply_detail_request();
    // One frame's worth of audio (11025/35 samples) -> I2S1 capture FIFO.
    DOOM_SubmitAudio();
}

void DG_SleepMs(uint32_t ms)
{
    // No busy-wait.  Advance the ABI clock RELATIVELY (monotonic, never
    // backwards) so I_GetTime-based waits (the melt wipe's "while (tics<=0)"
    // loop) resolve immediately even in harnesses with no driver clock
    // writer; the JS driver's absolute wall-clock writes per emu.step() stay
    // authoritative and this never flaps them (an absolute write here would
    // race the driver and send the wait loop's tics negative).
    g_abi.clockMs += ms;
}

uint32_t DG_GetTicksMs()
{
    // Read the ABI clockMs slot: the JS driver writes the page wall clock
    // before every emu.step(), so I_GetTime keeps advancing even while the
    // guest is inside a single doomgeneric_Tick (the level-start melt wipe
    // spins in "while (tics<=0)" until time advances — with a guest-internal
    // clock it wedges forever).
    return g_abi.clockMs;
}

int DG_GetKey(int *pressed, unsigned char *key)
{
    volatile uint32_t rd = g_abi.keyRd;
    volatile uint32_t wr = g_abi.keyWr;
    if (rd == wr) return 0;
    uint32_t off = (rd % KEYQ_SIZE);
    uint8_t k = g_abi.keyRing[off];
    uint8_t fl = g_abi.keyRing[(off + 1) % KEYQ_SIZE];
    g_abi.keyRd = (rd + 2) % KEYQ_SIZE;
    *key = k;
    *pressed = (fl & 0x80) ? 1 : 0;
    return 1;
}

void DG_SetWindowTitle(const char *title)
{
}
