void _start(void);
void DMA2D_IRQHandler(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20040000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[16 + 72])(void) = {
    (void (*)(void))STACK_TOP, // [0] SP
    _start,                     // [1] Reset
    [2 ... 15] = Default_Handler,
    [16 ... 71] = Default_Handler,
    [72] = DMA2D_IRQHandler,    // IRQ56 = DMA2D (slot 16+56)
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
