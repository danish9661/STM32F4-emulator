#include "FreeRTOS.h"
#include "task.h"
#include "semphr.h"
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

/* Counts how many times vHighTask ran.  vHighTask only runs after the TIM3
 * ISR gives xTimSem, so g_high_count > 0 proves the full
 * ISR -> xSemaphoreGiveFromISR -> portYIELD_FROM_ISR (PendSV) -> scheduler
 * context-switch path worked. */
static volatile uint32_t g_high_count __attribute__((section(".testvars"))) = 0;

static SemaphoreHandle_t xTimSem = NULL;

/* TIM2 update ISR: not enabled by the counter test, but must exist for the
 * vector table. Clears UIF so a stray update can't re-pend. */
__attribute__((interrupt)) void TIM2_IRQHandler(void) {
    g_tim2_isr++;
    *(volatile uint32_t *)(TIM2_BASE + 0x10) = 0;  /* clear UIF (SR) */
}

/* TIM3 update ISR: proves external NVIC IRQ delivery AND the ISR->semaphore
 * ->context-switch path.  On each update it clears UIF, then gives a binary
 * semaphore a higher-priority task pends on; portYIELD_FROM_ISR pends PendSV
 * (SCB ICSR PENDSVSET), and the gated-RET NVIC path switches to vHighTask.
 * It then SELF-DISABLES (deferred-interrupt pattern): giving the semaphore
 * while it is already "given" (a second overflow before vHighTask consumes it)
 * would corrupt FreeRTOS's event list.  vHighTask re-arms TIM3 after taking. */
__attribute__((interrupt)) void TIM3_IRQHandler(void) {
    g_tim3_isr++;
    *(volatile uint32_t *)(TIM3_BASE + 0x10) &= ~1u;     /* clear UIF (SR) */
    BaseType_t hp = pdFALSE;
    xSemaphoreGiveFromISR(xTimSem, &hp);
    portYIELD_FROM_ISR(hp);
    *(volatile uint32_t *)(TIM3_BASE + 0x0C) = 0;        /* disable UIE (DIER) */
    NVIC_ICER0 |= (1u << 29);                            /* mask TIM3 until task re-arms */
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

    /* TIM3: ISR gives xTimSem (which vHighTask pends on), exercising the full
     * ISR -> xSemaphoreGiveFromISR -> portYIELD_FROM_ISR (PendSV) -> context
     * switch path.  ARR=5000 overflows ~once per emulated step (no storm).
     * Spin until both the ISR fired and vHighTask was dispatched (capped so a
     * model bug can't hang the firmware). */
    RCC_APB1ENR |= (1u << 1);                        /* TIM3 clock enable */
    *(volatile uint32_t *)(TIM3_BASE + 0x28) = 0;   /* PSC = 0 */
    *(volatile uint32_t *)(TIM3_BASE + 0x2C) = 5000;/* ARR: ~1 overflow/step */
    *(volatile uint32_t *)(TIM3_BASE + 0x0C) = 1;   /* DIER UIE */
    *(volatile uint32_t *)(TIM3_BASE + 0x00) = 1;   /* CR1 CEN */
    NVIC_ISER0 |= (1u << 29);                        /* enable TIM3 IRQ (29) */
    volatile uint32_t spins = 0;
    while ((g_tim3_isr == 0 || g_high_count == 0) && spins < 5000000) spins++;
    uart_puts("TIM3 isr ");
    print_int((int)g_tim3_isr);
    uart_puts(" high ");
    print_int((int)g_high_count);
    uart_puts("\n");

    uart_puts((c1 > c0 && g_tim3_isr > 0 && g_high_count > 0) ? "TIM TEST PASS\n" : "TIM TEST FAIL\n");
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

/* Highest-priority task: pends on xTimSem, which the TIM3 ISR gives.  Runs only
 * after an ISR-driven context switch (preemption), proving the NVIC -> PendSV
 * -> scheduler path.  After consuming the semaphore it re-arms TIM3 (deferred
 * interrupt) so the next overflow re-triggers the ISR. */
void vHighTask(void *p) {
    (void)p;
    uart_puts("Hhigh start\n");
    for (;;) {
        uart_puts("Hbefore\n");
        xSemaphoreTake(xTimSem, portMAX_DELAY);
        uart_puts("Hafter\n");
        g_high_count++;
        RCC_APB1ENR |= (1u << 1);                        /* TIM3 clock */
        *(volatile uint32_t *)(TIM3_BASE + 0x0C) = 1;    /* DIER UIE */
        NVIC_ISER0 |= (1u << 29);                        /* unmask TIM3 IRQ */
    }
}

int main(void) {
    uart_puts("FreeRTOS boot\n");
    xTimSem = xSemaphoreCreateBinary();
    if (xTimSem == NULL) uart_puts("sem fail\n");
    if (xTaskCreate(vHighTask, "HIGH", 1024, NULL, 3, NULL) != pdPASS)
        uart_puts("create HIGH fail\n");
    if (xTaskCreate(vTask1, "T1", 512, NULL, 2, NULL) != pdPASS)
        uart_puts("create T1 fail\n");
    if (xTaskCreate(vTask2, "T2", 512, NULL, 1, NULL) != pdPASS)
        uart_puts("create T2 fail\n");
    uart_puts("start scheduler\n");
    vTaskStartScheduler();
    uart_puts("scheduler returned\n");
    for (;;) ;
}
