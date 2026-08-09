// Interrupt-driven Ethernet test: the entire TX/RX completion path runs
// through NVIC ETH IRQ 61 — no SRAM polling of a driver-written flag.
// The ISR reads DMASR (TS/RS), sets eth_irq_flag, scans RX descriptors,
// re-arms them, and write-1-clears DMASR. The JS driver only signals the
// model (eth_tx_done/eth_rx_done) and injects frames; it must NOT touch
// SRAM flags for this firmware.
#define ETH_MAC_BASE    0x40028000
#define ETH_DMA_BASE    0x40029000

#define MACCR   (*(volatile unsigned int *)(ETH_MAC_BASE + 0x00))
#define MACMIIAR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x10))
#define MACMIIDR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x14))
#define MACA0HR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x40))
#define MACA0LR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x44))

#define DMABMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x00))
#define DMATPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x04))
#define DMARPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x08))
#define DMARDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x0C))
#define DMATDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x10))
#define DMASR   (*(volatile unsigned int *)(ETH_DMA_BASE + 0x14))
#define DMAOMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x18))
#define DMAIER  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x1C))

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)

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

// No .data copy / .bss zero in this startup — init everything explicitly.
static volatile unsigned int tx_desc[2] __attribute__((aligned(8)));
static volatile unsigned int rx_desc[2] __attribute__((aligned(8)));
static volatile unsigned char tx_pkt[60] __attribute__((aligned(4)));
static volatile unsigned char rx_pkt[60] __attribute__((aligned(4)));
static volatile unsigned int eth_irq_flag;
static volatile unsigned int rx_frame_len;

static void frame_init(volatile unsigned char *p,
                       unsigned int dst0, unsigned int dst1,
                       unsigned int src0, unsigned int src1,
                       unsigned int type) {
    int i;
    for (i = 0; i < 60; i++) p[i] = 0;
    p[0] = 0x02; p[1] = 0x00; p[2] = 0x00; p[3] = 0x00; p[4] = 0x00; p[5] = dst0;
    p[6] = 0x02; p[7] = 0x00; p[8] = 0x00; p[9] = 0x00; p[10] = 0x00; p[11] = src0;
    p[12] = (type >> 8) & 0xFF; p[13] = type & 0xFF;
}

static void eth_send_packet(volatile unsigned char *pkt, unsigned int len) {
    eth_irq_flag &= ~1u;
    tx_desc[0] = 0x80000000 | (len & 0x3FFF); // OWN + TBS1
    tx_desc[1] = (unsigned int)pkt;
    DMATDLAR = (unsigned int)&tx_desc[0];
    DMATPDR = 1;
    for (int i = 0; i < 5000000 && !(eth_irq_flag & 1); i++);
}

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { // TS — TX done
        eth_irq_flag |= 1;
        DMASR = (1 << 0) | (1 << 16) | (1 << 14); // w1c TS + NIS + AIS
    }
    if (sr & (1 << 6)) { // RS — RX done
        if ((rx_desc[0] & 0x80000000) == 0) { // CPU owns the frame
            rx_frame_len = (rx_desc[0] >> 16) & 0x3FFF;
            eth_irq_flag |= 2;
            rx_desc[0] = 0x80000000 | 60; // re-arm: DMA owns
            DMARPDR = 1;
        }
        DMASR = (1 << 6) | (1 << 16) | (1 << 14); // w1c RS + NIS + AIS
    }
}

int main(void) {
    uart_init();
    uart_puts("=== ETH IRQ Test ===\r\n");

    RCC_AHB1ENR |= (1 << 25); // ETH clock
    uart_puts("ETH clock ON\r\n");

    DMABMR |= 1; // DMA soft reset
    for (volatile int i = 0; i < 8000; i++);
    uart_puts("DMA reset\r\n");

    MACCR = (1 << 2) | (1 << 3) | (1 << 11); // RE + TE + DM
    MACA0HR = 0x0000FFFF | (1 << 31); // AE
    MACA0LR = 0x02000001; // 02:00:00:00:00:01
    uart_puts("MAC addr set\r\n");

    DMAOMR = (1 << 13) | (1 << 1); // ST + SR
    uart_puts("DMA ST+SR\r\n");

    // RX descriptor: DMA owns, buffer at rx_pkt
    rx_desc[0] = 0x80000000 | 60;
    rx_desc[1] = (unsigned int)&rx_pkt[0];
    DMARDLAR = (unsigned int)&rx_desc[0];

    // Interrupts on TX done (bit0) + RX done (bit6) + NIS
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6);
    NVIC_ISER1 |= (1 << (61 - 32)); // ETH IRQ 61 -> ISER1 bit 29
    uart_puts("ETH IRQ enabled\r\n");

    frame_init(tx_pkt, 1, 2, 2, 1, 0x1234); // dst ...:01, src ...:02
    const char *payload = "ETH IRQ PING";
    for (int i = 0; payload[i]; i++) tx_pkt[14 + i] = payload[i];

    uart_puts("TX PING\r\n");
    eth_send_packet(tx_pkt, 60);
    if (eth_irq_flag & 1) uart_puts("TX done via IRQ\r\n");
    else uart_puts("TX TIMEOUT\r\n");

    // Wait for the RX interrupt (harness injects a frame). Re-arm the RX
    // poll periodically so the model's poll desc addr tracks DMARDLAR.
    for (int i = 0; i < 5000000 && !(eth_irq_flag & 2); i++) {
        if ((i & 0x3FF) == 0) DMARPDR = 1;
    }
    if (eth_irq_flag & 2) {
        uart_puts("RX via IRQ len=");
        uart_dec(rx_frame_len);
        uart_puts(" rdes0=");
        uart_hex32(rx_desc[0]);
        uart_puts("\r\n");
        // Echo the frame back (PONG)
        frame_init(tx_pkt, 2, 2, 1, 1, 0x1234); // dst ...:02, src ...:01
        for (int i = 0; payload[i]; i++) tx_pkt[14 + i] = payload[i];
        tx_pkt[14] = 'P'; tx_pkt[15] = 'O'; tx_pkt[16] = 'N'; tx_pkt[17] = 'G';
        eth_send_packet(tx_pkt, 60);
        if (eth_irq_flag & 1) uart_puts("PONG TX via IRQ\r\n");
        else uart_puts("PONG TX TIMEOUT\r\n");
    } else {
        uart_puts("RX TIMEOUT dmasr=");
    }

    uart_puts("ETH IRQ Test: done\r\n");
    while (1);
}
