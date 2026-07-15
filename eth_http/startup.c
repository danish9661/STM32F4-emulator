extern int main(void);
extern void _start(void);

void ETH_IRQHandler(void);
void SysTick_Handler(void) {}  // systick is polled, not interrupt-driven

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    (void (*)(void))0x20020000,
    _start,
    [2 ... 14] = Default_Handler,
    [15] = SysTick_Handler,
    [16 ... 76] = Default_Handler,
    [77] = ETH_IRQHandler,
    [78 ... 112] = Default_Handler,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
