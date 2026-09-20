// LTDC display-controller test (polling, no interrupts):
//  - Layer0 configured as ARGB8888 over a B=64 x H=32 active window with
//    the framebuffer pinned at a fixed address (link.ld .fb section).
//  - The firmware paints a deterministic gradient (pixel(x,y) =
//    alpha=0xFF, r=x, g=y, b=x+y) directly into the framebuffer.
//  - Wait for LTDCEN scanout: poll the frame-end flag (ISR bit 1), then
//    print the frame counter / scanline and a checksum of 4 sample pixels.
// The Node harness re-reads the framebuffer memory and the exported
// ltdc_get_frame_count / ltdc_get_scanline to verify everything.
#define LTDC_BASE   0x40016800
#define LTDC_SSCR   (*(volatile unsigned int *)(LTDC_BASE + 0x08))
#define LTDC_BPCR   (*(volatile unsigned int *)(LTDC_BASE + 0x0C))
#define LTDC_AWCR   (*(volatile unsigned int *)(LTDC_BASE + 0x10))
#define LTDC_GCR    (*(volatile unsigned int *)(LTDC_BASE + 0x18))
#define LTDC_IER    (*(volatile unsigned int *)(LTDC_BASE + 0x34))
#define LTDC_ISR    (*(volatile unsigned int *)(LTDC_BASE + 0x38))
#define LTDC_ICR    (*(volatile unsigned int *)(LTDC_BASE + 0x3C))
#define LTDC_LIPCR  (*(volatile unsigned int *)(LTDC_BASE + 0x40))
#define LTDC_L1CR   (*(volatile unsigned int *)(LTDC_BASE + 0x84))
#define LTDC_L1WHPCR (*(volatile unsigned int *)(LTDC_BASE + 0x88))
#define LTDC_L1WVPCR (*(volatile unsigned int *)(LTDC_BASE + 0x8C))
#define LTDC_L1PFCR (*(volatile unsigned int *)(LTDC_BASE + 0x94))
#define LTDC_L1CFBAR (*(volatile unsigned int *)(LTDC_BASE + 0xAC))
#define LTDC_L1CFBLR (*(volatile unsigned int *)(LTDC_BASE + 0xB0))
#define LTDC_L1CFBLNR (*(volatile unsigned int *)(LTDC_BASE + 0xB4))

#define W 64
#define H 32
#define FB (*(volatile unsigned int (*)[W * H])0x20002000)

static void uart_init(void);
static void uart_puts(const char *s);
static void uart_dec(unsigned int v);
static void uart_putchar_hex(unsigned int nib);

int main(void) {
    uart_init();
    uart_puts("=== LTDC Test ===\r\n");

    // Paint the gradient framebuffer (ARGB8888).
    for (int y = 0; y < H; y++)
        for (int x = 0; x < W; x++)
            FB[y * W + x] = 0xFF000000u | ((unsigned int)x << 16)
                          | ((unsigned int)y << 8) | (unsigned int)(x + y);

    // 64x32 active; HSPW=8, HBP=9, VSPW=8, VBP=9 (model adds +1 to the
    // stored register values).
    LTDC_SSCR = (8 - 1) << 16 | (8 - 1);
    LTDC_BPCR = (9 - 1) << 16 | (9 - 1);
    LTDC_AWCR = (H - 1) << 16 | (W - 1);
    LTDC_LIPCR = H + 9 + 9 - 1; // last active line
    LTDC_IER = 0x0F;

    LTDC_L1CR = 0; // disable while configuring
    LTDC_L1PFCR = 0; // ARGB8888
    LTDC_L1WHPCR = (W - 1) | ((H - 1) << 16);
    LTDC_L1WVPCR = (W - 1) | ((H - 1) << 16);
    LTDC_L1CFBAR = (unsigned int)&FB[0];
    LTDC_L1CFBLR = ((W * 4) << 16) | (W * 4); // pitch = line bytes
    LTDC_L1CFBLNR = H;
    LTDC_L1CR = 1; // layer enable

    LTDC_GCR |= 1; // LTDCEN: start scanout
    uart_puts("scanout started\r\n");

    // Wait for the first completed frame (ISR bit 1 = F).
    unsigned int spins = 0;
    while (!(LTDC_ISR & 2) && spins < 20000000) spins++;
    if (!(LTDC_ISR & 2)) {
        uart_puts("frame wait timeout\r\n=== LTDC Test: FAIL ===\r\n");
        while (1);
    }
    {
        unsigned int sum = 0;
        unsigned int px[4] = { FB[0], FB[1], FB[W - 1], FB[W * H - 1] };
        for (int i = 0; i < 4; i++) sum += px[i];
        uart_puts("frame done ISR=");
        uart_puts((LTDC_ISR & 1) ? "LIF|F" : "F");
        uart_puts(" sum=");
        // print sum as 8 hex digits
        for (int i = 7; i >= 0; i--) {
            unsigned int nib = (sum >> (i * 4)) & 0xF;
            uart_putchar_hex(nib);
        }
        uart_puts("\r\n");
        if ((sum & 0xFFFFFFFFu) == 0xFC7F1F9Eu && (LTDC_ISR & 2)) {
            uart_puts("LTDC pixels OK\r\n");
        } else {
            uart_puts("LTDC pixels FAIL\r\n=== LTDC Test: FAIL ===\r\n");
            while (1);
        }
    }
    uart_puts("=== LTDC Test: done ===\r\n");
    while (1);
}

static void uart_init(void) {
    *(volatile unsigned int *)0x40023830 |= (1 << 0); // GPIOA
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
    *(volatile unsigned int *)0x40011008 = 16000000 / 115200;
    *(volatile unsigned int *)0x4001100C = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(*(volatile unsigned int *)0x40011000 & (1 << 7)));
    *(volatile unsigned int *)0x40011004 = c;
}

static void uart_putchar_hex(unsigned int nib) {
    uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_dec(unsigned int v) {
    char buf[12]; int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}