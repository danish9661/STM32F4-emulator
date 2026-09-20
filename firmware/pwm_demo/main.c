// PWM Demo — multi-channel LED breathing on TIM2 CH1 (PA0) and TIM3 CH1 (PA6).
// Demonstrates: TIM output compare PWM, GPIO AF, breathing animation via CCR ramp.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)

#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB1ENR (*(volatile uint32_t*)0x40023840)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)

#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

// TIM2 (32-bit)
#define TIM2_CR1    (*(volatile uint32_t*)0x40000000)
#define TIM2_CCER   (*(volatile uint32_t*)0x40000020)
#define TIM2_CCMR1  (*(volatile uint32_t*)0x40000018)
#define TIM2_CCR1   (*(volatile uint32_t*)0x40000034)
#define TIM2_ARR    (*(volatile uint32_t*)0x4000002C)
#define TIM2_PSC    (*(volatile uint32_t*)0x40000028)
#define TIM2_EGR    (*(volatile uint32_t*)0x40000014)

// TIM3 (16-bit)
#define TIM3_CR1    (*(volatile uint32_t*)0x40000400)
#define TIM3_CCER   (*(volatile uint32_t*)0x40000420)
#define TIM3_CCMR1  (*(volatile uint32_t*)0x40000418)
#define TIM3_CCR1   (*(volatile uint32_t*)0x40000434)
#define TIM3_ARR    (*(volatile uint32_t*)0x4000042C)
#define TIM3_PSC    (*(volatile uint32_t*)0x40000428)
#define TIM3_EGR    (*(volatile uint32_t*)0x40000414)

static void uart_putc(char c) {
    while (!(USART1_SR & (1 << 7))) {}
    USART1_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putc(*s++); }
static void uart_put_uint(uint32_t v) {
    char buf[12]; int i = 0;
    if (v == 0) { uart_putc('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i--) uart_putc(buf[i]);
}
static void delay(volatile int n) { while (n--) __asm__("nop"); }

// Triangle wave for breathing: ramp up then down
static uint32_t triangle(uint32_t pos, uint32_t max) {
    if (pos < max) return pos;
    return 2 * max - pos;
}

int main(void) {
    // USART1 PA9/PA10 AF7
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18);
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 4)) | (7 << 4);
    USART1_BRR  = 0x683;
    USART1_CR1  = (1 << 13) | (1 << 3);

    uart_puts("=== PWM Demo ===\r\n");
    uart_puts("TIM2 CH1 (PA0) + TIM3 CH1 (PA6) — breathing LEDs\r\n");

    // PA0 AF1 (TIM2 CH1), PA6 AF2 (TIM3 CH1)
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 0)) | (2 << 0);  // PA0 AF
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 12)) | (2 << 12); // PA6 AF
    GPIOA_AFRL  = (GPIOA_AFRL & ~0xF) | 1;                // PA0 AF1
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 24)) | (2 << 24); // PA6 AF2

    RCC_APB1ENR |= (1 << 0);  // TIM2 clock
    RCC_APB1ENR |= (1 << 1);  // TIM3 clock

    // TIM2: PWM mode 1 on CH1
    TIM2_PSC   = 16 - 1;      // 1 MHz tick
    TIM2_ARR   = 1000 - 1;    // 1 kHz PWM
    TIM2_CCMR1 = (6 << 4) | (1 << 3);  // OC1M = PWM1, OC1PE
    TIM2_CCER  = (1 << 0);    // CC1E
    TIM2_CCR1  = 0;
    TIM2_EGR   = 1;           // UG
    TIM2_CR1   = (1 << 7) | (1 << 0);  // ARPE + CEN

    // TIM3: PWM mode 1 on CH1 (inverted phase for visual variety)
    TIM3_PSC   = 16 - 1;
    TIM3_ARR   = 1000 - 1;
    TIM3_CCMR1 = (6 << 4) | (1 << 3);
    TIM3_CCER  = (1 << 0);
    TIM3_CCR1  = 0;
    TIM3_EGR   = 1;
    TIM3_CR1   = (1 << 7) | (1 << 0);

    uart_puts("breathing (4 full cycles)...\r\n");

    // 4 breathing cycles: 0→1000→0, triangle wave
    uint32_t step = 0;
    uint32_t period = 2000; // steps per full breath
    for (int cycle = 0; cycle < 4; cycle++) {
        for (uint32_t i = 0; i < period; i += 5) {
            uint32_t bright = triangle(i, period / 2);
            TIM2_CCR1 = bright;
            TIM3_CCR1 = period / 2 - bright;  // inverted
            delay(800);  // ~0.2ms per step
        }
        uart_puts("breath ");
        uart_put_uint(cycle + 1);
        uart_puts("/4  peak=");
        uart_put_uint(TIM2_CCR1);
        uart_puts("\r\n");
    }

    // Final: 50% duty on both
    TIM2_CCR1 = 500;
    TIM3_CCR1 = 500;
    uart_puts("=== PWM Demo: done ===\r\n");
    while (1) {}
    return 0;
}
