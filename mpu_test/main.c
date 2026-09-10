// MPU enforcement proof: real region programming, then legal accesses
// must pass while violations take MemManage with exact MMFSR/MMFAR.
//
// Memory map used here (all inside the 128KB SRAM):
//   .bss/stack (MSP top): privileged-only code/data (R1 below).
//   0x20001000: shared state (fails/count/fsrs/fars) — the ONLY
//     data touched while unprivileged; R3 (FULL) covers it.
//   0x200013E0: PSP initial value (grows down, ~200B max depth, stays in
//     R3; must be strictly inside: 0x20001400 is already past R3's end
//     (0x200013FF) and even [sp,#4] spill slots fault there).
//   0x20002000: 32B no-access probe window (R4).
//   0x20003000: XN-exec probe slot (R1 XN).
// Regions (number = priority): R0 FLASH RX (RO both, so unprivileged fetch
// works), R1 SRAM 128KB RW-priv/XN, R2 peripherals 512MB FULL/XN, R3
// scratch 1KB FULL, R4 32B no-access. CTRL = ENABLE|PRIVDEFENA (background
// needed: PPB has no region of its own).
// Phases: legal R/W -> no-access write (DACCVIOL) -> XN exec (IACCVIOL)
// -> switch to unprivileged (PSP in R3) -> FLASH RO read (pass) ->
// SRAM-priv write (DACCVIOL) -> PPB read (DACCVIOL, proves no exemption)
// -> scratch write (pass) -> SVC back to privileged MSP -> done.
// Each faulting op is followed by a check() on the recorded
// MMFSR/MMFAR/count. Data faults are deferred-by-one in this model (the
// access completes dropped, stacked PC already past it) so the handler
// returns untouched; the XN fetch fault is precise so the handler skips
// the blx (fault #2, length-decoded like thumb::len). Deliberately no
// &&label resume pointers: GCC sinks/misplaces them across the faulting
// op (observed: volatile store emitted after the str, label 16B early),
// which fault-loops forever — see AGENTS.md §25.
//
// NOTE: startup.c does NOT zero .bss / copy .data — the few .bss words
// (saved_msp) are assigned before use. Only integer ops (no FPU here).
#include <stdint.h>

#define MPU_BASE 0xE000ED90
#define MPU_TYPE (*(volatile unsigned int *)(MPU_BASE + 0x00))
#define MPU_CTRL (*(volatile unsigned int *)(MPU_BASE + 0x04))
#define MPU_RNR  (*(volatile unsigned int *)(MPU_BASE + 0x08))
#define MPU_RBAR (*(volatile unsigned int *)(MPU_BASE + 0x0C))
#define MPU_RASR (*(volatile unsigned int *)(MPU_BASE + 0x10))
#define SHCSR (*(volatile unsigned int *)0xE000ED24)
#define CFSR_MMFSR() (*(volatile unsigned int *)0xE000ED28)
#define CFSR_MMFAR() (*(volatile unsigned int *)0xE000ED34)

#define SCR_BASE 0x20001000u
#define SCR_FAILS (*(volatile unsigned int *)(SCR_BASE + 0))
#define SCR_COUNT (*(volatile unsigned int *)(SCR_BASE + 4))
#define SCR_FSRS ((volatile unsigned int *)(SCR_BASE + 8))
#define SCR_FARS ((volatile unsigned int *)(SCR_BASE + 40))

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

static volatile unsigned int saved_msp;

// Naked trampoline: captures the fault frame pointer (MSP or PSP per
// EXC_RETURN bit 2) into r0 BEFORE any push — a C prologue's own push
// would shift SP and corrupt every frame+offset below. Then tail-calls
// the C handler, which returns directly to the exception return.
__attribute__((naked)) void MemManage_Handler(void) {
    __asm__ volatile (
        "tst lr, #4\n\t"
        "ite eq\n\t"
        "mrseq r0, msp\n\t"
        "mrsne r0, psp\n\t"
        "b MemManage_Handler_c\n\t");
}

void MemManage_Handler_c(unsigned int *frame) {
    unsigned int n = SCR_COUNT;
    if (n < 8) {
        SCR_FSRS[n] = CFSR_MMFSR();
        SCR_FARS[n] = CFSR_MMFAR();
    }
    SCR_COUNT = n + 1;
    // Sampled: clear the sticky flags so each fault records its own exact
    // bits (model CFSR is plain-write; on silicon this would be w1c).
    *(volatile unsigned int *)0xE000ED28 = 0;
    // Resume rule (matches the model's deferred-by-one data path): data
    // faults raise BEFORE the next fetch, so the stacked PC is already past
    // the faulting access — return untouched. Fetch (XN) faults are precise:
    // the stacked PC still points at the XN target, and stepping past it
    // just faults again (still XN ground) — so resume at the stacked LR
    // instead. The probe calls via blx, which always sets LR to the
    // instruction after the branch (fault #2 here). Deliberately no &&label
    // resume pointers: GCC sinks/misplaces them across the faulting op
    // (observed: volatile store emitted after the str, label 16B early),
    // which fault-loops forever — see AGENTS.md §25.
    // frame[] is the 8-word stacked frame (no FPU here): [5]=stacked LR,
    // [6]=retpc. The trampoline passed the pre-push pointer, so no SP
    // arithmetic is needed (or valid — our own push shifted SP) here.
    if (n == 1) {
        frame[6] = frame[5];
    }
}

void SVC_Handler(void) {
    // Back to privileged; keep SPSEL (still on PSP here) — main restores
    // MSP explicitly after return (see below).
    unsigned int c;
    __asm__ volatile ("mrs %0, control" : "=r"(c));
    c &= ~1u;
    __asm__ volatile ("msr control, %0" :: "r"(c));
}

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
        SCR_FAILS++;
    }
}

// 16-bit bx lr at dst (for the XN-exec probe).
static void mkret(unsigned int dst) {
    *(volatile unsigned short *)dst = 0x4770;
}

int main(void) {
    uart_init();
    uart_puts("=== MPU Test ===\r\n");

    // R0 FLASH 1MB RX, RO both (unprivileged fetch must work later).
    MPU_RNR = 0;
    MPU_RBAR = 0x08000010;
    MPU_RASR = 0x06000027;
    // R1 SRAM 128KB RW-priv, XN.
    MPU_RNR = 1;
    MPU_RBAR = 0x20000011;
    MPU_RASR = 0x11000021;
    // R2 peripherals 512MB FULL, XN.
    MPU_RNR = 2;
    MPU_RBAR = 0x40000002;
    MPU_RASR = 0x13000039;
    // R3 scratch 1KB FULL (unpriv stack+data lives here).
    MPU_RNR = 3;
    MPU_RBAR = 0x20001003;
    MPU_RASR = 0x13000013;
    // R4 32B no-access (beats R1 by region number).
    MPU_RNR = 4;
    MPU_RBAR = 0x20002004;
    MPU_RASR = 0x00000009;
    uart_puts("REGIONS OK\r\n");

    SCR_FAILS = 0;
    SCR_COUNT = 0;
    for (int i = 0; i < 8; i++) { SCR_FSRS[i] = 0; SCR_FARS[i] = 0; }
    SHCSR |= (1u << 16); // MEMFAULTENA: route violations to MemManage (not HardFault)
    MPU_CTRL = 0x5; // ENABLE | PRIVDEFENA
    uart_puts("MPU enabled CTRL=");
    uart_x32(MPU_CTRL);
    uart_puts(" R4ASR=");
    MPU_RNR = 4;
    uart_x32(MPU_RASR);
    uart_puts("\r\n");

    // Legal: SRAM R/W (proves enforcement passes clean traffic).
    *(volatile unsigned int *)0x20000100 = 0x12345678;
    if (*(volatile unsigned int *)0x20000100 != 0x12345678) {
        uart_puts("LEGAL FAIL\r\n");
        SCR_FAILS++;
    } else {
        uart_puts("LEGAL OK\r\n");
    }

    // No-access data write -> DACCVIOL + MMFAR. Deferred: the handler
    // returns past the store with no fixup needed.
    *(volatile unsigned int *)0x20002000 = 0xDEAD;
    check("NOACC", (SCR_COUNT == 1 && SCR_FSRS[0] == 0x82 && SCR_FARS[0] == 0x20002000) ? 1 : 0, 1);

    // XN execute-from-SRAM -> IACCVIOL + MMFAR=PC. Precise: the handler
    // skips the blx (fault #2).
    mkret(0x20003000);
    ((void (*)(void))0x20003001)();
    check("XNEXEC", (SCR_COUNT == 2 && SCR_FSRS[1] == 0x81 && SCR_FARS[1] == 0x20003000) ? 1 : 0, 1);

    // Drop to unprivileged on PSP (stack in the FULL scratch region).
    __asm__ volatile ("mrs %0, msp" : "=r"(saved_msp));
    __asm__ volatile (
        "msr psp, %0\n\t"
        "movs r0, #3\n\t"
        "msr control, r0\n\t"
        "isb\n\t" :: "r"(0x200013E0) : "r0");
    // Unprivileged FLASH RO read passes.
    if (*(volatile unsigned int *)0x08000000 != 0x20020000) {
        uart_puts("UPRIV-RO FAIL\r\n");
        SCR_FAILS++;
    } else {
        uart_puts("UPRIV-RO OK\r\n");
    }
    // Unprivileged SRAM-priv write -> DACCVIOL (deferred, no fixup).
    *(volatile unsigned int *)0x20000100 = 0xAAAA;
    check("UPRIV-W", (SCR_COUNT == 3 && SCR_FSRS[2] == 0x82 && SCR_FARS[2] == 0x20000100) ? 1 : 0, 1);
    // Unprivileged PPB read (no region, no background) -> DACCVIOL.
    volatile unsigned int t = MPU_TYPE;
    (void)t;
    check("UPRIV-PPB", (SCR_COUNT == 4 && SCR_FSRS[3] == 0x82 && SCR_FARS[3] == 0xE000ED90) ? 1 : 0, 1);
    // Unprivileged scratch write (FULL region) passes.
    *(volatile unsigned int *)0x20001200 = 0x55;
    if (*(volatile unsigned int *)0x20001200 != 0x55) {
        uart_puts("UPRIV-FULL FAIL\r\n");
        SCR_FAILS++;
    } else {
        uart_puts("UPRIV-FULL OK\r\n");
    }
    // Back to privileged MSP via SVC (handler clears nPRIV, keeps SPSEL;
    // the MSP restore below snaps r13 back — see note above).
    __asm__ volatile ("svc #0");
    __asm__ volatile ("msr msp, %0" :: "r"(saved_msp));
    __asm__ volatile ("movs r0, #0\n\tmsr control, r0\n\tisb" ::: "r0");

    if (SCR_FAILS == 0) uart_puts("MPU all PASS\r\n");
    uart_puts("MPU done\r\n");
    for (;;);
}
