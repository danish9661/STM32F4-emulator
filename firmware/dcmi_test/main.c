// DCMI camera-interface test (polling, no interrupts).
//
// The pixel source is external hardware (a JS camera in the harness, see
// site/test_dcmi.mjs). This firmware drives the controller the way a real
// polling driver would and checks the two behaviours that matter:
//
//   Phase 1 — a frame that FITS the FIFO (2x2 = 4 pixels, FIFO depth 4) is
//     delivered intact. Capture, wait for real data to appear in DR (NOT for
//     the FRAME flag — see wait_pixel), drain four times, compare against
//     the known pattern.
//
//   Phase 2 — a frame LARGER than the FIFO overruns a polling drain and
//     must raise OVR (RIS bit 3). That is what real silicon does too, and
//     it is exactly why capture drivers use DMA. The harness swaps the
//     camera to an 8x4 frame when it sees the PHASE2 marker, so this phase
//     retries until it observes the overflow.
//
// CAPTURE reloads the pending frame on its RISING edge and auto-clears when
// a frame completes, so every capture here is an explicit 0 -> 1.
#define RCC_AHB2ENR (*(volatile unsigned int *)0x40023834)

#define DCMI_BASE 0x50050000
#define DCMI_CR   (*(volatile unsigned int *)(DCMI_BASE + 0x00))
#define DCMI_SR   (*(volatile unsigned int *)(DCMI_BASE + 0x04))
#define DCMI_RIS  (*(volatile unsigned int *)(DCMI_BASE + 0x08))
#define DCMI_IER  (*(volatile unsigned int *)(DCMI_BASE + 0x0C))
#define DCMI_ICR  (*(volatile unsigned int *)(DCMI_BASE + 0x10))
#define DCMI_DR   (*(volatile unsigned int *)(DCMI_BASE + 0x28))

#define RIS_LINE  (1u << 1)
#define RIS_FRAME (1u << 2)
#define RIS_OVR   (1u << 3)

// DMA2 stream 1 — the stream DCMI is wired to on real silicon (channel 1).
// Stream N registers are at DMA2_BASE + 0x10 + 0x18*N.
#define DMA2_BASE   0x40026400
#define DMA2_S1CR   (*(volatile unsigned int *)(DMA2_BASE + 0x28))
#define DMA2_S1NDTR (*(volatile unsigned int *)(DMA2_BASE + 0x2C))
#define DMA2_S1PAR  (*(volatile unsigned int *)(DMA2_BASE + 0x30))
#define DMA2_S1M0AR (*(volatile unsigned int *)(DMA2_BASE + 0x34))
#define DMA2_S1FCR  (*(volatile unsigned int *)(DMA2_BASE + 0x3C))
#define RCC_AHB1ENR (*(volatile unsigned int *)0x40023830)

#define BIG_W 8
#define BIG_H 4
#define BIG_N (BIG_W * BIG_H)
static volatile unsigned char dma_buf[BIG_N];

static void uart_init(void);
static void uart_puts(const char *s);
static void uart_hex8(unsigned int v);

// Arm a fresh capture: clear the flags, drop CAPTURE, then raise it so the
// model sees a rising edge and reloads whatever the camera has fed.
static void capture_start(void) {
    DCMI_ICR = 0x1F;
    DCMI_CR = 0;
    DCMI_CR = 1;
}

// Spin until a pixel actually shows up in the FIFO, and return it.
// Returns 0 on timeout.
//
// Deliberately NOT waiting on the FRAME flag: the controller raises FRS at
// capture START as well as at frame end, so a FRAME-flag wait returns before
// a single pixel has moved and whether the drain then sees data comes down
// to timing. The test pattern contains no zero bytes, so "DR read back
// non-zero" is an unambiguous data-arrived signal.
static unsigned int wait_pixel(void) {
    for (unsigned int spins = 0; spins < 8000000u; spins++) {
        unsigned int v = DCMI_DR & 0xFF;
        if (v) return v;
    }
    return 0;
}

// Spin looking for OVR. Long enough to span several host ticks, which is
// what actually moves pixels.
static int wait_ovr(void) {
    for (unsigned int spins = 0; spins < 4000000u; spins++) {
        if (DCMI_RIS & RIS_OVR) return 1;
    }
    return 0;
}

int main(void) {
    uart_init();
    uart_puts("=== DCMI Test ===\r\n");

    RCC_AHB2ENR |= (1u << 0);      // DCMI clock
    DCMI_IER = 0x1F;               // flags all enabled (polled, not vectored)

    // ── Phase 1: a FIFO-sized frame arrives intact ──
    capture_start();
    unsigned int px[4];
    px[0] = wait_pixel();
    if (!px[0]) {
        uart_puts("pixel wait timeout\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }
    // The whole 2x2 frame lands in the FIFO in one go, so the rest are
    // already queued behind the first.
    for (int i = 1; i < 4; i++) px[i] = DCMI_DR & 0xFF;

    uart_puts("px=");
    for (int i = 0; i < 4; i++) uart_hex8(px[i]);
    uart_puts(" ris=");
    uart_hex8(DCMI_RIS & 0xFF);
    uart_puts("\r\n");

    if (px[0] != 0x11 || px[1] != 0x22 || px[2] != 0x33 || px[3] != 0x44) {
        uart_puts("DCMI pixels FAIL\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }
    if (!(DCMI_RIS & RIS_LINE)) {
        uart_puts("DCMI line flag FAIL\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }
    uart_puts("DCMI pixels OK\r\n");

    // ── Phase 2: a frame bigger than the FIFO must flag OVR ──
    // The harness swaps the camera when it sees this marker; retry so the
    // firmware never depends on when exactly that lands.
    uart_puts("PHASE2\r\n");
    int seen_ovr = 0;
    for (int attempt = 0; attempt < 6 && !seen_ovr; attempt++) {
        capture_start();
        if (wait_ovr()) seen_ovr = 1;
    }
    if (!seen_ovr) {
        uart_puts("DCMI ovr FAIL\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }
    uart_puts("DCMI ovr OK\r\n");

    // ── Phase 3: the same oversized frame, captured by DMA, arrives whole ──
    // This is how a real capture driver runs, and it is the direct contrast
    // with phase 2: identical frame, identical FIFO, but the DMA answers
    // each request at bus rate so nothing is dropped.
    //
    // ORDER MATTERS: start the capture FIRST, then enable the stream. The
    // model queues the whole transfer when EN is written and the controller
    // loads the frame on CAPTURE's rising edge, so arming the DMA against an
    // idle DCMI just reads out zeroes.
    RCC_AHB1ENR |= (1u << 22);          // DMA2 clock
    for (int i = 0; i < BIG_N; i++) dma_buf[i] = 0;

    DCMI_ICR = 0x1F;
    DCMI_CR = 0;
    DCMI_CR = 1;                         // CAPTURE first

    DMA2_S1CR = 0;                       // disable while configuring
    DMA2_S1NDTR = BIG_N;
    DMA2_S1PAR = DCMI_BASE + 0x28;       // peripheral = DCMI_DR
    DMA2_S1M0AR = (unsigned int)&dma_buf[0];
    DMA2_S1FCR = 0x21;
    // CHSEL=1 (bits 27:25), MINC (bit 10), PSIZE/MSIZE = byte, DIR = 00
    // (peripheral -> memory), EN (bit 0).
    DMA2_S1CR = (1u << 25) | (1u << 10) | 1u;

    // Wait for the transfer to land. The last byte is non-zero in the test
    // pattern, so it doubles as the completion signal.
    int filled = 0;
    for (unsigned int spins = 0; spins < 8000000u && !filled; spins++) {
        if (dma_buf[BIG_N - 1] != 0) filled = 1;
    }
    if (!filled) {
        uart_puts("DCMI dma timeout\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }

    uart_puts("dma=");
    for (int i = 0; i < 4; i++) uart_hex8(dma_buf[i]);
    uart_puts("..");
    uart_hex8(dma_buf[BIG_N - 1]);
    uart_puts("\r\n");

    // Every pixel, in order, with no gap where the FIFO would have dropped.
    int bad = -1;
    for (int i = 0; i < BIG_N; i++) {
        if (dma_buf[i] != (unsigned char)(i + 1)) { bad = i; break; }
    }
    if (bad >= 0) {
        uart_puts("DCMI dma FAIL at ");
        uart_hex8((unsigned int)bad);
        uart_puts("\r\n=== DCMI Test: FAIL ===\r\n");
        while (1);
    }
    uart_puts("DCMI dma OK\r\n");

    uart_puts("=== DCMI Test: done ===\r\n");
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

static void uart_hex8(unsigned int v) {
    for (int i = 1; i >= 0; i--) {
        unsigned int nib = (v >> (i * 4)) & 0xF;
        uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}
