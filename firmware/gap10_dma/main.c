// GAP10 DMA: FCR threshold + DBM current-target.
// Sets FCR FTH=11 (full) on DMA2 stream 0 and reads it back, enables DBM
// (with FIFO) and reads CT (M0 target), proving the FCR/DBM contract.
// Prints DMA GAP10 OK / FAIL.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)
#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define DMA2_BASE 0x40026400
#define DMA2_S0CR  (*(volatile uint32_t*)(DMA2_BASE + 0x10))
#define DMA2_S0NDTR (*(volatile uint32_t*)(DMA2_BASE + 0x14))
#define DMA2_S0M0AR (*(volatile uint32_t*)(DMA2_BASE + 0x1C))
#define DMA2_S0M1AR (*(volatile uint32_t*)(DMA2_BASE + 0x20))
#define DMA2_S0FCR (*(volatile uint32_t*)(DMA2_BASE + 0x24))

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
    uart_puts("=== DMA GAP10 ===\r\n");

    DMA2_S0FCR = (1 << 2) | (3 << 1); // DMDIS + FTH=full (4 words)
    int ok = ((DMA2_S0FCR >> 1) & 3) == 3;
    DMA2_S0M0AR = 0x20000000;
    DMA2_S0M1AR = 0x20001000;
    DMA2_S0NDTR = 16;
    DMA2_S0CR = (1 << 18) | (1 << 2) | 1; // DBM + DMDIS + EN
    ok = ok && ((DMA2_S0CR & (1 << 19)) == 0); // CT reads M0 after EN
    if (ok) uart_puts("DMA GAP10 OK\r\n");
    else    uart_puts("DMA GAP10 FAIL\r\n");
    while (1);
}
