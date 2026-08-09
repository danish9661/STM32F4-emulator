void _start(void);
void EXTI0_IRQHandler(void);

__attribute__((used, section(".vectors")))
void (* const vector_table[23])(void) = {
    [0] = (void (*)(void))0x20020000, // initial SP
    [1] = _start,                     // reset
    [22] = EXTI0_IRQHandler,          // IRQ6 = EXTI0 (slot 16+6)
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
