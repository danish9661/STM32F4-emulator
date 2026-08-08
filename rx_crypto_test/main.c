#define USART1_BASE 0x40011000
#define USART1_SR  (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART1_DR  (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART1_BRR (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART1_CR1 (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define RCC_AHB1ENR (*(volatile unsigned int *)0x40023830)
#define RCC_AHB2ENR (*(volatile unsigned int *)0x40023834)
#define RCC_APB2ENR (*(volatile unsigned int *)0x40023844)

#define GPIOA_MODER (*(volatile unsigned int *)0x40020000)
#define GPIOA_AFRL  (*(volatile unsigned int *)0x40020020)

#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)

#define CRC_DR    (*(volatile unsigned int *)0x40023000)
#define CRC_CR    (*(volatile unsigned int *)0x40023008)
#define CRC_IDR   (*(volatile unsigned int *)0x40023004)

extern volatile int rx_flag;
extern volatile unsigned int rx_byte;
extern volatile int rx_interrupt_fired;

static void uart_puts(const char *s) {
    while (*s) {
        while (!(USART1_SR & (1 << 7)));
        USART1_DR = *s++;
    }
}

static void uart_puthex8(unsigned int v) {
    for (int i = 1; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART1_SR & (1 << 7)));
        USART1_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

static void uart_puthex32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART1_SR & (1 << 7)));
        USART1_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

int main(void) {
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
    USART1_CR1 = (1 << 13) | (1 << 3) | (1 << 2) | (1 << 5);

    // Enable CRC clock
    RCC_AHB1ENR |= (1 << 12);
    CRC_CR |= 1;
    while (CRC_CR & 1);

    // Enable USART1 IRQ 37 in NVIC ISER1 (bit 5)
    NVIC_ISER1 |= (1 << 5);

    uart_puts("=== RX INT + CRC ===\n");
    uart_puts("Sending \"Hello\" via pipe...\n");

    // Wait for ISR to receive bytes
    int timeout = 50000000;
    while (!rx_flag && timeout--) { __asm__ volatile("nop"); }

    if (!rx_flag) {
        uart_puts("TIMEOUT - no interrupt\n");
    } else {
        uart_puts("INT fired! rx_interrupt_fired=");
        uart_puthex8(rx_interrupt_fired);
        uart_puts("\nCRC32=");
        uart_puthex32(CRC_DR);
        uart_puts("\n");

        // Check expected CRC32 of "Hello\n"
        unsigned int crc_hello = CRC_DR;

        // Reset CRC, feed "Hello\n" via polling to verify
        CRC_CR |= 1;
        while (CRC_CR & 1);
        const char *test = "Hello\n";
        for (const char *p = test; *p; p++) {
            CRC_DR = *p;
        }
        uart_puts("CRC32(poll)=");
        uart_puthex32(CRC_DR);
        uart_puts("\n");

        if (CRC_DR == crc_hello) {
            uart_puts("PASS: INT CRC matches polling\n");
        } else {
            uart_puts("FAIL: CRC mismatch\n");
        }
    }

    uart_puts("DONE\n");
    return 0;
}
