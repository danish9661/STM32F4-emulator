#include "FreeRTOS.h"
#include "task.h"
#include <stdint.h>

#define USART1_BASE 0x40011000

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

void vTask1(void *p) {
    (void)p;
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
