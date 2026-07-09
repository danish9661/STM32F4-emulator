extern "C" void init(void) {
}

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile uint32_t *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile uint32_t *)(RCC_BASE + 0x40))
#define RCC_APB2ENR (*(volatile uint32_t *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile uint32_t *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRL  (*(volatile uint32_t *)(GPIOA_BASE + 0x20))

#define TIM2_BASE   0x40000000
#define TIM_CR1     (*(volatile uint32_t *)(TIM2_BASE + 0x00))
#define TIM_CCMR1   (*(volatile uint32_t *)(TIM2_BASE + 0x18))
#define TIM_CCER    (*(volatile uint32_t *)(TIM2_BASE + 0x20))
#define TIM_CNT     (*(volatile uint32_t *)(TIM2_BASE + 0x24))
#define TIM_PSC     (*(volatile uint32_t *)(TIM2_BASE + 0x28))
#define TIM_ARR     (*(volatile uint32_t *)(TIM2_BASE + 0x2C))
#define TIM_CCR1    (*(volatile uint32_t *)(TIM2_BASE + 0x34))

#define ADC1_BASE   0x40012000
#define ADC_CR2     (*(volatile uint32_t *)(ADC1_BASE + 0x08))
#define ADC_SMPR2   (*(volatile uint32_t *)(ADC1_BASE + 0x10))
#define ADC_SQR3    (*(volatile uint32_t *)(ADC1_BASE + 0x34))
#define ADC_DR      (*(volatile uint32_t *)(ADC1_BASE + 0x4C))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile uint32_t *)(USART1_BASE + 0x0C))

static void uart_puts(const char *s) {
    while (*s) {
        while (!(USART_SR & (1 << 7)));
        USART_DR = *s++;
    }
}

static void uart_puthex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7)));
        USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

void setup() {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 8);
    RCC_APB2ENR |= (1 << 4);

    GPIOA_MODER &= ~(3 << 0);
    GPIOA_MODER |=  (2 << 0);
    GPIOA_AFRL  &= ~(0xF << 0);
    GPIOA_AFRL  |=  (1 << 0);

    GPIOA_MODER &= ~(3 << 2);
    GPIOA_MODER |=  (3 << 2);

    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);

    TIM_PSC = 0;
    TIM_ARR = 1000;
    TIM_CCR1 = 500;
    TIM_CCMR1 = 0x68;
    TIM_CCER |= 1;
    TIM_CR1 |= 1;

    ADC_CR2 |= 1;
    ADC_SMPR2 = 0;
    ADC_SQR3 = 1;

    uart_puts("ARDUINO OK\r\n");
}

void loop() {
    uart_puts("CNT=");
    uart_puthex(TIM_CNT);
    uart_puts(" ADC=");
    uart_puthex(ADC_DR);
    uart_puts("\r\n");

    volatile uint32_t d;
    for (d = 0; d < 1000000; d++);
}
