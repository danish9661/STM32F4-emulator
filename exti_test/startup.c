void _start(void);
void EXTI0_IRQHandler(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

__attribute__((used, section(".vectors")))
void (* const vector_table[23])(void) = {
    [0] = (void (*)(void))STACK_TOP, // initial SP
    [1] = _start,                     // reset
    [22] = EXTI0_IRQHandler,          // IRQ6 = EXTI0 (slot 16+6)
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
