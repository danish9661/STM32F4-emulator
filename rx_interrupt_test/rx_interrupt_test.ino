#define RCC_AHB1ENR (*(volatile unsigned int *)0x40023830)
#define RCC_AHB2ENR (*(volatile unsigned int *)0x40023834)
#define RCC_APB1ENR (*(volatile unsigned int *)0x40023840)
#define RCC_APB2ENR (*(volatile unsigned int *)0x40023844)

#define GPIOA_MODER (*(volatile unsigned int *)0x40020000)
#define GPIOA_AFRL  (*(volatile unsigned int *)0x40020020)

#define USART1_SR  (*(volatile unsigned int *)0x40011000)
#define USART1_DR  (*(volatile unsigned int *)0x40011004)
#define USART1_BRR (*(volatile unsigned int *)0x40011008)
#define USART1_CR1 (*(volatile unsigned int *)0x4001100C)

#define NVIC_ISER1 (*(volatile unsigned int *)0xE000E104)

#define CRC_DR  (*(volatile unsigned int *)0x40023008)
#define CRC_CR  (*(volatile unsigned int *)0x40023000)

#define HASH_CR  (*(volatile unsigned int *)0x50060400)
#define HASH_DIN (*(volatile unsigned int *)0x50060408)
#define HASH_STR (*(volatile unsigned int *)0x50060410)
#define HASH_HR0 (*(volatile unsigned int *)0x50060418)
#define HASH_HR1 (*(volatile unsigned int *)0x5006041C)
#define HASH_HR2 (*(volatile unsigned int *)0x50060420)
#define HASH_HR3 (*(volatile unsigned int *)0x50060424)
#define HASH_HR4 (*(volatile unsigned int *)0x50060428)

static volatile unsigned int rx_count = 0;
static volatile unsigned int rx_buf[64];
static volatile unsigned int rx_done = 0;

// USART1 IRQ 37 handler - vector table entry at 0x080000D4 (0x100 + 37*4)
void USART1_IRQHandler(void) __attribute__((interrupt));
void USART1_IRQHandler(void) {
    unsigned int sr = USART1_SR;
    if (sr & (1 << 5)) {
        unsigned char c = USART1_DR;
        if (rx_count < 64) {
            rx_buf[rx_count++] = c;
        }
        if (c == '\n' || c == '\r') {
            rx_done = 1;
        }
    }
}

static void uart_puts(const char *s) {
    while (*s) {
        while (!(USART1_SR & (1 << 7)));
        USART1_DR = *s++;
    }
}

static void uart_puthex(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART1_SR & (1 << 7)));
        USART1_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

void setup() {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);

    GPIOA_MODER &= ~(3 << 0);
    GPIOA_MODER |= (2 << 0);
    GPIOA_AFRL &= ~(0xF << 0);
    GPIOA_AFRL |= (7 << 0);
    GPIOA_MODER &= ~(3 << 2);
    GPIOA_MODER |= (2 << 2);
    GPIOA_AFRL &= ~(0xF << 4);
    GPIOA_AFRL |= (7 << 4);

    USART1_BRR = 139;
    USART1_CR1 = (1 << 13) | (1 << 3) | (1 << 2) | (1 << 5); // UE|TE|RE|RXNEIE

    NVIC_ISER1 |= (1 << 5);

    RCC_AHB1ENR |= (1 << 12);
    CRC_CR |= 1;

    RCC_AHB2ENR |= (1 << 4);

    uart_puts("RX-INT-TEST\n");
}

void loop() {
    if (!rx_done) {
        unsigned int *sr = (unsigned int *)0x40011000;
        unsigned int *dr = (unsigned int *)0x40011004;
        return;
    }

    rx_done = 0;

    CRC_CR |= 1;
    while (CRC_CR & 1);

    // Feed received bytes to CRC
    for (unsigned int i = 0; i < rx_count; i++) {
        unsigned int v = rx_buf[i] & 0xFF;
        CRC_DR = v;
    }

    unsigned int crc_result = CRC_DR;

    uart_puts("CRC=");
    uart_puthex(crc_result);
    uart_puts("\n");

    unsigned int saved_rx_count = rx_count;
    rx_count = 0;
}
