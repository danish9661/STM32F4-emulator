#define WWDG_BASE 0x40002C00
#define WWDG_CR  (*(volatile unsigned int *)(WWDG_BASE + 0x00))
#define WWDG_CFR (*(volatile unsigned int *)(WWDG_BASE + 0x04))

#define RCC_BASE 0x40023800
#define RCC_CSR  (*(volatile unsigned int *)(RCC_BASE + 0x74))

#define USART1_BASE 0x40011000
#define USART1_SR  (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART1_DR  (*(volatile unsigned int *)(USART1_BASE + 0x04))

static void uart_init(void) {
    unsigned int *rcc = (unsigned int *)0x40023830;   /* RCC_AHB1ENR */
    unsigned int *gpioa = (unsigned int *)0x40020000;  /* GPIOA base */
    unsigned int *usart = (unsigned int *)0x40023844;  /* RCC_APB2ENR */
    *rcc |= (1u << 0);          /* GPIOA clock */
    *(gpioa + 0x00) &= ~0xFFu;  /* MODER clear PA9/PA10 */
    *(gpioa + 0x00) |= (0x2u << 18) | (0x2u << 20);
    *(gpioa + 0x08) &= ~0xFFu;
    *(gpioa + 0x08) |= (0x7u << 4) | (0x7u << 8); /* AF7 */
    *usart |= (1u << 4);         /* USART1 clock */
    USART1_SR = 0xC0;
    USART1_DR = 0;
    *(volatile unsigned int *)(USART1_BASE + 0x0C) = 0x00000010; /* BRR ~115200 */
    *(volatile unsigned int *)(USART1_BASE + 0x08) = 0x00000008; /* TE */
    *(volatile unsigned int *)(USART1_BASE + 0x00) = 0x00000001; /* UE */
}

static void uart_putchar(int c) {
    while (!(USART1_SR & 0x80)) {}
    USART1_DR = (unsigned char)c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void delay_ms(unsigned int ms) {
    for (volatile unsigned int i = 0; i < ms * 4000; i++) {}
}

int main(void) {
    uart_init();
    uart_puts("=== WWDG Demo ===\r\n");

    unsigned int csr = RCC_CSR;
    if (csr & (1u << 30)) {
        uart_puts("WWDG reset detected\r\n");
        RCC_CSR = (1u << 24);  /* RMVF: clear reset flags */
    } else if (csr & (1u << 29)) {
        uart_puts("IWDG reset detected\r\n");
        RCC_CSR = (1u << 24);
    }

    /* Configure: prescaler /8 (WDGTB=3), window W=0x7F (no early-window
       restriction -> refresh any time counter > 0x3F). */
    WWDG_CFR = (3u << 7) | 0x7F;
    /* Enable + load counter to 0x7F. */
    WWDG_CR = 0x80 | 0x7F;

    uart_puts("WWDG started (pet every 2ms)\r\n");
    for (int i = 0; i < 5; i++) {
        WWDG_CR = 0x80 | 0x7F;   /* refresh: reload counter */
        delay_ms(2);
        uart_puts("alive ");
        uart_putchar('0' + i);
        uart_puts("\r\n");
    }

    uart_puts("stopping pet -> WWDG should reset\r\n");
    while (1) { delay_ms(10); }
}
