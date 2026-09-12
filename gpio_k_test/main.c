// GPIOK proof: PK3 output toggle with ODR readback.
// GPIO bank K exists only on F429-class silicon; the generic GPIO
// model covers A..K, and this is the firmware that drives a K pin.
// 64K RAM (see startup.c). UART banner + tick prints + PC13 toggling.
// No ETH, no DMA, no interrupts — USART + GPIO + SRAM only.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define GPIOK_BASE  0x40022800
#define GPIOK_MODER (*(volatile unsigned int *)(GPIOK_BASE + 0x00))
#define GPIOK_ODR   (*(volatile unsigned int *)(GPIOK_BASE + 0x14))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define LED_PIN     3

static void uart_init(void) {
    *(volatile unsigned int *)(RCC_BASE + 0x30) |= (1 << 0); // GPIOA clock
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

static void led_init(void) {
    RCC_AHB1ENR |= (1 << 10); // GPIOK clock
    GPIOK_MODER = (GPIOK_MODER & ~(3u << (LED_PIN * 2))) | (1u << (LED_PIN * 2)); // K3 output
    GPIOK_ODR &= ~(1u << LED_PIN);
}

static void led_set(int on) {
    if (on) GPIOK_ODR |= (1u << LED_PIN);
    else    GPIOK_ODR &= ~(1u << LED_PIN);
}

static void delay_ms(int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

static void uart_u32(unsigned int v) {
    char buf[12];
    int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

int main(void) {
    uart_init();
    led_init();
    uart_puts("=== GPIOK Test ===\r\n");
    for (int tick = 0; tick < 4; tick++) {
        led_set(1);
        uart_puts("tick ");
        uart_u32(tick);
        uart_puts(" PK3=ON odr=");
        uart_u32(GPIOK_ODR & 0x08u ? 1u : 0u);
        uart_puts("\r\n");
        delay_ms(100);
        led_set(0);
        uart_puts("tick ");
        uart_u32(tick);
        uart_puts(" PK3=OFF odr=");
        uart_u32(GPIOK_ODR & 0x08u ? 1u : 0u);
        uart_puts("\r\n");
        delay_ms(100);
    }
    uart_puts("GPIOK OK\r\n");
    for (;;);
}
