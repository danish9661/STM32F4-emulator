// ADC Demo — reads ADC1 channel 0 (PA0) and prints voltage via UART1.
// Demonstrates: ADC single conversion, USART1 TX, GPIO AF setup.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)

#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)

#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define ADC1_SR     (*(volatile uint32_t*)0x40012000)
#define ADC1_CR1    (*(volatile uint32_t*)0x40012004)
#define ADC1_CR2    (*(volatile uint32_t*)0x40012008)
#define ADC1_SQR3   (*(volatile uint32_t*)0x40012034)
#define ADC1_DR     (*(volatile uint32_t*)0x4001204C)
#define ADC1_SMPR2  (*(volatile uint32_t*)0x40012010)

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

static void uart_put_hex(uint32_t v) {
    const char *hex = "0123456789ABCDEF";
    uart_puts("0x");
    for (int i = 28; i >= 0; i -= 4) uart_putc(hex[(v >> i) & 0xF]);
}

static void delay(volatile int n) { while (n--) __asm__("nop"); }

int main(void) {
    // USART1 PA9/PA10 AF7
    RCC_AHB1ENR |= (1 << 0);   // GPIOA clock
    RCC_APB2ENR |= (1 << 4);   // USART1 clock
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18); // PA9 AF
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 4))  | (7 << 4); // AF7
    USART1_BRR  = 0x683;        // 115200 @ 16 MHz
    USART1_CR1  = (1 << 13) | (1 << 3); // UE + TE

    uart_puts("=== ADC Demo ===\r\n");
    uart_puts("Reading ADC1 CH0 (PA0) - 10 samples\r\n");

    // ADC1 power-on sequence: ADON 0->1
    RCC_APB2ENR |= (1 << 8);   // ADC1 clock
    ADC1_CR2    = 0;            // ensure ADON=0
    ADC1_SMPR2  = 0;            // fastest sample time (1.5 cycles)
    ADC1_CR1    = 0;
    ADC1_SQR3   = 0;            // channel 0

    // Enable ADC (ADON write)
    ADC1_CR2 = 1;               // ADON=1, power on

    for (int i = 0; i < 10; i++) {
        // Trigger single conversion: write SWSTART=1 (0->1 edge recorded by model)
        ADC1_CR2 = (1 << 30) | (1 << 2) | 1;  // SWSTART + ALIGN right + ADON
        delay(200);  // wait for conversion to complete (model needs ~15 cycles)
        // Read CR2 to trigger model's start_conversion() check
        (void)ADC1_CR2;
        // Read DR (reading DR also clears EOC in SR)
        uint32_t raw = ADC1_DR;
        uint32_t mv = (raw * 3300) / 4095;

        uart_puts("sample ");
        uart_put_uint(i);
        uart_puts(": raw=");
        uart_put_uint(raw);
        uart_puts(" (");
        uart_put_hex(raw);
        uart_puts(")  voltage=");
        uart_put_uint(mv / 1000);
        uart_putc('.');
        uart_put_uint(mv % 1000);
        uart_puts(" V\r\n");

        delay(1000);
    }

    uart_puts("=== ADC Demo: done ===\r\n");
    while (1) {}
    return 0;
}
