// MPU trap proof: programs two regions CMSIS-style, then sets CTRL.ENABLE.
// Protection is NOT enforced by the model, so the driver must halt LOUDLY
// (modelHaltInfo names MPU) instead of running on unprotected. Without the
// trap this firmware would print "MPU enabled" and spin forever; the pass
// condition is a stopped emulator with the MPU halt message.
//
// NOTE: startup.c does NOT zero .bss / copy .data — all state is local.
#include <stdint.h>

#define MPU_BASE 0xE000ED90
#define MPU_TYPE (*(volatile unsigned int *)(MPU_BASE + 0x00))
#define MPU_CTRL (*(volatile unsigned int *)(MPU_BASE + 0x04))
#define MPU_RNR  (*(volatile unsigned int *)(MPU_BASE + 0x08))
#define MPU_RBAR (*(volatile unsigned int *)(MPU_BASE + 0x0C))
#define MPU_RASR (*(volatile unsigned int *)(MPU_BASE + 0x10))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    *(volatile unsigned int *)0x40023830 |= (1 << 0);
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

static void uart_x32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        unsigned int n = (v >> (i * 4)) & 0xF;
        uart_putchar(n < 10 ? '0' + n : 'A' + n - 10);
    }
}

int main(void) {
    uart_init();
    uart_puts("=== MPU Test ===\r\n");
    uart_puts("TYPE ");
    uart_x32(MPU_TYPE);
    uart_puts("\r\n");

    // Region 0: FLASH RX (ADDR + VALID + region 0), 1MB full-access.
    MPU_RNR = 0;
    MPU_RBAR = 0x08000010;
    MPU_RASR = 0x03000027;
    // Region 1: SRAM RW, 128KB, XN.
    MPU_RNR = 1;
    MPU_RBAR = 0x20000011;
    MPU_RASR = 0x1300002D;
    // Read back region 0 to prove the registers are live storage.
    MPU_RNR = 0;
    uart_puts("R0BAR ");
    uart_x32(MPU_RBAR);
    uart_puts(" R0ASR ");
    uart_x32(MPU_RASR);
    uart_puts("\r\n");

    // Enable (+ PRIVDEFENA so the fault handler itself stays mapped).
    // The driver must halt HERE with an MPU message — never spin on.
    MPU_CTRL = 0x5;
    uart_puts("MPU enabled\r\n");
    for (volatile unsigned long i = 0; i < 500000; i++);
    uart_puts("MPU SPUN (should never print)\r\n");
    for (;;);
}
