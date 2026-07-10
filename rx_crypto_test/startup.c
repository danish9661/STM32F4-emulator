extern int main(void);
extern void _start(void);

#define USART1_DR  (*(volatile unsigned int *)0x40011004)
#define USART1_SR  (*(volatile unsigned int *)0x40011000)

#define CRC_DR  (*(volatile unsigned int *)0x40023000)

volatile unsigned int rx_byte = 0;
volatile int rx_flag = 0;
volatile int rx_interrupt_fired = 0;

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

void USART1_IRQHandler(void) {
    unsigned char c = USART1_DR;
    rx_byte = c;
    rx_flag = 1;
    rx_interrupt_fired = 1;
    CRC_DR = c;
}

#define DH Default_Handler
#define U1 USART1_IRQHandler

__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    // System exceptions (16 entries)
    (void (*)(void))0x20020000, // [0] SP
    _start,                     // [1] Reset
    DH, DH, DH, DH, DH,         // [2-6] NMI..UsageFault
    DH, 0, 0, 0, 0,            // [7-11] Reserved, Reserved, Reserved, Reserved, SVC
    DH, DH, 0,                  // [12-14] DebugMon, Reserved, PendSV
    DH,                         // [15] SysTick
    // IRQ 0-37 (38 entries)
    DH,DH,DH,DH,DH,DH,DH,DH,  // 0-7
    DH,DH,DH,DH,DH,DH,DH,DH,  // 8-15
    DH,DH,DH,DH,DH,DH,DH,DH,  // 16-23
    DH,DH,DH,DH,DH,DH,DH,DH,  // 24-31
    DH,DH,DH,DH,DH,DH,        // 32-37
    // IRQ 38: USART1
    U1,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
