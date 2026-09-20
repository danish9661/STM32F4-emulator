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

// Multi-register + parallel-DSP execution coverage (inline asm so the
// exact forms execute; GAS verifies every encoding at build time).
// noinline + core-reg-only operands keep the compiler's allocator out of
// the picture (S-regs are caller-saved, so no clobber declarations needed
// for the S-file traffic inside).
__attribute__((noinline)) static void multi_S(float *buf, unsigned *o) {
    __asm__ volatile (
        "vmov.f32 s4, #1.0\n\t"
        "vmov.f32 s5, #2.0\n\t"
        "vmov.f32 s6, #4.0\n\t"
        "vmov.f32 s7, #8.0\n\t"
        "vstmia %4, {s4-s7}\n\t"
        "vldmia %4, {s8-s11}\n\t"
        "vmov %0, s8\n\t"
        "vmov %1, s9\n\t"
        "vmov %2, s10\n\t"
        "vmov %3, s11\n\t"
        : "=&r" (o[0]), "=&r" (o[1]), "=&r" (o[2]), "=&r" (o[3])
        : "r" (buf)
        : "memory");
}

__attribute__((noinline)) static void dm_single(float *buf, unsigned *o) {
    __asm__ volatile (
        "vmov.f32 s2, #1.0\n\t"
        "vmov.f32 s3, #2.0\n\t"
        "vstr d1, [%2]\n\t"
        "vmov s2, %3\n\t"
        "vmov s3, %3\n\t"
        "vldr d1, [%2]\n\t"
        "vmov %0, s2\n\t"
        "vmov %1, s3\n\t"
        : "=&r" (o[0]), "=&r" (o[1])
        : "r" (buf), "r" (0)
        : "memory");
}

__attribute__((noinline)) static void dsp_ops(unsigned *o) {
    __asm__ volatile (
        "qadd8 %0, %4, %5\n\t"   // 127+1 saturates per lane -> 0x7F7F7F7F
        "shadd16 %1, %6, %7\n\t" // (2+2)/2 per lane -> 0x00020002
        "smlad %2, %8, %9, %10\n\t" // 1*3+2*4+16 = 27
        "usada8 %3, %11, %12, %13\n\t" // 4x|1-2|+16 = 20
        : "=&r" (o[0]), "=&r" (o[1]), "=&r" (o[2]), "=&r" (o[3])
        : "r" (0x7F7F7F7F), "r" (0x01010101),
          "r" (0x00020002), "r" (0x00020002),
          "r" (0x00010002), "r" (0x00030004), "r" (16),
          "r" (0x01010101), "r" (0x02020202), "r" (16)
        :);
}

static unsigned int fbits(float f) {
    union { float f; unsigned int u; } u;
    u.f = f;
    return u.u;
}

// Many live floats across a call: forces the compiler to spill callee-
// saved D-regs via vpush/vldm (the multi-register path — check the
// disassembly for vpush.64/vldm sp!). Plain (non-volatile) locals so they
// actually live in S-regs; the array is volatile so nothing folds.
static volatile float Vvals[20];
__attribute__((noinline)) static float spill_helper(float x) { return x * 1.5f; }
__attribute__((noinline)) static float spill_call(float *p) {
    float a0=p[0],a1=p[1],a2=p[2],a3=p[3],a4=p[4],a5=p[5],a6=p[6],a7=p[7];
    float a8=p[8],a9=p[9],a10=p[10],a11=p[11],a12=p[12],a13=p[13],a14=p[14],a15=p[15];
    float a16=p[16],a17=p[17],a18=p[18],a19=p[19];
    float t = spill_helper(a0);
    return t+a1+a2+a3+a4+a5+a6+a7+a8+a9+a10+a11+a12+a13+a14+a15+a16+a17+a18+a19;
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

    // Multi-register spill across a call (vpush.64/vldm in the disasm):
    // 1..20 with a0 replaced by 1.5x => 210.5 (0x43528000).
    for (int i = 0; i < 20; i++) Vvals[i] = (float)(i + 1);
    check("SPILL", fbits(spill_call((float *)Vvals)), 0x43528000);

    // S-list multi-transfer round-trip (vstmia/vldmia s-forms).
    {
        static float mbuf[8];
        unsigned o[5];
        multi_S(mbuf, o);
        if (o[0] != 0x3F800000 || o[1] != 0x40000000 || o[2] != 0x40800000 || o[3] != 0x41000000) {
            uart_puts("VLDM ");
            uart_x32(o[0]); uart_putchar(' ');
            uart_x32(o[3]); uart_puts(" FAIL\r\n");
            fails++;
        } else {
            uart_puts("VLDM OK\r\n");
        }
    }

    // D-single VLDR/VSTR round-trip.
    {
        static float dbuf[2];
        unsigned o[2];
        dm_single(dbuf, o);
        if (o[0] != 0x3F800000 || o[1] != 0x40000000) {
            uart_puts("VLDR-D FAIL\r\n");
            fails++;
        } else {
            uart_puts("VLDR-D OK\r\n");
        }
    }

    // Parallel DSP (saturate / halve / dual-mul-acc / byte-acc + Q flag).
    {
        unsigned o[5];
        dsp_ops(o);
        unsigned apsr = 0;
        __asm__ volatile ("mrs %0, apsr" : "=r"(apsr));
        if (o[0] != 0x7F7F7F7F || o[1] != 0x00020002 || o[2] != 27 || o[3] != 20) {
            uart_puts("DSP ");
            uart_x32(o[0]); uart_putchar(' ');
            uart_x32(o[1]); uart_putchar(' ');
            uart_x32(o[2]); uart_putchar(' ');
            uart_x32(o[3]); uart_puts(" FAIL\r\n");
            fails++;
        } else if (((apsr >> 27) & 1) != 1) {
            uart_puts("DSP-Q FAIL\r\n");
            fails++;
        } else {
            uart_puts("DSP OK\r\n");
        }
    }

    if (fails == 0) uart_puts("FPU all PASS\r\n");
    uart_puts("FPU done\r\n");
    for (;;);
}
