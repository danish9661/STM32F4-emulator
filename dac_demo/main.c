// DAC Demo — writes a sine wave directly to DAC1 CH1 (PA4).
// Demonstrates: DAC register writes, sine lookup table, USART1 TX.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)

#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB1ENR (*(volatile uint32_t*)0x40023840)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)

#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)

#define DAC_CR      (*(volatile uint32_t*)0x40007404)
#define DAC_DHR12R1 (*(volatile uint32_t*)0x40007408)
#define DAC_DHR12L1 (*(volatile uint32_t*)0x4000740C)
#define DAC_DOR1    (*(volatile uint32_t*)0x40007448)

// 64-point sine table (0-4095)
static const uint16_t sine_table[64] = {
    2048, 2249, 2447, 2642, 2831, 3013, 3187, 3351,
    3504, 3645, 3773, 3886, 3985, 4067, 4133, 4180,
    4209, 4219, 4211, 4184, 4139, 4076, 3996, 3901,
    3791, 3669, 3535, 3392, 3241, 3084, 2924, 2763,
    2603, 2447, 2297, 2156, 2026, 1910, 1810, 1727,
    1663, 1621, 1603, 1609, 1640, 1694, 1771, 1869,
    1986, 2120, 2268, 2426, 2591, 2759, 2926, 3089,
    3244, 3388, 3519, 3634, 3731, 3808, 3864, 3897
};

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

int main(void) {
    // USART1 PA9/PA10 AF7
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18);
    volatile uint32_t *af = (volatile uint32_t*)0x40020020;
    *af = (*af & ~(0xF << 4)) | (7 << 4);
    USART1_BRR = 0x683;
    USART1_CR1 = (1 << 13) | (1 << 3);

    uart_puts("=== DAC Demo ===\r\n");
    uart_puts("Sine wave on DAC1 CH1 (PA4) — 64 samples, 2 cycles\r\n");

    // PA4 analog mode (MODER = 0b11)
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 8)) | (3 << 8);

    // DAC1 channel 1 enable (no trigger — software mode)
    RCC_APB1ENR |= (1 << 29);  // DAC clock
    DAC_CR = (1 << 0);          // EN1 (enable DAC, no trigger = software)
    DAC_DHR12R1 = 2048;         // initial mid-value

    // Output 2 complete sine cycles
    for (int cycle = 0; cycle < 2; cycle++) {
        for (int i = 0; i < 64; i++) {
            DAC_DHR12R1 = sine_table[i];
            // Read back to verify
            uint32_t readback = DAC_DOR1;
            if (i == 0 || i == 32) {
                uart_puts("cycle ");
                uart_put_uint(cycle + 1);
                uart_puts(" sample ");
                uart_put_uint(i);
                uart_puts(": wrote ");
                uart_put_uint(sine_table[i]);
                uart_puts(" read=");
                uart_put_uint(readback);
                uart_puts("\r\n");
            }
            delay(2000);
        }
        uart_puts("cycle ");
        uart_put_uint(cycle + 1);
        uart_puts("/2 done\r\n");
    }

    // Verify final value
    uart_puts("final DOR1=");
    uart_put_uint(DAC_DOR1);
    uart_puts("\r\n");
    uart_puts("=== DAC Demo: done ===\r\n");
    while (1) {}
    return 0;
}
