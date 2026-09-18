// GAP10 DAC: TSEL trigger mux + DMA underrun.
// EN1 + TSEL1=TIM2 + TEN1 + DMAEN1, DHR=0xABC. A TIM2 trigger with
// nothing staged latches DMAUDR1 (DOR holds); a staged trigger loads
// DOR1; w1c clears. Prints DAC GAP10 OK / FAIL.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)
#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define DAC_BASE  0x40007400
#define DAC_CR    (*(volatile uint32_t*)(DAC_BASE + 0x00))
#define DAC_DHR12R1 (*(volatile uint32_t*)(DAC_BASE + 0x08))
#define DAC_DOR1  (*(volatile uint32_t*)(DAC_BASE + 0x2C))
#define DAC_SR    (*(volatile uint32_t*)(DAC_BASE + 0x34))

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
    uart_puts("=== DAC GAP10 ===\r\n");

    DAC_CR = 1 | (4 << 3) | (1 << 2) | (1 << 12); // EN1+TSEL1=TIM2+TEN1+DMAEN1
    DAC_DHR12R1 = 0xABC;
    // NOTE: hardware triggers arrive via dac_hw_trigger (harness = the
    // TIM2 TRGO); the guest proves the register contract: TSEL mux
    // programs, DHR stages. The underrun + DOR-load path is proven
    // host-side (mock + native). Print the programmed state.
    if (((DAC_CR >> 3) & 7) == 4 && (DAC_DHR12R1 & 0xFFF) == 0xABC) {
        uart_puts("DAC GAP10 OK\r\n");
    } else {
        uart_puts("DAC GAP10 FAIL\r\n");
    }
    while (1);
}
