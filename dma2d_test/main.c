// DMA2D Chrom-ART test (F429 only): register-to-memory fill, mem-to-mem
// copy, pixel-format conversion, alpha blend, and line-offset stride.
// Completion runs through the real path: START -> JS blit -> TCIF + IRQ56,
// whose ISR write-1-clears the flag and counts. All checks print OK;
// any timeout or mismatch prints FAIL and stops.
#define RCC_BASE      0x40023800
#define RCC_AHB1ENR   (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define DMA2D_BASE    0x4002B000
#define DMA2D_CR      (*(volatile unsigned int *)(DMA2D_BASE + 0x00))
#define DMA2D_ISR     (*(volatile unsigned int *)(DMA2D_BASE + 0x04))
#define DMA2D_IFCR    (*(volatile unsigned int *)(DMA2D_BASE + 0x08))
#define DMA2D_FGMAR   (*(volatile unsigned int *)(DMA2D_BASE + 0x0C))
#define DMA2D_FGOR    (*(volatile unsigned int *)(DMA2D_BASE + 0x10))
#define DMA2D_BGMAR   (*(volatile unsigned int *)(DMA2D_BASE + 0x14))
#define DMA2D_BGOR    (*(volatile unsigned int *)(DMA2D_BASE + 0x18))
#define DMA2D_FGPFCCR (*(volatile unsigned int *)(DMA2D_BASE + 0x1C))
#define DMA2D_BGPFCCR (*(volatile unsigned int *)(DMA2D_BASE + 0x24))
#define DMA2D_OPFCCR  (*(volatile unsigned int *)(DMA2D_BASE + 0x34))
#define DMA2D_OCOLR   (*(volatile unsigned int *)(DMA2D_BASE + 0x38))
#define DMA2D_OMAR    (*(volatile unsigned int *)(DMA2D_BASE + 0x3C))
#define DMA2D_OOR     (*(volatile unsigned int *)(DMA2D_BASE + 0x40))
#define DMA2D_NLR     (*(volatile unsigned int *)(DMA2D_BASE + 0x44))

#define NVIC_ISER1    (*(volatile unsigned int *)0xE000E104)

static volatile unsigned int dma2d_irqs = 0;

void DMA2D_IRQHandler(void) {
    if (DMA2D_ISR & 2) {
        DMA2D_IFCR = 2; // write-1-clear TCIF
        dma2d_irqs++;
    }
}

static void uart_init(void);
static void uart_puts(const char *s);
static void uart_hex32(unsigned int v);

static int dma2d_wait(const char *what) {
    volatile unsigned int n = dma2d_irqs;
    for (volatile unsigned int i = 0; i < 2000000; i++) {
        if (dma2d_irqs != n) return 1;
    }
    uart_puts("DMA2D TIMEOUT ");
    uart_puts(what);
    uart_puts("\r\n");
    return 0;
}

static int check32(const char *what, unsigned int got, unsigned int want) {
    if (got != want) {
        uart_puts("DMA2D FAIL ");
        uart_puts(what);
        uart_puts(" got=");
        uart_hex32(got);
        uart_puts(" want=");
        uart_hex32(want);
        uart_puts("\r\n");
        return 0;
    }
    return 1;
}

static unsigned int frame1[64 * 32];
static unsigned int frame2[64 * 32];
static const unsigned short rgb565[4] = { 0xF800, 0x07E0, 0x001F, 0xFFFF };
static unsigned int conv_out[4];
static const unsigned int fg_px[2] = { 0x80FF0000u, 0xFF00FF00u };
static const unsigned int bg_px[2] = { 0xFF0000FFu, 0xFFFFFFFFu };
static unsigned int blend_out[2];
static unsigned int pitch_buf[16 * 2];

int main(void) {
    uart_init();
    uart_puts("=== DMA2D Test ===\r\n");

    RCC_AHB1ENR |= (1u << 23); // DMA2DEN
    NVIC_ISER1 |= (1u << 24);  // IRQ56 = DMA2D

    // T1: register-to-memory solid red fill, 64x32 ARGB8888.
    DMA2D_OMAR = (unsigned int)frame1;
    DMA2D_OOR = 0;
    DMA2D_NLR = (64u << 16) | 32u;
    DMA2D_OPFCCR = 0;
    DMA2D_OCOLR = 0xFFFF0000u;
    DMA2D_CR = (3u << 16) | (1u << 9) | 1u; // R2M + TCIE + START
    if (!dma2d_wait("R2M")) while (1);
    if (!check32("R2M px0", frame1[0], 0xFFFF0000u)) while (1);
    if (!check32("R2M pxN", frame1[64 * 32 - 1], 0xFFFF0000u)) while (1);
    uart_puts("DMA2D R2M OK\r\n");

    // T2: mem-to-mem copy of the red frame.
    DMA2D_FGMAR = (unsigned int)frame1;
    DMA2D_FGOR = 0;
    DMA2D_OMAR = (unsigned int)frame2;
    DMA2D_OOR = 0;
    DMA2D_NLR = (64u << 16) | 32u;
    DMA2D_OPFCCR = 0;
    DMA2D_CR = (0u << 16) | (1u << 9) | 1u; // M2M + TCIE + START
    if (!dma2d_wait("M2M")) while (1);
    if (!check32("M2M px0", frame2[0], 0xFFFF0000u)) while (1);
    if (!check32("M2M pxN", frame2[64 * 32 - 1], 0xFFFF0000u)) while (1);
    uart_puts("DMA2D M2M OK\r\n");

    // T3: pixel-format conversion RGB565 -> ARGB8888.
    DMA2D_FGMAR = (unsigned int)rgb565;
    DMA2D_FGOR = 0;
    DMA2D_FGPFCCR = 2;
    DMA2D_OMAR = (unsigned int)conv_out;
    DMA2D_OOR = 0;
    DMA2D_NLR = (4u << 16) | 1u;
    DMA2D_OPFCCR = 0;
    DMA2D_CR = (1u << 16) | (1u << 9) | 1u; // M2M+PFC + TCIE + START
    if (!dma2d_wait("PFC")) while (1);
    if (!check32("PFC red", conv_out[0], 0xFFFF0000u)) while (1);
    if (!check32("PFC green", conv_out[1], 0xFF00FF00u)) while (1);
    if (!check32("PFC blue", conv_out[2], 0xFF0000FFu)) while (1);
    if (!check32("PFC white", conv_out[3], 0xFFFFFFFFu)) while (1);
    uart_puts("DMA2D PFC OK\r\n");

    // T4: blend semi-red over blue -> (255,128,0,127); opaque green wins.
    DMA2D_FGMAR = (unsigned int)fg_px;
    DMA2D_FGOR = 0;
    DMA2D_FGPFCCR = 0;
    DMA2D_BGMAR = (unsigned int)bg_px;
    DMA2D_BGOR = 0;
    DMA2D_BGPFCCR = 0;
    DMA2D_OMAR = (unsigned int)blend_out;
    DMA2D_OOR = 0;
    DMA2D_NLR = (2u << 16) | 1u;
    DMA2D_OPFCCR = 0;
    DMA2D_CR = (2u << 16) | (1u << 9) | 1u; // blend + TCIE + START
    if (!dma2d_wait("blend")) while (1);
    if (!check32("blend px0", blend_out[0], 0xFF80007Fu)) while (1);
    if (!check32("blend px1", blend_out[1], 0xFF00FF00u)) while (1);
    uart_puts("DMA2D blend OK\r\n");

    // T5: output line offset (pitch 16, width 8): gap pixels stay zero.
    for (int i = 0; i < 32; i++) pitch_buf[i] = 0;
    DMA2D_OMAR = (unsigned int)pitch_buf;
    DMA2D_OOR = 8;
    DMA2D_NLR = (8u << 16) | 2u;
    DMA2D_OPFCCR = 0;
    DMA2D_OCOLR = 0xFF00FF00u;
    DMA2D_CR = (3u << 16) | (1u << 9) | 1u;
    if (!dma2d_wait("stride")) while (1);
    if (!check32("stride r0c0", pitch_buf[0], 0xFF00FF00u)) while (1);
    if (!check32("stride r0c7", pitch_buf[7], 0xFF00FF00u)) while (1);
    if (!check32("stride gap", pitch_buf[8], 0u)) while (1);
    if (!check32("stride r1c0", pitch_buf[16], 0xFF00FF00u)) while (1);
    uart_puts("DMA2D stride OK\r\n");

    uart_puts("=== DMA2D Test: done ===\r\n");
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

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_hex32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        unsigned int nib = (v >> (i * 4)) & 0xF;
        uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}
