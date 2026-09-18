// GAP10 LTDC: CLUT load + indexed resolve probe.
// Loads CLUT idx0=red / idx1=green on layer 0, reads back the CLUTADD
// pointer (auto-increment) + PFCR-indexed resolve via the render probe
// is asserted host-side; here the guest proves load + pointer + a LUT
// framebuffer scan setup. Prints LTDC GAP10 OK / FAIL.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)
#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define LTDC_BASE  0x40016800
#define LTDC_GCR   (*(volatile uint32_t*)(LTDC_BASE + 0x18))
#define LTDC_L0CR   (*(volatile uint32_t*)(LTDC_BASE + 0x84))
#define LTDC_L0PFCR (*(volatile uint32_t*)(LTDC_BASE + 0x94))
#define LTDC_L0CFBAR (*(volatile uint32_t*)(LTDC_BASE + 0xAC))
#define LTDC_L0CFBLR (*(volatile uint32_t*)(LTDC_BASE + 0xB0))
#define LTDC_L0CFBLNR (*(volatile uint32_t*)(LTDC_BASE + 0xB4))
#define LTDC_L0CLUTWR (*(volatile uint32_t*)(LTDC_BASE + 0xC4))

static void uart_putc(char c) {
    while (!(USART1_SR & (1 << 7))) {}
    USART1_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18);
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 4))  | (7 << 4);
    USART1_BRR  = 0x683;
    USART1_CR1  = (1 << 13) | (1 << 3);
    uart_puts("=== LTDC GAP10 ===\r\n");

    LTDC_L0CLUTWR = (0 << 24) | 0xFF0000; // idx0 = red
    LTDC_L0CLUTWR = (1 << 24) | 0x00FF00; // idx1 = green
    uint32_t ptr = (LTDC_L0CLUTWR >> 24) & 0xFF;
    LTDC_L0PFCR = 5; // L8 indexed
    LTDC_L0CFBAR = 0x20002000;
    LTDC_L0CFBLR = (64 << 16) | 64;
    LTDC_L0CFBLNR = 32;
    LTDC_L0CR = 1; // layer enable
    LTDC_GCR = 1;  // LTDCEN
    if (ptr == 2) {
        uart_puts("LTDC GAP10 OK\r\n");
    } else {
        uart_puts("LTDC GAP10 FAIL (CLUTADD)\r\n");
    }
    while (1);
}
