// Watchdog (IWDG) demo: starts the independent watchdog, pets it while "alive",
// then stops petting so it expires and resets the MCU. On reboot the firmware
// reads RCC->CSR to detect the IWDG reset cause. Exercises the emulator's
// watchdog model + auto-reboot path end-to-end.
// Expect:
//   === Watchdog Demo ===
//   IWDG started (pet every 150ms, ~1s timeout)
//   alive 0 .. alive 4
//   stopping pet -> watchdog should reset
//   (reset) === Watchdog Demo ===
//   IWDG reset detected
#define IWDG_BASE 0x40003000
#define IWDG_KR  (*(volatile unsigned int *)(IWDG_BASE + 0x00))
#define IWDG_PR  (*(volatile unsigned int *)(IWDG_BASE + 0x04))
#define IWDG_RLR (*(volatile unsigned int *)(IWDG_BASE + 0x08))

#define RCC_BASE 0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))
#define RCC_CSR  (*(volatile unsigned int *)(RCC_BASE + 0x74))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    RCC_APB2ENR |= (1 << 4); // USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void delay_ms(int ms) {
    for (volatile int i = 0; i < ms * 4000; i++);
}

int main(void) {
    uart_init();
    uart_puts("=== Watchdog Demo ===\r\n");

    unsigned int csr = RCC_CSR;
    if (csr & (1u << 29)) {       // IWDGRSTF
        uart_puts("IWDG reset detected\r\n");
        RCC_CSR = (1u << 24);     // RMVF: clear reset-status flags
    }
    if (csr & (1u << 30)) {       // WWDGRSTF
        uart_puts("WWDG reset detected\r\n");
        RCC_CSR = (1u << 24);
    }

    // Start IWDG: prescaler /8 (PR=1), max reload (0xFFF) -> ~1s timeout
    // (model: tick = 128 * prescaler instructions; 0xFFF * 1024 ~= 4.2M inst).
    IWDG_KR = 0x5555;  // unlock register access
    IWDG_PR = 1;       // /8
    IWDG_RLR = 0xFFF;  // max reload
    IWDG_KR = 0xCCCC;  // start the watchdog

    uart_puts("IWDG started (pet every 150ms, ~1s timeout)\r\n");
    for (int i = 0; i < 5; i++) {
        IWDG_KR = 0xAAAA;  // reload (pet) the counter
        delay_ms(150);
        uart_puts("alive ");
        uart_putchar('0' + i);
        uart_puts("\r\n");
    }
    uart_puts("stopping pet -> watchdog should reset\r\n");
    while (1) { delay_ms(200); }
}
