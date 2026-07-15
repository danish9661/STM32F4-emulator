extern int main(void);
extern void _start(void);

void ETH_IRQHandler(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

// IRQ 61 = ETH = index 77 in vector table (16 system + 61)
__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    (void (*)(void))0x20020000,
    _start,
    [2]  = Default_Handler, [3]  = Default_Handler,
    [4]  = Default_Handler, [5]  = Default_Handler,
    [6]  = Default_Handler, [7]  = Default_Handler,
    [8]  = Default_Handler, [9]  = Default_Handler,
    [10] = Default_Handler, [11] = Default_Handler,
    [12] = Default_Handler, [13] = Default_Handler,
    [14] = Default_Handler, [15] = Default_Handler,
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
