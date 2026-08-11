// buzzer_test: active buzzer driven by TIM2 CH1 PWM on PA0 (AF1).
// Plays a short melody by reprogramming ARR/CCR1; the JS layer
// (site/emulator.js buzzer device) reads the TIM2 registers and
// renders the tone with WebAudio.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRL  (*(volatile unsigned int *)(GPIOA_BASE + 0x20))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define TIM2_BASE   0x40000000
#define TIM_CR1     (*(volatile unsigned int *)(TIM2_BASE + 0x00))
#define TIM_CCMR1   (*(volatile unsigned int *)(TIM2_BASE + 0x18))
#define TIM_CCER    (*(volatile unsigned int *)(TIM2_BASE + 0x20))
#define TIM_PSC     (*(volatile unsigned int *)(TIM2_BASE + 0x28))
#define TIM_ARR     (*(volatile unsigned int *)(TIM2_BASE + 0x2C))
#define TIM_CCR1    (*(volatile unsigned int *)(TIM2_BASE + 0x34))

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    *(volatile unsigned int *)0x40023844 |= (1 << 4);
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA;
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70;
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

static void delay_ms(int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

// TIM2 CH1 PWM on PA0: PSC=83 -> 1 MHz tick. freq -> ARR = 1e6/f - 1.
static void buzzer_freq(unsigned int freq) {
    unsigned int arr;
    if (freq == 0) { arr = 0; }
    else { arr = 1000000 / freq - 1; }
    TIM_ARR = arr;
    TIM_CCR1 = freq == 0 ? 0 : arr / 2;   // 50% duty
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);             // GPIOA
    RCC_APB1ENR |= (1 << 0);             // TIM2
    uart_init();
    uart_puts("Buzzer test\r\n");
    uart_puts("TIM2 CH1 PWM @ PA0\r\n");

    // PA0 -> AF1
    GPIOA_MODER = (GPIOA_MODER & ~(3u << 0)) | (2u << 0);
    GPIOA_AFRL  = (GPIOA_AFRL & ~0xF) | 1;

    TIM_PSC = 83;                        // 84 MHz / 84 = 1 MHz
    TIM_CCMR1 = (TIM_CCMR1 & ~0xFF) | 0x60;  // OC1M = PWM mode 1
    TIM_CCER = (1 << 0);                 // CC1E
    TIM_CR1 = (1 << 0);                  // CEN

    static const unsigned int melody[][2] = {
        { 262, 200 },  // C4
        { 294, 200 },  // D4
        { 330, 200 },  // E4
        { 349, 200 },  // F4
        { 392, 200 },  // G4
        { 440, 200 },  // A4
        { 494, 200 },  // B4
        { 523, 400 },  // C5
        { 0,   200 },  // rest
        { 392, 200 },
        { 440, 200 },
        { 523, 600 },
    };
    static const int n_notes = sizeof(melody) / sizeof(melody[0]);

    uart_puts("Buzzer melody\r\n");
    for (int i = 0; i < n_notes; i++) {
        buzzer_freq(melody[i][0]);
        uart_puts("BUZZ ");
        uart_u32(melody[i][0]);
        uart_puts(" Hz\r\n");
        delay_ms(melody[i][1]);
    }
    buzzer_freq(0);
    uart_puts("Buzzer done\r\n");
    for (;;) delay_ms(1000);
}
