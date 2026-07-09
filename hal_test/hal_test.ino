// No init() override — let Arduino's real init run

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile uint32_t *)(USART1_BASE + 0x0C))

#define STK_CTRL  (*(volatile uint32_t*)0xE000E010)
#define STK_LOAD  (*(volatile uint32_t*)0xE000E014)
#define STK_VAL   (*(volatile uint32_t*)0xE000E018)

static void tx_c(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}

static void tx_s(const char *s) { while (*s) tx_c(*s++); }

void setup() {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
    tx_s("HAL INIT OK\n");

    STK_LOAD = 1000;
    STK_CTRL = 1;
    uint32_t a = STK_VAL;
    volatile int d;
    for (d = 0; d < 100; d++);
    uint32_t b = STK_VAL;
    if (a != b) tx_s("SYSTICK OK\n");
    else        tx_s("SYSTICK FAIL\n");
}

void loop() {
    tx_s("LOOP\n");
    volatile uint32_t d;
    for (d = 0; d < 500000; d++);
}
