// Differential micro-fuzz firmware: executes fixed-point/DSP/shift op
// vectors with VOLATILE operands (so gcc cannot constant-fold them) and
// prints result + APSR hex. The same binary runs on the wasm CPU backend
// and the Unicorn backend; outputs must be byte-identical.
// NOTE: startup.c does NOT zero .bss — all state below is static-initialized.
#include <stdint.h>

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    *(volatile unsigned int *)0x40023830 |= (1 << 0);
    *(volatile unsigned int *)0x40023844 |= (1 << 4);
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}
static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putchar(*s++); }
static void uart_hex8(uint32_t v) {
    for (int i = 7; i >= 0; i--) uart_putchar("0123456789ABCDEF"[(v >> (i * 4)) & 0xF]);
}

// Volatile operand sources (defeat constant folding).
static volatile int32_t VA;
static volatile int32_t VB;
static volatile int32_t VC;
static volatile uint32_t VSH;

static uint32_t read_apsr(void) {
    uint32_t r;
    __asm__ volatile ("mrs %0, apsr" : "=r" (r));
    return r;
}
// Clear NZCVQ (keep GE): prevents Q-stickiness from cascading across vectors.
static void clear_flags(void) {
    uint32_t z = 0;
    __asm__ volatile ("msr apsr_nzcvq, %0" :: "r" (z));
}

// Deterministic LCG for the FixedMul/FixedDiv sweep.
static uint32_t lcg_state = 0x12345678;
static uint32_t lcg_next(void) {
    lcg_state = lcg_state * 1664525u + 1013904223u;
    return lcg_state;
}

static void show(const char *name, uint32_t extra) {
    uart_puts(name);
    uart_putchar(' ');
    uart_hex8(extra);
    uart_putchar(' ');
    uart_hex8(read_apsr());
    uart_putchar('\n');
}

#define V2(op, body) do { clear_flags(); { body } } while (0)

int main(void) {
    uart_init();
    uart_puts("=== FUZZ ===\n");
    int32_t a, b, c, r, hi, lo;
    uint32_t apsr_dummy;
    (void)apsr_dummy;

    // ---- MSR/MRS round-trip probe (isolates flag-path handling) ----
    {
        uint32_t back;
        __asm__ volatile (
            "movs r0, #0\n"
            "msr apsr_nzcvq, r0\n"
            "mrs %0, apsr\n"
            : "=r" (back) :: "r0", "cc");
        uart_puts("MSRCLR "); uart_hex8(back); uart_putchar('\n');
        __asm__ volatile (
            "movs r0, #0\n"
            "adds r0, #1\n"      // N=0,Z=0,C=0,V=0 expected
            "mrs %0, apsr\n"
            : "=r" (back) :: "r0", "cc");
        uart_puts("ADDS01 "); uart_hex8(back); uart_putchar('\n');
        __asm__ volatile (
            "movs r0, #0\n"
            "subs r0, #1\n"      // N=1,Z=0,C=0,V=0 expected
            "mrs %0, apsr\n"
            : "=r" (back) :: "r0", "cc");
        uart_puts("SUBS01 "); uart_hex8(back); uart_putchar('\n');
    }

    // ---- SDIV / UDIV ----
    {
        static const int32_t AV[] = {7, -7, 7, -7, 0, 0x7FFFFFFF, (int32_t)0x80000000, 1000000};
        static const int32_t BV[] = {-3, 3, 3, -3, 5, 1, -1, -777};
        for (int i = 0; i < 8; i++) {
            VA = AV[i]; VB = BV[i];
            a = VA; b = VB;
            __asm__ volatile ("sdiv %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SDIV "); uart_hex8((uint32_t)a); uart_putchar(' ');
            uart_hex8((uint32_t)b); uart_putchar(' ');
            uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
        }
        static const uint32_t UV[] = {7, 0xFFFFFFFFu, 0x80000000u, 1000000u};
        static const uint32_t UW[] = {3, 3, 2, 777u};
        for (int i = 0; i < 4; i++) {
            uint32_t ua = UV[i], ub = UW[i], ur;
            __asm__ volatile ("udiv %0, %1, %2" : "=r" (ur) : "r" (ua), "r" (ub));
            uart_puts("UDIV "); uart_hex8(ua); uart_putchar(' ');
            uart_hex8(ub); uart_putchar(' ');
            uart_hex8(ur); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
        }
    }

    // ---- SMULL / UMULL / SMLAL / MLA / MLS ----
    {
        static const int32_t AV[] = {0x12345678, -1, (int32_t)0x80000000, 0x7FFFFFFF, 0x00010001};
        static const int32_t BV[] = {0x11111111, -1, (int32_t)0x80000000, 0x7FFFFFFF, 0xFFFF0001};
        for (int i = 0; i < 5; i++) {
            VA = AV[i]; VB = BV[i]; VC = 0x01020304;
            a = VA; b = VB; c = VC;
            __asm__ volatile ("smull %0, %1, %2, %3" : "=r" (lo), "=r" (hi) : "r" (a), "r" (b));
            uart_puts("SMULL "); uart_hex8((uint32_t)a); uart_putchar(' ');
            uart_hex8((uint32_t)b); uart_putchar(' ');
            uart_hex8((uint32_t)lo); uart_putchar(' ');
            uart_hex8((uint32_t)hi); uart_putchar('\n');
            __asm__ volatile ("umull %0, %1, %2, %3" : "=r" (lo), "=r" (hi) : "r" (a), "r" (b));
            uart_puts("UMULL "); uart_hex8((uint32_t)a); uart_putchar(' ');
            uart_hex8((uint32_t)b); uart_putchar(' ');
            uart_hex8((uint32_t)lo); uart_putchar(' ');
            uart_hex8((uint32_t)hi); uart_putchar('\n');
            __asm__ volatile ("smlal %0, %1, %2, %3" : "+r" (lo), "+r" (hi) : "r" (a), "r" (b));
            uart_puts("SMLAL "); uart_hex8((uint32_t)lo); uart_putchar(' ');
            uart_hex8((uint32_t)hi); uart_putchar('\n');
            __asm__ volatile ("mla %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (c));
            uart_puts("MLA "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("mls %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (c));
            uart_puts("MLS "); uart_hex8((uint32_t)r); uart_putchar('\n');
        }
    }

    // ---- SMULxy / SMLAxy / SMULW / SMLAD / SMLSD ----
    {
        static const int32_t AV[] = {(int32_t)0x80008000, 0x7FFF7FFF, 0x1234FEDC, (int32_t)0xFFFF0001};
        static const int32_t BV[] = {(int32_t)0x80008000, 0x7FFF7FFF, (int32_t)0xEDCBA987, 0x0002FFFE};
        for (int i = 0; i < 4; i++) {
            VA = AV[i]; VB = BV[i]; VC = 0x11112222;
            a = VA; b = VB; c = VC;
            __asm__ volatile ("smulbb %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULBB "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smulbt %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULBT "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smultb %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULTB "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smultt %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULTT "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smlabb %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (c));
            uart_puts("SMLABB "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smulwt %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULWT "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smulwb %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMULWB "); uart_hex8((uint32_t)r); uart_putchar('\n');
            clear_flags();
            __asm__ volatile ("smlad %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (c));
            uart_puts("SMLAD "); uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
            clear_flags();
            __asm__ volatile ("smlsd %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (c));
            uart_puts("SMLSD "); uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
        }
    }

    // ---- SSAT / USAT / PKHBT / PKHTB / QADD / QSUB / QDADD / QDSUB ----
    {
        static const int32_t AV[] = {0, 127, 128, -129, 0x7FFFFFFF, (int32_t)0x80000000, 0x12345678};
        for (int i = 0; i < 7; i++) {
            VA = AV[i]; a = VA;
            __asm__ volatile ("ssat %0, #8, %1" : "=r" (r) : "r" (a));
            uart_puts("SSAT8 "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("ssat %0, #16, %1" : "=r" (r) : "r" (a));
            uart_puts("SSAT16 "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("usat %0, #8, %1" : "=r" (r) : "r" (a));
            uart_puts("USAT8 "); uart_hex8((uint32_t)r); uart_putchar('\n');
            clear_flags();
            __asm__ volatile ("ssat %0, #8, %1" : "=r" (r) : "r" (a));
            uart_puts("SSAT8Q "); uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
        }
        {
            VA = 0x11223344; VB = 0xAABBCCDD;
            a = VA; b = VB;
            __asm__ volatile ("pkhbt %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("PKHBT "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("pkhtb %0, %1, %2, asr #7" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("PKHTB "); uart_hex8((uint32_t)r); uart_putchar('\n');
        }
        {
            static const int32_t QV[] = {0x7FFFFFFF, 0x7FFFFFFF, (int32_t)0x80000000, 100, -100};
            static const int32_t QW[] = {1, 0x7FFFFFFF, -1, 200, -200};
            for (int i = 0; i < 5; i++) {
                VA = QV[i]; VB = QW[i]; a = VA; b = VB;
                clear_flags();
                __asm__ volatile ("qadd %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
                uart_puts("QADD "); uart_hex8((uint32_t)r); uart_putchar(' ');
                uart_hex8(read_apsr()); uart_putchar('\n');
                clear_flags();
                __asm__ volatile ("qsub %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
                uart_puts("QSUB "); uart_hex8((uint32_t)r); uart_putchar(' ');
                uart_hex8(read_apsr()); uart_putchar('\n');
                clear_flags();
                __asm__ volatile ("qdadd %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
                uart_puts("QDADD "); uart_hex8((uint32_t)r); uart_putchar(' ');
                uart_hex8(read_apsr()); uart_putchar('\n');
                clear_flags();
                __asm__ volatile ("qdsub %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
                uart_puts("QDSUB "); uart_hex8((uint32_t)r); uart_putchar(' ');
                uart_hex8(read_apsr()); uart_putchar('\n');
            }
        }
    }

    // ---- Shifts by register (edge amounts) / MULS+flags / CLZ / RBIT / REV / BFI / BFC / UBFX / SBFX / SXTH ----
    {
        static const uint32_t SV[] = {0x12345678, 0x80000001, 0xFFFFFFFFu};
        static const uint32_t SH[] = {0, 1, 31, 32, 33, 255};
        for (int i = 0; i < 3; i++) {
            for (int j = 0; j < 6; j++) {
                uint32_t v = SV[i], s = SH[j], rr;
                __asm__ volatile ("lsl %0, %1, %2" : "=r" (rr) : "r" (v), "r" (s));
                uart_puts("LSL "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("lsr %0, %1, %2" : "=r" (rr) : "r" (v), "r" (s));
                uart_puts("LSR "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("asr %0, %1, %2" : "=r" (rr) : "r" (v), "r" (s));
                uart_puts("ASR "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("ror %0, %1, %2" : "=r" (rr) : "r" (v), "r" (s));
                uart_puts("ROR "); uart_hex8(rr); uart_putchar('\n');
            }
        }
        {
            VA = 0x12345678; VB = 0x11111111;
            a = VA; b = VB;
            clear_flags();
            r = a;
            __asm__ volatile ("muls %0, %1" : "+r" (r) : "r" (b));
            uart_puts("MULS "); uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
            clear_flags();
            __asm__ volatile ("subs %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SUBS "); uart_hex8((uint32_t)r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
        }
        {
            static const uint32_t CV[] = {0, 1, 0x80000000u, 0xFFFFFFFFu, 0x00F00000u};
            for (int i = 0; i < 5; i++) {
                uint32_t v = CV[i], rr;
                __asm__ volatile ("clz %0, %1" : "=r" (rr) : "r" (v));
                uart_puts("CLZ "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("rbit %0, %1" : "=r" (rr) : "r" (v));
                uart_puts("RBIT "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("rev %0, %1" : "=r" (rr) : "r" (v));
                uart_puts("REV "); uart_hex8(rr); uart_putchar('\n');
                __asm__ volatile ("rev16 %0, %1" : "=r" (rr) : "r" (v));
                uart_puts("REV16 "); uart_hex8(rr); uart_putchar('\n');
            }
        }
        {
            VA = 0x11223344; VB = 0xAABBCCDD;
            a = VA; b = VB;
            __asm__ volatile ("bfi %0, %1, #8, #8" : "+r" (a) : "r" (b));
            uart_puts("BFI "); uart_hex8((uint32_t)a); uart_putchar('\n');
            a = VA;
            __asm__ volatile ("bfc %0, #8, #8" : "+r" (a));
            uart_puts("BFC "); uart_hex8((uint32_t)a); uart_putchar('\n');
            __asm__ volatile ("ubfx %0, %1, #5, #7" : "=r" (r) : "r" (b));
            uart_puts("UBFX "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("sbfx %0, %1, #5, #7" : "=r" (r) : "r" (b));
            uart_puts("SBFX "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("sxth %0, %1" : "=r" (r) : "r" (b));
            uart_puts("SXTH "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("uxth %0, %1" : "=r" (r) : "r" (b));
            uart_puts("UXTH "); uart_hex8((uint32_t)r); uart_putchar('\n');
        }
    }

    // ---- FixedMul / FixedDiv differential core: LCG values across physics
    // magnitudes (momenta, friction, wall deltas, scales). Rolling checksum
    // every 64 iters. Any fixed-point divergence shows here.
    {
        uint32_t acc = 0;
        for (int i = 0; i < 512; i++) {
            int32_t a = (int32_t)lcg_next();
            int32_t b = (int32_t)lcg_next();
            // FixedMul(a,b) = (int64)a*b >> 16, via SMULL-grade codegen
            int64_t p = (int64_t)a * (int64_t)b;
            int32_t fxm = (int32_t)(p >> 16);
            acc = acc * 33u + (uint32_t)fxm;
            // FixedDiv(a,b): guard + divide (SDIV-grade), b==0-safe
            int32_t fxd;
            {
                int32_t aa = a ^ (a >> 31);
                int32_t ab = b ^ (b >> 31);
                uint32_t au = (uint32_t)(aa - (a >> 31));
                uint32_t bu = (uint32_t)(ab - (b >> 31));
                if ((au >> 14) >= bu) {
                    fxd = ((a ^ b) < 0) ? (int32_t)0x80000000 : 0x7FFFFFFF;
                } else {
                    int64_t q = ((int64_t)a << 16) / b;
                    fxd = (int32_t)q;
                }
            }
            acc = acc * 33u + (uint32_t)fxd;
            if ((i & 63) == 63) {
                uart_puts("FXCK "); uart_hex8(acc); uart_putchar('\n');
            }
        }
        uart_puts("FXSUM "); uart_hex8(acc); uart_putchar('\n');
    }

    // ---- Load/store addressing surface: same value read/written via
    // imm5-T1, imm12-W, literal pool, register-offset (plain + lsl#2),
    // post-indexed and pre-indexed forms, widths B/H/W/SB/SH. The demo
    // split survives a clean ALU, so the fault (if any) lives here or in
    // branches: same inputs + same arithmetic can still split on an
    // addressing or predicate edge.
    {
        static volatile uint8_t bytes[64];
        static volatile uint32_t words[16];
        for (int i = 0; i < 64; i++) bytes[i] = (uint8_t)(0x31 + i * 7);
        for (int i = 0; i < 16; i++) words[i] = 0x10203040u + (uint32_t)i * 0x01010101u;
        uint32_t r;
        // LDRB imm5-T1 vs imm12-W vs literal vs reg-offset vs post-indexed
        __asm__ volatile ("ldrb %0, [%1, #7]" : "=r" (r) : "r" (bytes));
        uart_puts("LB5 "); uart_hex8(r); uart_putchar('\n');
        __asm__ volatile ("ldrb.w %0, [%1, #60]" : "=r" (r) : "r" (bytes));
        uart_puts("LB12 "); uart_hex8(r); uart_putchar('\n');
        {
            // literal-pool LDRB: force pool placement via large function offset is
            // unreliable; use LDR.W literal of a word + byte extract instead.
            uint32_t w;
            __asm__ volatile ("ldr.w %0, [%1]" : "=r" (w) : "r" (words));
            uart_puts("LW0 "); uart_hex8(w); uart_putchar('\n');
        }
        {
            uint32_t idx = 5;
            __asm__ volatile ("ldrb %0, [%1, %2]" : "=r" (r) : "r" (bytes), "r" (idx));
            uart_puts("LBR "); uart_hex8(r); uart_putchar('\n');
            __asm__ volatile ("ldrb %0, [%1, %2, lsl #2]" : "=r" (r) : "r" (bytes), "r" (idx));
            uart_puts("LBR2 "); uart_hex8(r); uart_putchar('\n');
            __asm__ volatile ("ldrb %0, [%1], #9" : "+r" (idx) : "r" (bytes) : );
            (void)idx;
        }
        // LDRH/LDRSH/STRH + LDRSB sign forms
        {
            uint32_t h;
            __asm__ volatile ("ldrh %0, [%1, #6]" : "=r" (h) : "r" (bytes));
            uart_puts("LH5 "); uart_hex8(h); uart_putchar('\n');
            __asm__ volatile ("ldrsh %0, [%1, #6]" : "=r" (h) : "r" (bytes));
            uart_puts("LSH5 "); uart_hex8(h); uart_putchar('\n');
            __asm__ volatile ("ldrsb %0, [%1, #63]" : "=r" (h) : "r" (bytes));
            uart_puts("LSB63 "); uart_hex8(h); uart_putchar('\n');
        }
        // STRB/STRH/STR writeback + pre-indexed forms, read back in C
        {
            uint32_t *p = (uint32_t *)words;
            __asm__ volatile (
                "movw r4, #0xBEEF\n"
                "strh r4, [%0, #4]!\n"
                "movw r4, #0xDEAD\n movt r4, #0xBEEF\n"
                "str r4, [%0, #-4]\n"
                "movs r4, #0xA5\n"
                "strb r4, [%0, #7]\n"
                : "+r" (p) :: "memory", "r4");
            uart_puts("STWB "); uart_hex8(words[1]); uart_putchar(' ');
            uart_hex8(words[0]); uart_putchar('\n');
            uart_puts("STB7 "); uart_hex8(words[1]); uart_putchar('\n');
        }
    }
    // ---- Parallel UADD8/USUB8 + SEL (GE-mediated SIMD search idiom from
    // memchr). Result AND GE-carrying APSR compared (GE drives SEL).
    {
        static const uint32_t AV[] = {0x11223344u, 0xFFFFFFFFu, 0x00000000u, 0x80808080u, 0x7F7F7F7Fu};
        static const uint32_t BV[] = {0xAABBCCDDu, 0x00000001u, 0xFFFFFFFFu, 0x01010101u, 0x01010101u};
        for (int i = 0; i < 5; i++) {
            uint32_t a = AV[i], b = BV[i], r;
            clear_flags();
            __asm__ volatile ("uadd8 %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("UADD8 "); uart_hex8(r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
            clear_flags();
            __asm__ volatile ("usub8 %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("USUB8 "); uart_hex8(r); uart_putchar(' ');
            uart_hex8(read_apsr()); uart_putchar('\n');
            __asm__ volatile ("sel %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SEL "); uart_hex8(r); uart_putchar('\n');
        }
    }
    // ---- UMLAL (unsigned 64-bit accumulate, op-14 plain form) ----
    {
        static const uint32_t AV[] = {0x12345678u, 0xFFFFFFFFu, 0u, 0x80000000u};
        static const uint32_t BV[] = {0x11111111u, 0xFFFFFFFFu, 0x12345678u, 2u};
        for (int i = 0; i < 4; i++) {
            uint32_t lo = 0x01020304u, hi = 0x05060708u;
            uint32_t a = AV[i], b = BV[i];
            __asm__ volatile ("umlal %0, %1, %2, %3" : "+r" (lo), "+r" (hi) : "r" (a), "r" (b));
            uart_puts("UMLAL "); uart_hex8(lo); uart_putchar(' ');
            uart_hex8(hi); uart_putchar('\n');
        }
    }
    // ---- Memory-path x region matrix: B/H/W loads+stores to SRAM
    // (0x2000...), EXTRAM (0xC000...), and the WAD mapping (0xB800...),
    // aligned AND misaligned. A region/width-specific silent drop or
    // rotation here explains static-data divergence (sector light/special
    // read 0 on one backend) with identical code and identical ALU.
    // EXTRAM scratch MUST avoid doom's save area (0xC0080000): use top.
#define EXTMEM ((volatile uint8_t *)0xC0F00000u)
    {
        // SRAM control first (known-good baseline)
        static volatile uint32_t sramw[4];
        sramw[0] = 0x11223344u; sramw[1] = 0xAABBCCDDu;
        uart_puts("MEMS "); uart_hex8(sramw[0]); uart_putchar(' ');
        uart_hex8(sramw[1]); uart_putchar('\n');
        // EXTRAM word + halfword + byte round-trips
        volatile uint32_t *ew = (volatile uint32_t *)0xC0F00000u;
        ew[0] = 0x12345678u; ew[1] = 0x9ABCDEF0u;
        uart_puts("MEMXW "); uart_hex8(ew[0]); uart_putchar(' ');
        uart_hex8(ew[1]); uart_putchar('\n');
        *(volatile uint16_t *)(0xC0F00008u) = 0xBEEFu;
        *(volatile uint16_t *)(0xC0F0000Au) = 0x1234u;
        uart_puts("MEMXH ");
        uart_hex8(*(volatile uint16_t *)(0xC0F00008u)); uart_putchar(' ');
        uart_hex8(*(volatile uint16_t *)(0xC0F0000Au)); uart_putchar(' ');
        uart_hex8(*(volatile uint32_t *)(0xC0F00008u)); uart_putchar('\n');
        *(volatile uint8_t *)(0xC0F00100u) = 0x5Au;
        *(volatile uint8_t *)(0xC0F00101u) = 0xA5u;
        uart_puts("MEMXB ");
        uart_hex8(*(volatile uint8_t *)(0xC0F00100u)); uart_putchar(' ');
        uart_hex8(*(volatile uint8_t *)(0xC0F00101u)); uart_putchar(' ');
        uart_hex8(*(volatile uint32_t *)(0xC0F00100u)); uart_putchar('\n');
        // misaligned word + halfword on EXTRAM
        uart_puts("MEMXU ");
        uart_hex8(*(volatile uint32_t *)(0xC0F00001u)); uart_putchar(' ');
        uart_hex8(*(volatile uint16_t *)(0xC0F00003u)); uart_putchar('\n');
        // WAD-map reads (bytes/halfword/word at known offsets) + write attempt
        uart_puts("MEMWB ");
        uart_hex8(*(volatile uint8_t *)(0xB8000000u)); uart_putchar(' ');
        uart_hex8(*(volatile uint8_t *)(0xB8000001u)); uart_putchar(' ');
        uart_hex8(*(volatile uint16_t *)(0xB8000000u)); uart_putchar(' ');
        uart_hex8(*(volatile uint32_t *)(0xB8000000u)); uart_putchar('\n');
        (void)EXTMEM;
    }
    {
        // flag setups: subs pairs chosen for NZCV variety
        struct { uint32_t a, b; } fp[] = {{0,0},{1,0},{0,1},{0x7FFFFFFF,0xFFFFFFFFu},{0x80000000,1}};
        for (int s = 0; s < 5; s++) {
            uint32_t word = 0;
            uint32_t a = fp[s].a, b = fp[s].b;
            // for each cond: refresh NZCV via subs first (orrs below
            // would otherwise clobber the flags the next cond reads!).
            // Predicated pairs need explicit IT (Thumb-2 rule).
            __asm__ volatile (
                "subs r4, %1, %2\n"
                "ite eq\n moveq r5, #1\n movne r5, #0\n orrs %0, %0, r5, lsl #0\n"
                "subs r4, %1, %2\n"
                "ite cs\n movcs r5, #1\n movcc r5, #0\n orrs %0, %0, r5, lsl #1\n"
                "subs r4, %1, %2\n"
                "ite mi\n movmi r5, #1\n movpl r5, #0\n orrs %0, %0, r5, lsl #2\n"
                "subs r4, %1, %2\n"
                "ite vs\n movvs r5, #1\n movvc r5, #0\n orrs %0, %0, r5, lsl #3\n"
                "subs r4, %1, %2\n"
                "ite hi\n movhi r5, #1\n movls r5, #0\n orrs %0, %0, r5, lsl #4\n"
                "subs r4, %1, %2\n"
                "ite ge\n movge r5, #1\n movlt r5, #0\n orrs %0, %0, r5, lsl #5\n"
                "subs r4, %1, %2\n"
                "ite gt\n movgt r5, #1\n movle r5, #0\n orrs %0, %0, r5, lsl #6\n"
                : "+r" (word) : "r" (a), "r" (b) : "r4", "r5", "cc");
            uart_puts("BCC "); uart_hex8(word); uart_putchar('\n');
        }
        {
            // CBZ/CBNZ forward-taken, forward-untaken, backward loop.
            // Accumulate with ORRS (plain movs would overwrite).
            uint32_t m = 0;
            __asm__ volatile (
                "movs r4, #0\n cbz r4, 11f\n orrs %0, %0, #1\n11:\n"
                "movs r4, #7\n cbz r4, 12f\n orrs %0, %0, #2\n12:\n"
                "movs r4, #0\n cbnz r4, 13f\n orrs %0, %0, #4\n13:\n"
                "movs r4, #3\n"
                "14:\n subs r4, #1\n bne 14b\n"
                "orrs %0, %0, #8\n"
                : "+r" (m) :: "r4", "cc");
            uart_puts("CBZ "); uart_hex8(m); uart_putchar('\n');
        }
        {
            // ITTEE eq: slots T,T,E,E with Z=1 -> runs slots 0,1 only
            uint32_t m = 0;
            __asm__ volatile (
                "movs r4, #0\n"
                "ittee eq\n"
                "moveq %0, #0x1\n"
                "moveq r4, #0\n"
                "movne %0, #0x10\n"
                "movne r4, #0\n"
                "orrs %0, %0, r4\n"
                : "+r" (m) :: "r4", "cc");
            uart_puts("ITE2 "); uart_hex8(m); uart_putchar('\n');
        }
        {
            // ITETT ne with Z=0: runs slots 0,2,3 (T,E,T,T)
            uint32_t m = 0;
            __asm__ volatile (
                "movs r4, #1\n"
                "itett ne\n"
                "movne %0, #0x1\n"
                "moveq r4, #0\n"
                "movne %0, #0x10\n"
                "movne %0, #0x100\n"
                : "+r" (m) :: "r4", "cc");
            uart_puts("ITE3 "); uart_hex8(m); uart_putchar('\n');
        }
        {
            // ITF1: predicated SUBS that FAIL must preserve NZCV; that RUN
            // must update. E/N pair per setup brackets both sides for every Z.
            struct { uint32_t a, b; } fq[] = {{0,0},{0,1},{0x7FFFFFFFu,0xFFFFFFFFu},{0x80000000u,1}};
            for (int s = 0; s < 4; s++) {
                uint32_t f, a = fq[s].a, b = fq[s].b;
                __asm__ volatile (
                    "subs r4, %1, %2\n"
                    "it eq\n subseq r5, %1, %2\n"
                    "mrs %0, apsr\n"
                    : "=r" (f) : "r" (a), "r" (b) : "r4", "r5", "cc");
                uart_puts("ITF1E "); uart_hex8(f); uart_putchar('\n');
                __asm__ volatile (
                    "subs r4, %1, %2\n"
                    "it ne\n subsne r5, %1, %2\n"
                    "mrs %0, apsr\n"
                    : "=r" (f) : "r" (a), "r" (b) : "r4", "r5", "cc");
                uart_puts("ITF1N "); uart_hex8(f); uart_putchar('\n');
            }
            // ITF1M: skipped MOVS must preserve flags AND leave Rd untouched.
            {
                uint32_t r5v, fm;
                __asm__ volatile (
                    "movs r5, #0x12\n subs r4, r4\n"
                    "it ne\n movsne r5, #0xFF000000\n"
                    "mrs %0, apsr\n mov %1, r5\n"
                    : "=r" (fm), "=r" (r5v) :: "r4", "r5", "cc");
                uart_puts("ITF1M "); uart_hex8(r5v); uart_putchar(' ');
                uart_hex8(fm); uart_putchar('\n');
                __asm__ volatile (
                    "movs r5, #0x12\n subs r4, #1\n"
                    "it eq\n movseq r5, #0xFF000000\n"
                    "mrs %0, apsr\n mov %1, r5\n"
                    : "=r" (fm), "=r" (r5v) :: "r4", "r5", "cc");
                uart_puts("ITF1M2 "); uart_hex8(r5v); uart_putchar(' ');
                uart_hex8(fm); uart_putchar('\n');
            }
        }
        {
            // ITF2: later slots evaluate against LIVE flags (changed by an
            // earlier slot in the same block), not the entry flags.
            uint32_t r6v, f2;
            __asm__ volatile (
                "movs r6, #0\n subs r4, r4\n"   // Z=1
                "ite eq\n"
                "subseq r5, r4, #1\n"           // runs: Z->0
                "movne r6, #0xBB\n"             // ne now true -> runs
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f2), "=r" (r6v) :: "r4", "r5", "r6", "cc");
            uart_puts("ITF2E "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f2); uart_putchar('\n');
            __asm__ volatile (
                "movs r6, #0\n subs r4, #1\n"   // Z=0
                "ite eq\n"
                "subseq r5, r4, #1\n"           // skipped: flags stay Z=0
                "movne r6, #0xBB\n"             // runs
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f2), "=r" (r6v) :: "r4", "r5", "r6", "cc");
            uart_puts("ITF2N "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f2); uart_putchar('\n');
        }
        {
            // ITF3: skipped 16-bit T1 ADDS preserves flags and Rd.
            uint32_t r5v, f3;
            __asm__ volatile (
                "movs r5, #5\n subs r4, r4\n"   // Z=1
                "it ne\n addsne r5, #1\n"       // skipped
                "mrs %0, apsr\n mov %1, r5\n"
                : "=r" (f3), "=r" (r5v) :: "r4", "r5", "cc");
            uart_puts("ITF3N "); uart_hex8(r5v); uart_putchar(' ');
            uart_hex8(f3); uart_putchar('\n');
            __asm__ volatile (
                "movs r5, #5\n subs r4, #1\n"   // Z=0
                "it eq\n addseq r5, #1\n"       // skipped
                "mrs %0, apsr\n mov %1, r5\n"
                : "=r" (f3), "=r" (r5v) :: "r4", "r5", "cc");
            uart_puts("ITF3E "); uart_hex8(r5v); uart_putchar(' ');
            uart_hex8(f3); uart_putchar('\n');
        }
        {
            // ITF4: mask sweep with flag-setting slot ops (Z=1 entry).
            // orrs slots interact with live flags; r6 spread pins mask rule.
            // (First mask slot is always T by encoding rule: ttt/tte/tet/tee.)
            uint32_t r6v, f4;
            __asm__ volatile (
                "movs r6, #0\n subs r4, r4\n"
                "ittt eq\n orrseq r6, #1\n orrseq r6, #2\n orrseq r6, #4\n"
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f4), "=r" (r6v) :: "r4", "r6", "cc");
            uart_puts("ITF4_0 "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f4); uart_putchar('\n');
            __asm__ volatile (
                "movs r6, #0\n subs r4, r4\n"
                "itte eq\n orrseq r6, #1\n orrseq r6, #2\n orrsne r6, #4\n"
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f4), "=r" (r6v) :: "r4", "r6", "cc");
            uart_puts("ITF4_1 "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f4); uart_putchar('\n');
            __asm__ volatile (
                "movs r6, #0\n subs r4, r4\n"
                "itet eq\n orrseq r6, #1\n orrsne r6, #2\n orrseq r6, #4\n"
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f4), "=r" (r6v) :: "r4", "r6", "cc");
            uart_puts("ITF4_2 "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f4); uart_putchar('\n');
            __asm__ volatile (
                "movs r6, #0\n subs r4, r4\n"
                "itee eq\n orrseq r6, #1\n orrsne r6, #2\n orrsne r6, #4\n"
                "mrs %0, apsr\n mov %1, r6\n"
                : "=r" (f4), "=r" (r6v) :: "r4", "r6", "cc");
            uart_puts("ITF4_3 "); uart_hex8(r6v); uart_putchar(' ');
            uart_hex8(f4); uart_putchar('\n');
        }
    }
    {
        static volatile uint32_t buf[32];
        uint32_t *p = (uint32_t *)buf;
        uint32_t a, b, c, d, sp0, sp1;
        __asm__ volatile ("mov %0, sp" : "=r" (sp0));
        __asm__ volatile (
            "movw r4, #0x1111\n movt r4, #0x1111\n"
            "movw r5, #0x2222\n movt r5, #0x2222\n"
            "movw r6, #0x3333\n movt r6, #0x3333\n"
            "movw r7, #0x4444\n movt r7, #0x4444\n"
            "stmdb %0!, {r4-r7}\n"
            "movs r4, #0\n movs r5, #0\n movs r6, #0\n movs r7, #0\n"
            "ldmia %0!, {r4-r7}\n"
            "mov %1, r4\n mov %2, r5\n mov %3, r6\n mov %4, r7\n"
            : "+r" (p), "=r" (a), "=r" (b), "=r" (c), "=r" (d)
            :: "memory", "r4", "r5", "r6", "r7");
        __asm__ volatile ("mov %0, sp" : "=r" (sp1));
        uart_puts("STM1 "); uart_hex8(a); uart_putchar(' ');
        uart_hex8(b); uart_putchar(' '); uart_hex8(c); uart_putchar(' ');
        uart_hex8(d); uart_putchar(' '); uart_hex8(sp1 - sp0); uart_putchar('\n');
        // high regs, no-writeback store + writeback load
        {
            uint32_t *q = (uint32_t *)buf;
            uint32_t e, f;
            __asm__ volatile (
                "movw r8, #0x9999\n movt r8, #0x9999\n"
                "movw r10, #0xAAAA\n movt r10, #0xAAAA\n"
                "movw r4, #0x5555\n movt r4, #0x5555\n"
                "movw r5, #0x6666\n movt r5, #0x6666\n"
                "stmia %0, {r4, r5, r8, r10}\n"
                "movs r4, #0\n movs r5, #0\n"
                "ldmia %0!, {r4, r5, r8, r10}\n"
                "mov %1, r8\n mov %2, r10\n"
                : "+r" (q), "=r" (e), "=r" (f)
                :: "memory", "r4", "r5", "r8", "r10");
            uart_puts("STMH "); uart_hex8(e); uart_putchar(' ');
            uart_hex8(f); uart_putchar('\n');
        }
        // strd post-indexed + ldrd offset round-trip (64-bit struct copies)
        {
            int64_t wv = (int64_t)0x1122334455667788LL;
            uint32_t lo = (uint32_t)wv, hi = (uint32_t)(wv >> 32);
            uint32_t rlo = 0, rhi = 0;
            uint32_t *p = (uint32_t *)buf;
            __asm__ volatile (
                "strd %3, %4, [%0], #8\n"
                "ldrd %1, %2, [%0, #-8]\n"
                : "+r" (p), "=r" (rlo), "=r" (rhi)
                : "r" (lo), "r" (hi) : "memory");
            uart_puts("STRD "); uart_hex8(rlo); uart_putchar(' ');
            uart_hex8(rhi); uart_putchar('\n');
        }
    }
    {
        static const int SH[] = {0, 1, 15, 16, 31, 32, 33, 63};
        static const int64_t VV[] = {0, 1, -1, 0x123456789ABCDEFLL,
                                     (int64_t)0x8000000000000000LL,
                                     0x7FFFFFFFFFFFFFFFLL, 0x0000000100000000LL};
        for (int i = 0; i < 6; i++) {
            for (int j = 0; j < 8; j++) {
                int64_t v = VV[i];
                int s = SH[j];
                volatile int sv = s;
                volatile int64_t vv = v;
                int64_t ga = vv >> sv;              // ASR (possibly reg-count)
                uint64_t gl = (uint64_t)vv >> sv;   // LSR
                int64_t gb = vv << sv;              // LSL
                int64_t ga16 = vv >> 16;            // ASR-imm (FixedMul >>16!)
                uint64_t gl16 = (uint64_t)vv >> 16; // LSR-imm
                uart_puts("SH64 ");
                uart_hex8((uint32_t)(ga >> 32)); uart_hex8((uint32_t)ga);
                uart_hex8((uint32_t)(gl >> 32)); uart_hex8((uint32_t)gl);
                uart_hex8((uint32_t)(gb >> 32)); uart_hex8((uint32_t)gb);
                uart_hex8((uint32_t)(ga16 >> 32)); uart_hex8((uint32_t)ga16);
                uart_hex8((uint32_t)(gl16 >> 32)); uart_hex8((uint32_t)gl16);
                uart_putchar('\n');
            }
        }
        // NOTE: ref values computed on HOST (x86-64, exact) vs guest values;
        // any mismatch line pinpoints the op. (Host prints first per group.)
        {
            int64_t acc = 0;
            for (int i = 0; i < 256; i++) {
                int64_t a = (int64_t)(int32_t)lcg_next();
                int64_t b = (int64_t)(int32_t)lcg_next();
                int64_t p = a * b;                 // __aeabi_lmul-grade
                int64_t q = (b == 0) ? 0 : (a / b); // sdiv-grade (avoid div0 trap parity questions)
                int64_t r = (b == 0) ? 0 : ((uint64_t)a / (uint64_t)b);
                acc = acc * 33u + (uint64_t)(p >> 32) + (uint64_t)p + (uint64_t)(q >> 32) + (uint64_t)q + (uint64_t)(r >> 32) + (uint64_t)r;
                if ((i & 63) == 63) {
                    uart_puts("LMCK "); uart_hex8((uint32_t)(acc >> 32));
                    uart_hex8((uint32_t)acc); uart_putchar('\n');
                }
            }
            uart_puts("LMSUM "); uart_hex8((uint32_t)(acc >> 32));
            uart_hex8((uint32_t)acc); uart_putchar('\n');
        }
    }

    // ---- Flag corners: SUBS/ADDS/CMP/RSB/ADC/SBC near carry/borrow/overflow
    // edges + PRINTED NZCV. A flag bit wrong flips one guest branch (e.g. in
    // wall-slide code) into a discrete trajectory split.
    {
        static const uint32_t FV[] = {0u, 1u, 0xFFFFFFFFu, 0x7FFFFFFFu, 0x80000000u,
                                      0x80000001u, 0x7FFFFFFEu, 0x00FF00FFu, 0xFF00FF00u};
        for (int i = 0; i < 9; i++) {
            for (int j = 0; j < 9; j++) {
                uint32_t a = FV[i], b = FV[j], r;
                __asm__ volatile (
                    "subs %0, %1, %2\n"
                    "mrs %1, apsr\n"
                    : "=r" (r), "+r" (a) : "r" (b) : "cc");
                // NOTE: a clobbered intentionally (holds APSR now); print r + a
                uart_puts("FSUB "); uart_hex8(r); uart_putchar(' ');
                uart_hex8(a); uart_putchar('\n');
                a = FV[i];
                __asm__ volatile (
                    "adds %0, %1, %2\n"
                    "mrs %1, apsr\n"
                    : "=r" (r), "+r" (a) : "r" (b) : "cc");
                uart_puts("FADD "); uart_hex8(r); uart_putchar(' ');
                uart_hex8(a); uart_putchar('\n');
            }
        }
        // ADC/SBC chains with C=0 and C=1 in
        for (int c = 0; c < 2; c++) {
            uint32_t a = 0xFFFFFFFFu, b = 0x00000001u, r, f;
            if (c) {
                __asm__ volatile (
                    "subs r0, r0\n"          // C=1 (0-0, no borrow)
                    "adcs %0, %1, %2\n"
                    "mrs %3, apsr\n"
                    : "=r" (r), "+r" (a), "+r" (b), "=r" (f) :: "r0", "cc");
            } else {
                __asm__ volatile (
                    "subs r0, #1\n"          // C=0 (0-1 borrows); r0=0xFFFFFFFF
                    "sbcs %0, %1, %2\n"
                    "mrs %3, apsr\n"
                    : "=r" (r), "+r" (a), "+r" (b), "=r" (f) :: "r0", "cc");
            }
            uart_puts(c ? "ADC1 " : "SBC0 ");
            uart_hex8(r); uart_putchar(' ');
            uart_hex8(f); uart_putchar('\n');
        }
    }

    *(volatile uint32_t *)0x20001004u = 0x22222222u;
    // ---- SMUAD/SMUSD (Ra==15 dual, no accumulate) + SMLAWT (top-top) +
    // shifted SAT/PKH forms. Prior rounds only covered Ra!=15/plain/shift-0
    // variants of these exact arms.
    {
        static const int32_t AV[] = {(int32_t)0x80008000, 0x7FFF7FFF, 0x1234FEDC, (int32_t)0xFFFF0001};
        static const int32_t BV[] = {(int32_t)0x80008000, 0x7FFF7FFF, (int32_t)0xEDCBA987, 0x0002FFFE};
        for (int i = 0; i < 4; i++) {
            int32_t a = AV[i], b = BV[i], r;
            *(volatile uint32_t *)(0x20001010u + (uint32_t)i * 4u) = 0xA0u + (uint32_t)i;
            __asm__ volatile ("smuad %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMUAD "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smusd %0, %1, %2" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("SMUSD "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("smlawt %0, %1, %2, %3" : "=r" (r) : "r" (a), "r" (b), "r" (0x11112222));
            uart_puts("SMLAWT "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("pkhbt %0, %1, %2, lsl #5" : "=r" (r) : "r" (a), "r" (b));
            uart_puts("PKHS5 "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("ssat %0, #16, %1, lsl #3" : "=r" (r) : "r" (a));
            uart_puts("SSATSH "); uart_hex8((uint32_t)r); uart_putchar('\n');
            __asm__ volatile ("usat %0, #12, %1, lsl #3" : "=r" (r) : "r" (a));
            uart_puts("USATSH "); uart_hex8((uint32_t)r); uart_putchar('\n');
        }
    }
    // ---- Section markers (distinguish lost-UART from skipped-code):
    // each major section after this point stamps a known SRAM word.
    // If unicorn is missing UART lines but HAS the stamps, its execution
    // ran fine and output was lost (buffer cap); if stamps are missing,
    // it skipped code (control flow).
    *(volatile uint32_t *)0x20001000u = 0x11111111u;
    *(volatile uint32_t *)0x20001008u = 0x33333333u;
    // ---- Fault parity: encodings that must STOP (not silently continue)
    // on both backends. Runs LAST (a fault ends the run by design).
    {
        uart_puts("=== FUZZ-FAULTS ===\n");
        __asm__ volatile ("bkpt #0");
        uart_puts("FAULT-MISS-BKPT\n");
        __asm__ volatile ("svc #0");
        uart_puts("FAULT-MISS-SVC\n");
        __asm__ volatile (".word 0xEDD00A00");   // vldr s0, [r0] (no FPU)
        uart_puts("FAULT-MISS-VLDR\n");
        __asm__ volatile ("smlsld r2, r3, r4, r5"); // dual-sub long (unimpl)
        uart_puts("FAULT-MISS-SMLSLD\n");
        __asm__ volatile ("umaal r0, r1, r2, r3");  // unsigned long add-acc
        uart_puts("FAULT-MISS-UMAAL\n");
    }

    uart_puts("=== FUZZ-DONE ===\n");
    while (1) { }
    return 0;
}
