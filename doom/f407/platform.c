// Bare-metal platform glue for the doomgeneric port (F407).
// DG_* callbacks, key ring (JS-poked SRAM), palette export, newlib syscalls.
#include "doomgeneric.h"
#include "doomkeys.h"
#include "doomplatform.h"
#include "i_video.h"

#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
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

int doom_file_exists(const char *filename)
{
    return doom_wad_name(filename) != NULL;
}

// I_ZoneBase: the zone lives in the emulated external SDRAM
void *doom_zone_base(int *size)
{
    *size = (int)DOOM_ZONE_SIZE;
    return (void*)DOOM_ZONE_ADDR;
}

// ── newlib syscalls ─────────────────────────────────────────────────────────

int _write(int fd, const char *buf, int len)
{
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

int _read(int fd, char *buf, int len) { return 0; }
int _close(int fd) { return 0; }
int _lseek(int fd, int off, int whence) { return 0; }
int _fstat(int fd, void *st) { return 0; }
int _isatty(int fd) { return 1; }
int _getpid(void) { return 1; }
int _kill(int pid, int sig) { return -1; }
int _open(const char *path, int flags, ...) { return -1; }
int _unlink(const char *path) { return -1; }
int _link(const char *oldp, const char *newp) { return -1; }
int mkdir(const char *path, int mode) { return -1; }
void _exit(int code) { for (;;) ; }

// ── DG_* platform callbacks ─────────────────────────────────────────────────

// Monotonic ms clock, advanced only by DG_SleepMs so I_GetTime is stable
// within a single doomgeneric_Tick call.
static volatile uint32_t s_msClock = 0;

void DG_Init()
{
    // Expose the engine's framebuffer pointer (SCREENWIDTH*SCREENHEIGHT
    // palette indices under CMAP256) to the JS driver.
    g_abi.screenBuffer = (uint32_t)(uintptr_t)DG_ScreenBuffer;
}

void DG_DrawFrame()
{
    // Export the current CMAP256 palette to the fixed ABI address so the JS
    // driver can map framebuffer indices -> RGB each frame.
    memcpy((void*)g_abi.palette, (const void*)colors, PALETTE_SIZE);
}

void DG_SleepMs(uint32_t ms)
{
    s_msClock += ms;
    // Light busy-wait so the emulated execution spends some instructions
    // (the emulator is instruction-count driven; this paces the game loop).
    volatile uint32_t n = ms * 4000;
    while (n--) ;
}

uint32_t DG_GetTicksMs()
{
    return s_msClock;
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
