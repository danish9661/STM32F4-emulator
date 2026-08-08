// Blinky: minimal non-ethernet STM32F407 firmware.
// UART banner + LED on GPIOA PA5 toggling at ~2 Hz, with a tick counter.
// No ETH, no DMA, no interrupts — exercises USART + GPIO + SRAM only.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_ODR   (*(volatile unsigned int *)(GPIOA_BASE + 0x14))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define LED_PIN     5

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA clock
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // USART1 clock (APB2)
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

static void uart_u32(unsigned int v) {
    char buf[12];
    int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

static void led_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA clock (idempotent)
    GPIOA_MODER = (GPIOA_MODER & ~(3u << (LED_PIN * 2))) | (1u << (LED_PIN * 2)); // PA5 output
    GPIOA_ODR &= ~(1u << LED_PIN);
}

static void led_set(int on) {
    if (on) GPIOA_ODR |= (1u << LED_PIN);
    else    GPIOA_ODR &= ~(1u << LED_PIN);
}

static void delay_ms(int n) {
    // ~4000 nops/ms — same convention as the eth_* firmwares
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

int main(void) {
    uart_init();
    uart_puts("=== Blinky ===\r\n");
    uart_puts("LED: GPIOA PA5\r\n");
    uart_puts("UART: 115200 8N1\r\n");
    uart_puts("No ethernet required\r\n");

    led_init();

    unsigned int tick = 0;
    for (;;) {
        led_set(1);
        uart_puts("tick ");
        uart_u32(tick);
        uart_puts(" LED=ON\r\n");
        delay_ms(100);

        led_set(0);
        uart_puts("tick ");
        uart_u32(tick);
        uart_puts(" LED=OFF\r\n");
        delay_ms(100);

        tick++;
    }
}
