// audio_play_test: continuous 440 Hz-ish tone via I2S1 TX (SPI1 block,
// 0x40013000) using a 256-sample sine table. Each DR write pushes a
// sample into the model's capture FIFO; the JS layer (site/emulator.js
// speaker device) drains it and plays it with WebAudio.

#include "sine_table.h"

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOB_BASE  0x40020400
#define GPIOB_MODER (*(volatile unsigned int *)(GPIOB_BASE + 0x00))
#define GPIOB_AFRL  (*(volatile unsigned int *)(GPIOB_BASE + 0x20))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define SPI1_BASE   0x40013000
#define SPI1_CR1    (*(volatile unsigned int *)(SPI1_BASE + 0x00))
#define SPI1_I2SCFGR (*(volatile unsigned int *)(SPI1_BASE + 0x1C))
#define SPI1_I2SPR  (*(volatile unsigned int *)(SPI1_BASE + 0x20))
#define SPI1_SR     (*(volatile unsigned int *)(SPI1_BASE + 0x08))
#define SPI1_DR     (*(volatile unsigned int *)(SPI1_BASE + 0x0C))

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
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

static void i2s_init(void) {
    RCC_AHB1ENR |= (1 << 0);             // GPIOA
    RCC_APB2ENR |= (1 << 12);            // SPI1
    // I2S1 pins: PB12 WS? use PB9 SDA? Standard I2S1: PB12 CK, PB13 SD? no —
    // this demo only drives the DR register; pins are cosmetic here.
    GPIOB_MODER = (GPIOB_MODER & ~(3u << 18)) | (2u << 18);  // PB9 AF (cosmetic)
    GPIOB_AFRL  = (GPIOB_AFRL & ~0xF0000) | (5 << 18);

    SPI1_CR1 = 0;
    SPI1_I2SPR = (2 << 0) | (1 << 8);    // prescaler 8, master clock div
    SPI1_I2SCFGR = (1 << 11) | (1 << 10) | (1 << 9) | (1 << 0);
    // I2SMOD=1 (bit 11), I2SE=1 (bit 10), I2SCFG=10 master TX (bits 9:8)
}

static void i2s_write_sample(unsigned short s) {
    while (!(SPI1_SR & (1 << 1)));       // TXE
    SPI1_DR = s;
}

int main(void) {
    uart_init();
    uart_puts("Audio play test\r\n");
    uart_puts("I2S1 TX sine 256 samples\r\n");
    i2s_init();
    uart_puts("I2S ready\r\n");

    unsigned int idx = 0;
    unsigned int tick = 0;
    for (;;) {
        for (int i = 0; i < 256; i++) {
            i2s_write_sample(sine256[(i + idx) & 0xFF]);
        }
        idx = (idx + 1) & 0xFF;
        tick++;
        if ((tick & 0xFF) == 0) {
            uart_puts("TICK ");
            uart_u32(tick);
            uart_puts("\r\n");
        }
    }
}
