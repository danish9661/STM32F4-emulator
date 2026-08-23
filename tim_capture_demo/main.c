#define TIM3_BASE 0x40000400
#define TIM3_CR1  (*(volatile unsigned int *)(TIM3_BASE + 0x00))
#define TIM3_SR   (*(volatile unsigned int *)(TIM3_BASE + 0x10))
#define TIM3_CCMR1 (*(volatile unsigned int *)(TIM3_BASE + 0x18))
#define TIM3_CCER (*(volatile unsigned int *)(TIM3_BASE + 0x20))
#define TIM3_CNT  (*(volatile unsigned int *)(TIM3_BASE + 0x24))
#define TIM3_PSC  (*(volatile unsigned int *)(TIM3_BASE + 0x28))
#define TIM3_ARR  (*(volatile unsigned int *)(TIM3_BASE + 0x2C))
#define TIM3_CCR1 (*(volatile unsigned int *)(TIM3_BASE + 0x34))

#define RCC_BASE 0x40023800
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x1C))

#define USART1_BASE 0x40011000
#define USART1_SR  (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART1_DR  (*(volatile unsigned int *)(USART1_BASE + 0x04))

static void uart_init(void) {
    unsigned int *rcc = (unsigned int *)0x40023830;
    unsigned int *gpioa = (unsigned int *)0x40020000;
    unsigned int *usart = (unsigned int *)0x40023844;
    *rcc |= (1u << 0);
    *(gpioa + 0x00) &= ~0xFFu;
    *(gpioa + 0x00) |= (0x2u << 18) | (0x2u << 20);
    *(gpioa + 0x08) &= ~0xFFu;
    *(gpioa + 0x08) |= (0x7u << 4) | (0x7u << 8);
    *usart |= (1u << 4);
    USART1_SR = 0xC0;
    USART1_DR = 0;
    *(volatile unsigned int *)(USART1_BASE + 0x0C) = 0x00000010;
    *(volatile unsigned int *)(USART1_BASE + 0x08) = 0x00000008;
    *(volatile unsigned int *)(USART1_BASE + 0x00) = 0x00000001;
}

static void uart_putchar(int c) {
    while (!(USART1_SR & 0x80)) {}
    USART1_DR = (unsigned char)c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_print_u32(unsigned int v) {
    char buf[12];
    int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i--) uart_putchar(buf[i]);
}

static void delay_ms(unsigned int ms) {
    for (volatile unsigned int i = 0; i < ms * 4000; i++) {}
}

int main(void) {
    uart_init();
    uart_puts("=== TIM Input Capture Demo ===\r\n");

    RCC_APB1ENR |= (1u << 1);   // TIM3 clock

    TIM3_CCMR1 = 0x01;          // CC1S = 0b01 (input capture TI1)
    TIM3_CCER = 0x01;           // CC1E: capture enable on CH1
    TIM3_PSC = 0;
    TIM3_ARR = 0xFFFF;
    TIM3_CNT = 0;
    TIM3_CR1 = 0x01;            // CEN: start counting

    uart_puts("TIM capture ready\r\n");

    unsigned int last = 0xFFFFFFFF;
    for (int i = 0; i < 300; i++) {
        unsigned int sr = TIM3_SR;
        if (sr & 2) {           // CC1IF: capture occurred
            unsigned int cap = TIM3_CCR1;
            if (cap != last) {
                uart_puts("cap=");
                uart_print_u32(cap);
                uart_puts("\r\n");
                last = cap;
            }
            TIM3_SR = sr & ~2u;  // clear CC1IF
        }
        delay_ms(1);
    }
    uart_puts("done\r\n");
    while (1) { delay_ms(10); }
}
