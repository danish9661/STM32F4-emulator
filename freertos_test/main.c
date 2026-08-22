#include "FreeRTOS.h"
#include "task.h"
#include <stdint.h>

#define USART1_BASE 0x40011000

#define RCC_APB1ENR   (*(volatile uint32_t *)0x40023840UL)
#define NVIC_ISER0    (*(volatile uint32_t *)0xE000E100UL)
#define NVIC_ICER0    (*(volatile uint32_t *)0xE000E180UL)

#define TIM2_BASE 0x40000000UL
#define TIM3_BASE 0x40000400UL

static inline void uart_putc(char c) {
    volatile uint32_t *sr = (volatile uint32_t *)USART1_BASE;
    volatile uint32_t *dr = (volatile uint32_t *)(USART1_BASE + 4);
    while (!(*sr & 0x80U));  /* TXE */
    *dr = (uint8_t)c;
    while (!(*sr & 0x40U));  /* TC */
}

static void uart_puts(const char *s) {
    while (*s) {
        if (*s == '\n') uart_putc('\r');
        uart_putc(*s++);
    }
}

static void print_int(int v) {
    char buf[12];
    int i = 0;
    if (v < 0) { uart_putc('-'); v = -v; }
    if (v == 0) buf[i++] = '0';
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putc(buf[--i]);
}

/* Update-interrupt counters, incremented from the firmware ISRs.
 * Placed in .testvars (high RAM) so they never alias FreeRTOS kernel
 * statics that the linker puts in the low .bss (0x20000000+). */
static volatile uint32_t g_tim2_isr __attribute__((section(".testvars"))) = 0;
static volatile uint32_t g_tim3_isr __attribute__((section(".testvars"))) = 0;

/* TIM2 update ISR: not enabled by the counter test, but must exist for the
 * vector table. Clears UIF so a stray update can't re-pend. */
__attribute__((interrupt)) void TIM2_IRQHandler(void) {
    g_tim2_isr++;
    *(volatile uint32_t *)(TIM2_BASE + 0x10) = 0;  /* clear UIF (SR) */
}

/* TIM3 update ISR: proves external NVIC IRQ delivery end-to-end.
 * Self-disables after the first delivery: TIM3 has the highest default NVIC
 * priority and (with ARR=50) would otherwise overflow ~twice per emulated step,
 * storming the interrupt pump and starving the SysTick so vTaskDelay could
 * never complete.  One confirmed ISR is enough to prove delivery. */
__attribute__((interrupt)) void TIM3_IRQHandler(void) {
    g_tim3_isr++;
    *(volatile uint32_t *)(TIM3_BASE + 0x10) = 0;        /* clear UIF (SR) */
    *(volatile uint32_t *)(TIM3_BASE + 0x0C) = 0;        /* disable UIE (DIER) */
    NVIC_ICER0 |= (1u << 29);                            /* mask TIM3 IRQ in NVIC */
}

/* Exercises the TIM models + external NVIC interrupt delivery.
 * - TIM2 (IRQ 28): free-running counter must advance over time.
 * - TIM3 (IRQ 29): update interrupt must be delivered by the NVIC.
 * Uses busy-wait loops (not vTaskDelay): run_tim_tests() is the first thing
 * vTask1 does, and the first external ISR (TIM3) can fire during a vTaskDelay
 * here and corrupt this task's delay-list entry, making the delay return
 * immediately.  A busy-wait lets the emulated timers advance and the ISR fire
 * without touching the scheduler's delay lists. */
static void run_tim_tests(void) {
    /* TIM2: counter advance (no interrupt) */
    RCC_APB1ENR |= (1u << 0);                        /* TIM2 clock enable */
    *(volatile uint32_t *)(TIM2_BASE + 0x28) = 0;   /* PSC = 0 */
    *(volatile uint32_t *)(TIM2_BASE + 0x2C) = 0x100000; /* ARR large */
    *(volatile uint32_t *)(TIM2_BASE + 0x00) = 1;   /* CR1 CEN */
    uint32_t c0 = *(volatile uint32_t *)(TIM2_BASE + 0x24);
    for (volatile uint32_t i = 0; i < 1000000; i++) { }   /* let TIM2 advance */
    uint32_t c1 = *(volatile uint32_t *)(TIM2_BASE + 0x24);
    uart_puts("TIM2 adv ");
    print_int((int)c0);
    uart_puts("->");
    print_int((int)c1);
    uart_puts("\n");

    /* TIM3: update interrupt delivery through the NVIC.  The ISR self-disables
     * after one delivery (see TIM3_IRQHandler), so we just spin until it
     * increments g_tim3_isr (capped so a model bug can't hang the firmware). */
    RCC_APB1ENR |= (1u << 1);                        /* TIM3 clock enable */
    *(volatile uint32_t *)(TIM3_BASE + 0x28) = 0;   /* PSC = 0 */
    *(volatile uint32_t *)(TIM3_BASE + 0x2C) = 50;  /* ARR small -> fast wrap */
    *(volatile uint32_t *)(TIM3_BASE + 0x0C) = 1;   /* DIER UIE */
    *(volatile uint32_t *)(TIM3_BASE + 0x00) = 1;   /* CR1 CEN */
    NVIC_ISER0 |= (1u << 29);                        /* enable TIM3 IRQ (29) */
    volatile uint32_t spins = 0;
    while (g_tim3_isr == 0 && spins < 5000000) spins++;
    uart_puts("TIM3 isr ");
    print_int((int)g_tim3_isr);
    uart_puts("\n");

    uart_puts((c1 > c0 && g_tim3_isr > 0) ? "TIM TEST PASS\n" : "TIM TEST FAIL\n");
}

void vTask1(void *p) {
    (void)p;
    run_tim_tests();
    int n = 0;
    for (;;) {
        uart_puts("TASK1 n=");
        print_int(n);
        uart_puts(" tick=");
        print_int((int)xTaskGetTickCount());
        uart_puts("\n");
        n++;
        vTaskDelay(pdMS_TO_TICKS(30));
    }
}

void vTask2(void *p) {
    (void)p;
    int n = 0;
    for (;;) {
        uart_puts("TASK2 n=");
        print_int(n);
        uart_puts(" tick=");
        print_int((int)xTaskGetTickCount());
        uart_puts("\n");
        n++;
        vTaskDelay(pdMS_TO_TICKS(70));
    }
}

int main(void) {
    uart_puts("FreeRTOS boot\n");
    if (xTaskCreate(vTask1, "T1", 256, NULL, 2, NULL) != pdPASS)
        uart_puts("create T1 fail\n");
    if (xTaskCreate(vTask2, "T2", 256, NULL, 1, NULL) != pdPASS)
        uart_puts("create T2 fail\n");
    uart_puts("start scheduler\n");
    vTaskStartScheduler();
    uart_puts("scheduler returned\n");
    for (;;) ;
}
