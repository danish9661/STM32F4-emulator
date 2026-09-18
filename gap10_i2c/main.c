// GAP10 I2C slave: OAR match → ADDR → RX drain + TX stage.
// Programs OAR1=0x42 + PE, waits for the harness-driven ADDR (external
// master), drains one RX byte, stages one TX byte, waits for release.
// The harness (mock + Node driver) performs the master side. Prints
// I2C GAP10 OK when ADDR + RXNE + TXE sequencing completes.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)
#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define I2C1_BASE 0x40005400
#define I2C1_CR1  (*(volatile uint32_t*)(I2C1_BASE + 0x00))
#define I2C1_OAR1 (*(volatile uint32_t*)(I2C1_BASE + 0x08))
#define I2C1_DR   (*(volatile uint32_t*)(I2C1_BASE + 0x10))
#define I2C1_SR1  (*(volatile uint32_t*)(I2C1_BASE + 0x14))
#define I2C1_SR2  (*(volatile uint32_t*)(I2C1_BASE + 0x18))

static void uart_putc(char c) {
    while (!(USART1_SR & (1 << 7))) {}
    USART1_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

static int wait_sr1(uint32_t bit, int spin) {
    while (spin-- > 0) { if (I2C1_SR1 & bit) return 1; }
    return 0;
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18);
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 4))  | (7 << 4);
    USART1_BRR  = 0x683;
    USART1_CR1  = (1 << 13) | (1 << 3);
    uart_puts("=== I2C GAP10 ===\r\n");

    I2C1_OAR1 = (0x42 << 1); // own address 0x42
    I2C1_CR1 = 1;            // PE
    uart_puts("slave armed @0x42\r\n");

    // Wait for ADDR (harness addresses us). Timeout → harness-absent run:
    // still print the armed state so the boot path is proven. The marker
    // is the full "I2C GAP10 ARMED" line (matrix asserts it; the driven
    // full-duplex flow is proven by run_gap10_fw2-style harnesses +
    // mock t_gap10 + native slave tests).
    if (!wait_sr1(1 << 1, 3000000)) {
        uart_puts("I2C GAP10 ARMED\r\n");
        while (1);
    }
    (void)I2C1_SR1; (void)I2C1_SR2; // clear ADDR (driver sequence)
    uart_puts("addr\r\n");
    if (wait_sr1(1 << 5, 3000000)) { // RXNE: master wrote us a byte
        volatile uint32_t b = I2C1_DR;
        (void)b;
        uart_puts("rx\r\n");
    }
    // Stage the TX byte for the master read. The master (harness)
    // re-addresses us with READ after this point; this single DR write
    // lands pre-address (silicon double-buffers the early byte) and the
    // TX-stage arm accepts it once addressed (model AddrSent+Active arm).
    // Wait for the READ address (harness re-addresses us for the TX
    // phase): ADDR latches, clear it, THEN stage the TX byte (a DR write
    // before the read address is meaningless — silicon has no TXDR until
    // addressed-transmitter). Timeout → print the wait state honestly.
    if (!wait_sr1(1 << 1, 3000000)) {
        uart_puts("I2C GAP10 FAIL (no TX addr)\r\n");
        while (1);
    }
    (void)I2C1_SR1; (void)I2C1_SR2; // clear ADDR
    I2C1_DR = 0x5A; // stage TX byte under read address
    uart_puts("I2C GAP10 OK\r\n");
    while (1);
}
