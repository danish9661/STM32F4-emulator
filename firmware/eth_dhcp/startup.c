extern int main(void);
extern void _start(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

void ETH_IRQHandler(void);
void SysTick_Handler(void) {}  // systick is polled, not interrupt-driven

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    (void (*)(void))STACK_TOP,
    _start,
    [2 ... 14] = Default_Handler,
    [15] = SysTick_Handler,
    [16 ... 76] = Default_Handler,
    [77] = ETH_IRQHandler,
    [78 ... 112] = Default_Handler,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
