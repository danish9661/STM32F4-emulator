// CAN bus arbitration test: two controllers (CAN1, CAN2) on one shared bus.
// Phase 1 (loopback): CAN1 transmits with BTR LBKM set — the frame is
// echoed into CAN1's own RX FIFO only.
// Phase 2 (arbitration): CAN1 and CAN2 both stage a TX in the same round —
// the lower arbitration ID (0x200 from CAN2) wins the bus; both nodes
// receive it (broadcast + winner self-echo), the loser (0x300) completes on
// the next free round, and both nodes then have both frames.
#define CAN1_BASE 0x40006400
#define CAN2_BASE 0x40006800

#define CAN_MCR   (*(volatile unsigned int *)(CAN1_BASE + 0x00))
#define CAN_MSR   (*(volatile unsigned int *)(CAN1_BASE + 0x04))
#define CAN_TSR   (*(volatile unsigned int *)(CAN1_BASE + 0x08))
#define CAN_RF0R  (*(volatile unsigned int *)(CAN1_BASE + 0x0C))
#define CAN_IER   (*(volatile unsigned int *)(CAN1_BASE + 0x14))
#define CAN_BTR   (*(volatile unsigned int *)(CAN1_BASE + 0x1C))

#define CAN_TIR0  (*(volatile unsigned int *)(CAN1_BASE + 0x180))
#define CAN_TDTR0 (*(volatile unsigned int *)(CAN1_BASE + 0x184))
#define CAN_TDLR0 (*(volatile unsigned int *)(CAN1_BASE + 0x188))
#define CAN_TDHR0 (*(volatile unsigned int *)(CAN1_BASE + 0x18C))

#define CAN_RIR0  (*(volatile unsigned int *)(CAN1_BASE + 0x1B0))
#define CAN_RDLR0 (*(volatile unsigned int *)(CAN1_BASE + 0x1B8))
#define CAN_RDHR0 (*(volatile unsigned int *)(CAN1_BASE + 0x1BC))
#define CAN_RIR1  (*(volatile unsigned int *)(CAN1_BASE + 0x1C0))
#define CAN_RDLR1 (*(volatile unsigned int *)(CAN1_BASE + 0x1C8))
#define CAN_RDHR1 (*(volatile unsigned int *)(CAN1_BASE + 0x1CC))
#define CAN_RIR2  (*(volatile unsigned int *)(CAN1_BASE + 0x1D0))
#define CAN_RDLR2 (*(volatile unsigned int *)(CAN1_BASE + 0x1D8))
#define CAN_RDHR2 (*(volatile unsigned int *)(CAN1_BASE + 0x1DC))

#define CAN_FMR   (*(volatile unsigned int *)(CAN1_BASE + 0x200))
#define CAN_FS1R  (*(volatile unsigned int *)(CAN1_BASE + 0x20C))
#define CAN_FA1R  (*(volatile unsigned int *)(CAN1_BASE + 0x21C))
#define CAN_FILT0 (*(volatile unsigned int *)(CAN1_BASE + 0x240))
#define CAN_FILT1 (*(volatile unsigned int *)(CAN1_BASE + 0x244))

#define CAN2_MCR   (*(volatile unsigned int *)(CAN2_BASE + 0x00))
#define CAN2_TSR   (*(volatile unsigned int *)(CAN2_BASE + 0x08))
#define CAN2_RF0R  (*(volatile unsigned int *)(CAN2_BASE + 0x0C))
#define CAN2_IER   (*(volatile unsigned int *)(CAN2_BASE + 0x14))
#define CAN2_BTR   (*(volatile unsigned int *)(CAN2_BASE + 0x1C))
#define CAN2_TIR0  (*(volatile unsigned int *)(CAN2_BASE + 0x180))
#define CAN2_TDTR0 (*(volatile unsigned int *)(CAN2_BASE + 0x184))
#define CAN2_TDLR0 (*(volatile unsigned int *)(CAN2_BASE + 0x188))
#define CAN2_TDHR0 (*(volatile unsigned int *)(CAN2_BASE + 0x18C))
#define CAN2_RIR0  (*(volatile unsigned int *)(CAN2_BASE + 0x1B0))
#define CAN2_RDLR0 (*(volatile unsigned int *)(CAN2_BASE + 0x1B8))
#define CAN2_RDHR0 (*(volatile unsigned int *)(CAN2_BASE + 0x1BC))
#define CAN2_RIR1  (*(volatile unsigned int *)(CAN2_BASE + 0x1C0))
#define CAN2_RDLR1 (*(volatile unsigned int *)(CAN2_BASE + 0x1C8))
#define CAN2_RDHR1 (*(volatile unsigned int *)(CAN2_BASE + 0x1CC))
#define CAN2_RIR2  (*(volatile unsigned int *)(CAN2_BASE + 0x1D0))
#define CAN2_RDLR2 (*(volatile unsigned int *)(CAN2_BASE + 0x1D8))
#define CAN2_RDHR2 (*(volatile unsigned int *)(CAN2_BASE + 0x1DC))
#define CAN2_FMR   (*(volatile unsigned int *)(CAN2_BASE + 0x200))
#define CAN2_FS1R  (*(volatile unsigned int *)(CAN2_BASE + 0x20C))
#define CAN2_FA1R  (*(volatile unsigned int *)(CAN2_BASE + 0x21C))
#define CAN2_FILT0 (*(volatile unsigned int *)(CAN2_BASE + 0x240))
#define CAN2_FILT1 (*(volatile unsigned int *)(CAN2_BASE + 0x244))

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // RCC APB2 USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
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

static void uart_dec(unsigned int v) {
    char buf[12]; int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

// Init a controller: enter init mode, pass-all filter (bank 0, 32-bit,
// mask 0), back to normal mode.
static void can_init(volatile unsigned int *mcr, volatile unsigned int *btr,
                     volatile unsigned int *fmr, volatile unsigned int *fs1r,
                     volatile unsigned int *fa1r,
                     volatile unsigned int *filt0) {
    *mcr = 1; // INRQ: enter init mode
    *btr = 0x00000034; // 16 MHz APB, ~500 kbit
    *fmr = 1; // FINIT
    *fs1r = 0xFFFFFFFF; // all 32-bit scale
    *filt0 = 0; // id = 0
    *(filt0 + 1) = 0; // mask = 0 (pass all)
    *fa1r = 1; // bank 0 active
    *fmr = 0; // FINIT=0
    *mcr = 0; // INRQ=0: normal mode
}

static void can_tx(volatile unsigned int *tir, volatile unsigned int *tdtr,
                   volatile unsigned int *tdlr, volatile unsigned int *tdhr,
                   unsigned int id, const unsigned char *d) {
    *tdtr = 8;
    *tdlr = d[0] | (d[1] << 8) | (d[2] << 16) | (d[3] << 24);
    *tdhr = d[4] | (d[5] << 8) | (d[6] << 16) | (d[7] << 24);
    *tir = (id << 21) | 1; // TXRQ
}

// Drain every pending FIFO0 frame (mailboxes 0..FMP-1, oldest first) into
// out_id/out_data and release each. Returns the number of frames read.
static int can_drain(volatile unsigned int *rf0r,
                     volatile unsigned int *rir0, volatile unsigned int *rdlr0,
                     volatile unsigned int *rdhr0,
                     volatile unsigned int *rir1, volatile unsigned int *rdlr1,
                     volatile unsigned int *rdhr1,
                     volatile unsigned int *rir2, volatile unsigned int *rdlr2,
                     volatile unsigned int *rdhr2,
                     unsigned int *out_id, unsigned long long *out_data,
                     int max) {
    int n = 0;
    while ((*rf0r & 3) && n < max) {
        volatile unsigned int *r, *l, *h;
        if (n == 0) { r = rir0; l = rdlr0; h = rdhr0; }
        else if (n == 1) { r = rir1; l = rdlr1; h = rdhr1; }
        else { r = rir2; l = rdlr2; h = rdhr2; }
        out_id[n] = ((*r) >> 21) & 0x7FF;
        out_data[n] = (unsigned long long)(*l) | ((unsigned long long)(*h) << 32);
        n++;
        *rf0r = 0x20; // RFOM: release
    }
    return n;
}

int main(void) {
    unsigned int id[4];
    unsigned long long data[4];

    uart_init();
    uart_puts("=== CAN Test ===\r\n");
    RCC_APB1ENR |= (1 << 25) | (1 << 26); // CAN1, CAN2 clocks

    // ===== Phase 1: loopback (LBKM) =====
    uart_puts("CAN1 init loopback\r\n");
    can_init(&CAN_MCR, &CAN_BTR, &CAN_FMR, &CAN_FS1R, &CAN_FA1R, &CAN_FILT0);
    CAN_BTR |= (1 << 30); // LBKM
    CAN_IER = 0x7F;       // all TX/RX interrupts on
    can_tx(&CAN_TIR0, &CAN_TDTR0, &CAN_TDLR0, &CAN_TDHR0, 0x123,
           (const unsigned char *)"CANLOOP!");
    for (int i = 0; i < 200000 && !(CAN_TSR & (1 << 16)); i++); // wait TME0
    uart_puts("loopback TX done\r\n");
    for (int i = 0; i < 200000 && !(CAN_RF0R & 1); i++); // wait FMP0 >= 1
    if (!(CAN_RF0R & 1)) {
        uart_puts("CAN loopback FAIL (no RX)\r\n");
        uart_puts("=== CAN Test: FAIL ===\r\n");
        while (1);
    }
    {
        unsigned int tir = CAN_RIR0;
        unsigned long long d = (unsigned long long)CAN_RDLR0
                            | ((unsigned long long)CAN_RDHR0 << 32);
        if ((((tir >> 21) & 0x7FF) == 0x123) && (d == 0x21504F4F4C4E4143ULL)) {
            uart_puts("CAN loopback OK: id=0x123 data=CANLOOP!\r\n");
        } else {
            uart_puts("CAN loopback FAIL: id=");
            uart_hex32((tir >> 21) & 0x7FF);
            uart_puts(" data=");
            uart_hex32((unsigned int)d);
            uart_hex32((unsigned int)(d >> 32));
            uart_puts("\r\n=== CAN Test: FAIL ===\r\n");
            while (1);
        }
    }
    CAN_RF0R = 0x20; // release

    // ===== Phase 2: two nodes, arbitration =====
    uart_puts("CAN1+CAN2 arbitration\r\n");
    CAN_BTR &= ~(1 << 30); // clear LBKM
    can_init(&CAN2_MCR, &CAN2_BTR, &CAN2_FMR, &CAN2_FS1R, &CAN2_FA1R, &CAN2_FILT0);
    CAN2_IER = 0x7F;
    // Stage both TX requests back-to-back (same arbitration round):
    can_tx(&CAN2_TIR0, &CAN2_TDTR0, &CAN2_TDLR0, &CAN2_TDHR0, 0x200,
           (const unsigned char *)"HELLO-2!");
    can_tx(&CAN_TIR0, &CAN_TDTR0, &CAN_TDLR0, &CAN_TDHR0, 0x300,
           (const unsigned char *)"HI-CAN1!");
    for (int i = 0; i < 200000 && !(CAN_TSR & (1 << 16)); i++);
    for (int i = 0; i < 200000 && !(CAN2_TSR & (1 << 16)); i++);
    uart_puts("both TX done\r\n");

    // Both nodes must end with both frames (broadcast + loser retry).
    int n1 = 0, n2 = 0;
    for (int i = 0; i < 200000 && (n1 < 2 || n2 < 2); i++) {
        n1 = can_drain(&CAN_RF0R, &CAN_RIR0, &CAN_RDLR0, &CAN_RDHR0,
                       &CAN_RIR1, &CAN_RDLR1, &CAN_RDHR1,
                       &CAN_RIR2, &CAN_RDLR2, &CAN_RDHR2, id, data, 4);
        n2 = can_drain(&CAN2_RF0R, &CAN2_RIR0, &CAN2_RDLR0, &CAN2_RDHR0,
                       &CAN2_RIR1, &CAN2_RDLR1, &CAN2_RDHR1,
                       &CAN2_RIR2, &CAN2_RDLR2, &CAN2_RDHR2, id, data, 4);
        (void)data;
    }
    uart_puts("CAN1 RX frames=");
    uart_dec(n1);
    uart_puts(" CAN2 RX frames=");
    uart_dec(n2);
    uart_puts("\r\n");

    if (n1 >= 2 && n2 >= 2 && !!(CAN_TSR & (1 << 16)) && !!(CAN2_TSR & (1 << 16))) {
        uart_puts("CAN arbitration OK\r\n");
    } else {
        uart_puts("CAN arbitration FAIL\r\n");
        uart_puts("=== CAN Test: FAIL ===\r\n");
        while (1);
    }
    uart_puts("=== CAN Test: done ===\r\n");
    while (1);
}