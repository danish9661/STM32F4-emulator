void _start(void);
void USART1_IRQHandler(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[16 + 38])(void) = {
    (void (*)(void))STACK_TOP, // [0] SP
    _start,                     // [1] Reset
    [2 ... 15] = Default_Handler,
    [16 ... 52] = Default_Handler,
    [53] = USART1_IRQHandler,   // IRQ37 = USART1 (slot 16+37)
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
