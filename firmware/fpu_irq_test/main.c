// FPU-under-interrupt proof: lazy FP stacking across real SysTick IRQs.
//
// Main seeds S0-S3 with magic values (via vmov, so CONTROL.FPCA sets) and
// FPSCR=0, then spins while SysTick fires every 2000 instructions. The
// handler does its own float work on other S-regs and dirties FPSCR
// (0.0/0.0 -> IOC). At the end main reads everything back: S0-S3 must be
// intact and FPSCR clean — only possible if exception entry reserved the
// FP frame and the first handler FPU use stacked the live regs (lazy),
// with a full restore on return. Without lazy stacking the handler's
// float traffic stomps S0-S3 and this prints FAIL.
//
// NOTE: startup.c does NOT zero .bss / copy .data, so irq_count/hsink are
// assigned in main before use. Only `float` (never `double`).
#include <stdint.h>

#define SCB_CPACR (*(volatile unsigned int *)0xE000ED88)
#define FPCCR     (*(volatile unsigned int *)0xE000EF34)
#define FPCAR     (*(volatile unsigned int *)0xE000EF38)

#define SYST_CSR (*(volatile unsigned int *)0xE000E010)
#define SYST_RVR (*(volatile unsigned int *)0xE000E014)
#define SYST_CVR (*(volatile unsigned int *)0xE000E018)

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

static void uart_u32(unsigned int v) {
    char buf[12];
    int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

static unsigned int read_sreg(int n) {
    unsigned int v = 0;
    switch (n) {
    case 0: __asm__ volatile ("vmov %0, s0" : "=r"(v)); break;
    case 1: __asm__ volatile ("vmov %0, s1" : "=r"(v)); break;
    case 2: __asm__ volatile ("vmov %0, s2" : "=r"(v)); break;
    default: __asm__ volatile ("vmov %0, s3" : "=r"(v)); break;
    }
    return v;
}

static void write_sreg(int n, unsigned int v) {
    switch (n) {
    case 0: __asm__ volatile ("vmov s0, %0" :: "r"(v)); break;
    case 1: __asm__ volatile ("vmov s1, %0" :: "r"(v)); break;
    case 2: __asm__ volatile ("vmov s2, %0" :: "r"(v)); break;
    default: __asm__ volatile ("vmov s3, %0" :: "r"(v)); break;
    }
}

static unsigned int read_fpscr(void) {
    unsigned int v = 0;
    __asm__ volatile ("vmrs %0, fpscr" : "=r"(v));
    return v;
}

static void write_fpscr(unsigned int v) {
    __asm__ volatile ("vmsr fpscr, %0" :: "r"(v));
}

static volatile unsigned int irq_count;
static volatile float hsink;

void SysTick_Handler(void) {
    irq_count++;
    // The handler's own float traffic (compiler-emitted VFP on S-regs).
    volatile float x = 1.5f;
    volatile float y = 2.5f;
    hsink = x * y + 0.5f;
    // Dirty the cumulative flags: 0.0/0.0 -> NaN + IOC. Main's FPSCR must
    // come back clean (restored from the lazy frame on return).
    volatile float z = 0.0f;
    volatile float w = z / z;
    hsink += w;
}

int main(void) {
    uart_init();
    uart_puts("=== FPU IRQ Test ===\r\n");

    SCB_CPACR = 0x00F00000;
    if ((SCB_CPACR & 0x00F00000) != 0x00F00000) {
        uart_puts("CPACR FAIL\r\n");
        goto done;
    }
    uart_puts("CPACR ok\r\n");

    // Seed thread FP state (also sets CONTROL.FPCA via FPU use).
    write_sreg(0, 0x11111111);
    write_sreg(1, 0x22222222);
    write_sreg(2, 0x33333333);
    write_sreg(3, 0x44444444);
    write_fpscr(0);

    irq_count = 0;
    hsink = 0.0f;
    SYST_RVR = 2000;
    SYST_CVR = 0;
    SYST_CSR = 0x7; // ENABLE | TICKINT | CLKSOURCE

    for (volatile unsigned long i = 0; i < 300000; i++);

    SYST_CSR = 0; // stop the tick before verifying
    uart_puts("IRQ count ");
    uart_u32(irq_count);
    uart_puts("\r\n");
    uart_puts("FPCCR ");
    uart_x32(FPCCR);
    uart_puts(" FPCAR ");
    uart_x32(FPCAR);
    uart_puts("\r\n");

    int fails = 0;
    if (irq_count < 10) { uart_puts("IRQ-COUNT FAIL\r\n"); fails++; }
    unsigned int want[4] = { 0x11111111, 0x22222222, 0x33333333, 0x44444444 };
    for (int i = 0; i < 4; i++) {
        unsigned int got = read_sreg(i);
        uart_puts("S");
        uart_putchar('0' + i);
        uart_putchar(' ');
        uart_x32(got);
        if (got != want[i]) {
            uart_puts(" FAIL\r\n");
            fails++;
        } else {
            uart_puts(" ok\r\n");
        }
    }
    unsigned int fpscr = read_fpscr();
    uart_puts("FPSCR ");
    uart_x32(fpscr);
    if (fpscr != 0) {
        uart_puts(" FAIL\r\n");
        fails++;
    } else {
        uart_puts(" ok\r\n");
    }
    if (fails == 0) uart_puts("FPU IRQ all PASS\r\n");
done:
    uart_puts("FPU IRQ done\r\n");
    for (;;);
}
