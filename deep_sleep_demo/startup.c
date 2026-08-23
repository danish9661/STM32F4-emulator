extern int main(void);
extern void _start(void);
extern void RTC_Alarm_IRQHandler(void);

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

// Full Cortex-M4 vector table. RTC_Alarm is IRQ 41 -> index 16 + 41 = 57.
__attribute__((used, section(".vectors")))
void (* const vector_table[90])(void) = {
    (void (*)(void))0x20020000,
    _start,
    [2 ... 56] = Default_Handler,
    [57] = RTC_Alarm_IRQHandler,     /* IRQ 41  RTC_Alarm */
    [58 ... 89] = Default_Handler,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
