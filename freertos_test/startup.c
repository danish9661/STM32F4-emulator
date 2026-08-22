extern int main(void);
extern void _start(void);
extern void vPortSVCHandler(void);
extern void xPortPendSVHandler(void);
extern void xPortSysTickHandler(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[16])(void) = {
    (void (*)(void))0x20020000,
    _start,
    [2 ... 10] = Default_Handler,
    vPortSVCHandler,            /* 11  SVC       -> FreeRTOS */
    [12] = Default_Handler,    /* 12  DebugMon  */
    [13] = Default_Handler,    /* 13  reserved  */
    xPortPendSVHandler,         /* 14  PendSV    -> FreeRTOS */
    xPortSysTickHandler,        /* 15  SysTick   -> FreeRTOS */
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
