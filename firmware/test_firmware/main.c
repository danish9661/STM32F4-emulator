#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRL  (*(volatile unsigned int *)(GPIOA_BASE + 0x20))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))

#define TIM2_BASE   0x40000000
#define TIM_CR1     (*(volatile unsigned int *)(TIM2_BASE + 0x00))
#define TIM_CCMR1   (*(volatile unsigned int *)(TIM2_BASE + 0x18))
#define TIM_CCER    (*(volatile unsigned int *)(TIM2_BASE + 0x20))
#define TIM_CNT     (*(volatile unsigned int *)(TIM2_BASE + 0x24))
#define TIM_PSC     (*(volatile unsigned int *)(TIM2_BASE + 0x28))
#define TIM_ARR     (*(volatile unsigned int *)(TIM2_BASE + 0x2C))
#define TIM_CCR1    (*(volatile unsigned int *)(TIM2_BASE + 0x34))

#define ADC1_BASE   0x40012000
#define ADC_SR      (*(volatile unsigned int *)(ADC1_BASE + 0x00))
#define ADC_CR1     (*(volatile unsigned int *)(ADC1_BASE + 0x04))
#define ADC_CR2     (*(volatile unsigned int *)(ADC1_BASE + 0x08))
#define ADC_SMPR2   (*(volatile unsigned int *)(ADC1_BASE + 0x10))
#define ADC_SQR3    (*(volatile unsigned int *)(ADC1_BASE + 0x34))
#define ADC_DR      (*(volatile unsigned int *)(ADC1_BASE + 0x4C))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

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
    while (!(USART_SR & (1 << 7)));
    USART_DR = ' ';
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 8);

    GPIOA_MODER &= ~(3 << 0);
    GPIOA_MODER |=  (2 << 0);
    GPIOA_AFRL  &= ~(0xF << 0);
    GPIOA_AFRL  |=  (1 << 0);

    GPIOA_MODER &= ~(3 << 2);
    GPIOA_MODER |=  (3 << 2);

    uart_init();
    uart_puts("TIM2 ADC test\r\n");

    TIM_PSC = 0;
    TIM_ARR = 1000;
    TIM_CCR1 = 500;
    TIM_CCMR1 = (0x68);  // PWM1, preload
    TIM_CCER |= 1;
    TIM_CR1 |= 1;

    TIM_CCR1 = 500;

    uart_puts("CR1=");
    uart_puthex(TIM_CR1);
    uart_puts("PSC=");
    uart_puthex(TIM_PSC);
    uart_puts("ARR=");
    uart_puthex(TIM_ARR);
    uart_puts("CCR1=");
    uart_puthex(TIM_CCR1);
    uart_puts("CCMR1=");
    uart_puthex(TIM_CCMR1);
    uart_puts("\r\n");

    ADC_CR2 |= 1;
    ADC_SMPR2 = 0;
    ADC_SQR3 = 1;

    for (int i = 0; i < 10; i++) {
        uart_puts("CNT=");
        uart_puthex(TIM_CNT);

        ADC_CR2 |= 1;
        volatile int d;
        for (d = 0; d < 1000; d++);
        ADC_CR2 |= 1;
        for (d = 0; d < 1000; d++);
        unsigned int adc = ADC_DR;

        uart_puts("ADC=");
        uart_puthex(adc);
        uart_puts("\r\n");
    }

    uart_puts("DONE\r\n");
    while (1);
}
