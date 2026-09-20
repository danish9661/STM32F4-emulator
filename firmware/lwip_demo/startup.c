extern int main(void);
extern void _start(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

void ETH_IRQHandler(void);
void ETH_WKUP_IRQHandler(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

// IRQ 61 = ETH = index 77, IRQ 62 = ETH_WKUP = index 78 (16 system + irq)
__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    (void (*)(void))STACK_TOP,
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
    [78] = ETH_WKUP_IRQHandler,
    [79 ... 112] = Default_Handler,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        // Copy .data (flash -> RAM) and zero .bss (real LwIP needs it).
        "ldr r0, =_sdata\n"
        "ldr r1, =_edata\n"
        "ldr r2, =_sidata\n"
        "1: cmp r0, r1\n"
        "bcs 2f\n"
        "ldr r3, [r2], #4\n"
        "str r3, [r0], #4\n"
        "b 1b\n"
        "2: ldr r0, =_sbss\n"
        "ldr r1, =_ebss\n"
        "movs r2, #0\n"
        "3: cmp r0, r1\n"
        "bcs 4f\n"
        "str r2, [r0], #4\n"
        "b 3b\n"
        "4: ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
