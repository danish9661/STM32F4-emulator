// CAN host-injection RX demo: initializes CAN1 with a pass-all filter and
// polls the RX FIFO for frames injected by the host (via the emulator's
// canInject API — exercised by site/test_can_inject.mjs). Prints each
// received frame's ID and 8 data bytes.
// Expect:
//   === CAN Host RX ===
//   CAN RX ready (pass-all filter)
//   RX id=0x00000123 data=HELLO!!
#define CAN1_BASE 0x40006400

#define CAN_MCR   (*(volatile unsigned int *)(CAN1_BASE + 0x00))
#define CAN_TSR   (*(volatile unsigned int *)(CAN1_BASE + 0x08))
#define CAN_RF0R  (*(volatile unsigned int *)(CAN1_BASE + 0x0C))
#define CAN_IER   (*(volatile unsigned int *)(CAN1_BASE + 0x14))
#define CAN_BTR   (*(volatile unsigned int *)(CAN1_BASE + 0x1C))

#define CAN_RIR0  (*(volatile unsigned int *)(CAN1_BASE + 0x1B0))
#define CAN_RDLR0 (*(volatile unsigned int *)(CAN1_BASE + 0x1B8))
#define CAN_RDHR0 (*(volatile unsigned int *)(CAN1_BASE + 0x1BC))

#define CAN_FMR   (*(volatile unsigned int *)(CAN1_BASE + 0x200))
#define CAN_FS1R  (*(volatile unsigned int *)(CAN1_BASE + 0x20C))
#define CAN_FA1R  (*(volatile unsigned int *)(CAN1_BASE + 0x21C))
#define CAN_FILT0 (*(volatile unsigned int *)(CAN1_BASE + 0x240))

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

// Init CAN1: enter init mode, pass-all filter (bank 0, 32-bit, mask 0), back
// to normal mode.
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

int main(void) {
    uart_init();
    uart_puts("=== CAN Host RX ===\r\n");
    RCC_APB1ENR |= (1 << 25); // CAN1 clock
    can_init(&CAN_MCR, &CAN_BTR, &CAN_FMR, &CAN_FS1R, &CAN_FA1R, &CAN_FILT0);
    uart_puts("CAN RX ready (pass-all filter)\r\n");
    while (1) {
        if (CAN_RF0R & 3) { // FMP0 >= 1
            unsigned int id = (CAN_RIR0 >> 21) & 0x7FF;
            unsigned int dl = CAN_RDLR0, dh = CAN_RDHR0;
            uart_puts("RX id=0x");
            uart_hex32(id);
            uart_puts(" data=");
            for (int i = 0; i < 8; i++) {
                unsigned char c = (i < 4) ? (dl >> (i * 8)) : (dh >> ((i - 4) * 8));
                uart_putchar(c ? c : '.');
            }
            uart_puts("\r\n");
            CAN_RF0R = 0x20; // RFOM: release
        }
    }
}
