extern int main(void);
extern void _start(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)
extern void vPortSVCHandler(void);
extern void xPortPendSVHandler(void);
extern void xPortSysTickHandler(void);
extern void TIM2_IRQHandler(void);
extern void TIM3_IRQHandler(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[46])(void) = {
    (void (*)(void))STACK_TOP,
    _start,
    [2 ... 10] = Default_Handler,
    vPortSVCHandler,            /* 11  SVC       -> FreeRTOS */
    [12] = Default_Handler,    /* 12  DebugMon  */
    [13] = Default_Handler,    /* 13  reserved  */
    xPortPendSVHandler,         /* 14  PendSV    -> FreeRTOS */
    xPortSysTickHandler,        /* 15  SysTick   -> FreeRTOS */
    [16 ... 45] = Default_Handler,
    [44] = TIM2_IRQHandler,     /* 44  TIM2  (IRQ 28) */
    [45] = TIM3_IRQHandler,     /* 45  TIM3  (IRQ 29) */
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
