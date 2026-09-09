// FPU test: VFPv4-SP via real GCC hard-float output.
// The core must execute vmov/vadd/vsub/vmul/vdiv/vsqrt/vfma/vcmp/vcvt
// exactly like silicon (see AGENTS.md §25). All results are printed as raw
// hex bits (no libc float formatting in this freestanding build) and
// checked against host-computed IEEE-754 expectations.
//
static float fsqrt(float x) {
    float r;
    __asm__ volatile ("vsqrt.f32 %0, %1" : "=t"(r) : "t"(x));
    return r;
}

// NOTE: startup.c does NOT zero .bss / copy .data (blinky convention), so
// every variable here is a stack local assigned before use. Only `float`
// (never `double`: -Wdouble-promotion -Werror enforces it) so no soft-float
// helpers are needed; __builtin_sqrtf/__builtin_fmaf lower to vsqrt/vfma.
#include <stdint.h>

#define SCB_CPACR (*(volatile unsigned int *)0xE000ED88)

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    *(volatile unsigned int *)0x40023830 |= (1 << 0);  // GPIOA clock
    *(volatile unsigned int *)0x40023844 |= (1 << 4);  // USART1 clock
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

static unsigned int fbits(float f) {
    union { float f; unsigned int u; } u;
    u.f = f;
    return u.u;
}

static int fails = 0;

static void check(const char *name, unsigned int got, unsigned int want) {
    uart_puts(name);
    uart_putchar(' ');
    uart_x32(got);
    if (got == want) {
        uart_puts(" PASS\r\n");
    } else {
        uart_puts(" want ");
        uart_x32(want);
        uart_puts(" FAIL\r\n");
        fails++;
    }
}

int main(void) {
    uart_init();
    uart_puts("=== FPU Test ===\r\n");

    // The model faults (UsageFault NOCP) on any VFP insn until CP10+CP11
    // are fully enabled — the gate itself is under test here.
    SCB_CPACR = 0x00F00000;
    if ((SCB_CPACR & 0x00F00000) != 0x00F00000) {
        uart_puts("CPACR FAIL\r\n");
        fails++;
    } else {
        uart_puts("CPACR ok\r\n");
    }

    volatile float a15 = 1.5f, b225 = 2.25f;
    volatile float a20 = 2.0f, b30 = 3.0f, c10 = 10.0f;
    volatile float seven = 7.0f, five = 5.0f, one = 1.0f;

    check("ADD", fbits(a15 + b225), 0x40700000); // 3.75
    check("SUB", fbits(a15 - five), 0xC0600000); // -3.5
    check("MUL", fbits(a20 * b225), 0x40900000); // 4.5
    check("DIV", fbits(seven / a20), 0x40600000); // 3.5
    check("SQRT", fbits(fsqrt(a20)), 0x3FB504F3); // sqrt(2)

    // Fused check: a=b=1+2^-23, acc=-(1+2^-22). Unfused rounds a*b to
    // 1+2^-22 first and yields 0; fused keeps eps^2 = 2^-46 (0x28800000).
    volatile float e = 0x1P-23F;
    volatile float f1pe = one + e;
    volatile float acc = -(one + 2.0f * e);
    check("FMA", fbits(__builtin_fmaf(f1pe, f1pe, acc)), 0x28800000);
    uart_puts("FMA-PLAIN ");
    uart_x32(fbits(f1pe * f1pe + acc));
    uart_puts("\r\n");

    // Comparisons (vcmp + vmrs flag import, done by the compiler).
    if (!(one < a20) || !(a20 > one) || !(one == one)) {
        uart_puts("CMP FAIL\r\n");
        fails++;
    } else {
        volatile float zero = 0.0f;
        volatile float nanv = zero / zero; // runtime NaN, no trap (flags masked)
        if ((nanv < one) || (nanv == one)) {
            uart_puts("CMP-NAN FAIL\r\n");
            fails++;
        } else {
            uart_puts("CMP OK\r\n");
        }
    }

    // Conversions (vcvt, RNE): ties go to even (2.5 -> 2, not 3).
    volatile float f25 = 2.5f, f19 = 1.9f;
    volatile float f42 = 42.0f, fn15 = -1.5f;
    if ((int)f25 != 2 || (int)a15 != 2 || (int)fn15 != -2 || (int)f19 != 2) {
        uart_puts("CVT-I FAIL\r\n");
        fails++;
    } else if (fbits((float)42) != 0x42280000 || fbits(f42) != 0x42280000) {
        uart_puts("CVT-F FAIL\r\n");
        fails++;
    } else {
        uart_puts("CVT OK\r\n");
    }

    if (fails == 0) uart_puts("FPU all PASS\r\n");
    uart_puts("FPU done\r\n");
    for (;;);
}
