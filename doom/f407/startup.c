// Startup: vector table + _start. Doom uses no interrupts (input comes via
// the SRAM key ring), so every IRQ slot is Default_Handler.
extern int main(void);
extern void _start(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

__attribute__((used, section(".vectors")))
void (* const vector_table[107])(void) = {
    (void (*)(void))0x20020000,   // initial SP (top of 128K SRAM)
    _start,
    [2 ... 15] = Default_Handler, // 0x08..0x3C system exceptions
    [16 ... 106] = Default_Handler, // 0x40..0x1A8 external IRQs
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl __do_init\n"
        "bl main\n"
        "1: b 1b\n"
    );
}

// Zero .bss and copy .data (LMA -> VMA) before main. Not strictly required on
// a fresh emulator instance (memory is pre-zeroed) but correct.
void __do_init(void) {
    extern char __bss_start__[], __bss_end__[];
    extern char __data_start__[], __data_end__[], __data_load__[];
    char *p;
    for (p = __bss_start__; p < __bss_end__; p++) *p = 0;
    for (p = __data_start__; p < __data_end__; p++) *p = *(char *)(__data_load__ + (p - __data_start__));
}
