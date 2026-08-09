#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_PUPDR (*(volatile unsigned int *)(GPIOA_BASE + 0x0C))
#define GPIOA_ODR   (*(volatile unsigned int *)(GPIOA_BASE + 0x14))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))

#define SYSCFG_BASE 0x40013800
#define SYSCFG_EXTICR1 (*(volatile unsigned int *)(SYSCFG_BASE + 0x08))

#define EXTI_BASE   0x40013C00
#define EXTI_IMR    (*(volatile unsigned int *)(EXTI_BASE + 0x00))
#define EXTI_RTSR   (*(volatile unsigned int *)(EXTI_BASE + 0x08))
#define EXTI_FTSR   (*(volatile unsigned int *)(EXTI_BASE + 0x0C))
#define EXTI_PR     (*(volatile unsigned int *)(EXTI_BASE + 0x14))

#define NVIC_ISER0  (*(volatile unsigned int *)0xE000E100)

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static volatile unsigned int exti0_count = 0;

static void uart_init(void) {
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER &= ~((3 << 18) | (3 << 20));
    GPIOA_MODER |=  ((2 << 18) | (2 << 20));
    GPIOA_AFRH  |=  ((7 << 4) | (7 << 8));
    USART_CR1 = 0;
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_puts(const char *s) {
    while (*s) {
        while (!(USART_SR & (1 << 7)));
        USART_DR = *s++;
    }
}

static void uart_puthex(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7)));
        USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

static void test(const char *name, int cond) {
    uart_puts("  ");
    uart_puts(cond ? "PASS" : "FAIL");
    uart_puts(" ");
    uart_puts(name);
    uart_puts("\r\n");
}

void EXTI0_IRQHandler(void) {
    if (EXTI_PR & (1 << 0)) {
        EXTI_PR = (1 << 0); // clear pending
        exti0_count++;
    }
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    RCC_APB2ENR |= (1 << 14); // SYSCFG
    uart_init();
    uart_puts("\r\n=== EXTI test ===\r\n");

    // PA0 input, no pull
    GPIOA_MODER &= ~(3 << 0);
    GPIOA_PUPDR &= ~(3 << 0);

    // EXTI0 -> PA (EXTICR1 lines[3:0] = 0)
    SYSCFG_EXTICR1 &= ~(0xF << 0);

    // EXTI0 rising edge enabled, unmasked
    EXTI_RTSR = (1 << 0);
    EXTI_FTSR = 0;
    EXTI_IMR = (1 << 0);

    // NVIC: enable IRQ6 (EXTI0)
    NVIC_ISER0 = (1 << 6);

    uart_puts("  waiting for PA0 rising edge...\r\n");

    // The JS driver raises PA0 (gpio_set_input). We wait for the ISR to run.
    int spins = 0;
    while (exti0_count == 0 && spins < 200000000) spins++;

    test("EXTI0 fired once", exti0_count == 1);
    test("PR cleared by handler", (EXTI_PR & (1 << 0)) == 0);

    // second edge
    exti0_count = 0;
    uart_puts("  waiting for PA0 2nd edge...\r\n");
    spins = 0;
    while (exti0_count == 0 && spins < 200000000) spins++;
    test("EXTI0 fired on 2nd edge", exti0_count == 1);

    uart_puts("EXTI TEST DONE\r\n");
    while (1);
}
