// Ethernet MAC register base
#define ETH_MAC_BASE    0x40028000
#define ETH_DMA_BASE    0x40029000

#define MACCR   (*(volatile unsigned int *)(ETH_MAC_BASE + 0x00))
#define MACFFR  (*(volatile unsigned int *)(ETH_MAC_BASE + 0x04))
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

// ARP request packet (60 bytes minimum Ethernet frame)
static volatile unsigned char arp_pkt[60] __attribute__((aligned(4))) = {
    // Ethernet header: dst MAC (broadcast), src MAC, type=ARP
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,           // dst
    0x02, 0x00, 0x00, 0x00, 0x00, 0x01,           // src
    0x08, 0x06,                                     // ARP
    // ARP header
    0x00, 0x01, // HTYPE = Ethernet
    0x08, 0x00, // PTYPE = IPv4
    0x06,       // HLEN = 6
    0x04,       // PLEN = 4
    0x00, 0x01, // OPER = request
    // SHA: 02:00:00:00:00:01
    0x02, 0x00, 0x00, 0x00, 0x00, 0x01,
    // SPA: 10.0.2.15
    0x0A, 0x00, 0x02, 0x0F,
    // THA: 00:00:00:00:00:00
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // TPA: 10.0.2.2
    0x0A, 0x00, 0x02, 0x02,
    // Pad to 60 bytes
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
};

// TX descriptor (normal format): tdes0, tdes1
static volatile unsigned int tx_desc[2] __attribute__((aligned(8))) = { 0, 0 };
static volatile int eth_done = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { // TS bit
        eth_done = 1;
        DMASR = (1 << 0) | (1 << 16) | (1 << 14); // write-1-to-clear TS+NIS+AIS
    }
}

static void wait_ms(volatile int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

int main(void) {
    uart_init();
    uart_puts("ETH Test: starting\r\n");

    // Enable ETH clock (AHB1 bit 25)
    RCC_AHB1ENR |= (1 << 25);
    uart_puts("ETH clock enabled\r\n");

    // Reset DMA
    DMABMR |= 1;
    wait_ms(10);
    uart_puts("DMA reset done\r\n");

    // Enable MAC TX and RX (RE=2, TE=3)
    MACCR = (1 << 2) | (1 << 3) | (1 << 11); // RE + TE + DM
    uart_puts("MAC enabled\r\n");

    // Set MAC address 02:00:00:00:00:01
    MACA0HR = 0x0000FFFF | (1 << 31); // AE bit, bits 31:16 = 0000, bits 15:0 = FFFF -> MAC = 00:00:00:FF:FF:FF
    MACA0LR = 0x02000001; // MAC address low: 02:00:00:01

    // Wait for MII link
    uart_puts("Waiting for link...\r\n");

    // Enable DMA TX and RX
    DMAOMR = (1 << 13) | (1 << 1); // ST + SR
    uart_puts("DMA enabled\r\n");

    // Enable normal interrupt summary (NIS) with TX completion
    DMAIER = (1 << 16) | (1 << 0); // NIE + TSE
    NVIC_ISER1 |= (1 << (61 - 32)); // IRQ 61 in ISER1 (bit 29)

    uart_puts("Setup complete. Starting TX...\r\n");

    // Build TX descriptor chain with one descriptor
    tx_desc[0] = 0x80000000 | (60 & 0x3FFF); // OWN=1, FS=0 (not set), LS=0, TCH=0, TBS1=60
    tx_desc[1] = (unsigned int)&arp_pkt[0];

    // Set TX descriptor list address
    DMATDLAR = (unsigned int)&tx_desc[0];

    // Poll transmit demand
    DMATPDR = 1;

    // Wait for TX completion
    for (int i = 0; i < 1000000; i++) {
        if (eth_done) break;
    }

    if (eth_done) {
        uart_puts("TX completed. tdes0=");
        uart_hex32(tx_desc[0]);
        uart_puts("\r\n");
    } else {
        uart_puts("TIMEOUT! dmasr=");
        uart_hex32(DMASR);
        uart_puts("\r\n");
    }

    uart_puts("ETH Test: done\r\n");
    while (1);
}
