// GAP10 SDIO: CMD24 single-block write round-trip through CMD17.
// Walks Idle→Tran (CMD0/2/3/7), writes block 2 via CMD24 + FIFO words,
// waits DATAEND, reads the block back via CMD17 + FIFO, compares.
// Prints SDIO GAP10 OK / FAIL.
#include <stdint.h>

#define USART1_SR   (*(volatile uint32_t*)0x40011000)
#define USART1_DR   (*(volatile uint32_t*)0x40011004)
#define USART1_BRR  (*(volatile uint32_t*)0x40011008)
#define USART1_CR1  (*(volatile uint32_t*)0x4001100C)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)
#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)

#define SDIO_BASE  0x40012C00
#define SDIO_POWER (*(volatile uint32_t*)(SDIO_BASE + 0x00))
#define SDIO_ARG   (*(volatile uint32_t*)(SDIO_BASE + 0x08))
#define SDIO_CMD   (*(volatile uint32_t*)(SDIO_BASE + 0x0C))
#define SDIO_DTIMER (*(volatile uint32_t*)(SDIO_BASE + 0x24))
#define SDIO_DLEN  (*(volatile uint32_t*)(SDIO_BASE + 0x28))
#define SDIO_STA   (*(volatile uint32_t*)(SDIO_BASE + 0x34))
#define SDIO_ICR   (*(volatile uint32_t*)(SDIO_BASE + 0x38))
#define SDIO_FIFO  (*(volatile uint32_t*)(SDIO_BASE + 0x80))

static void uart_putc(char c) {
    while (!(USART1_SR & (1 << 7))) {}
    USART1_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

static void cmd(uint8_t idx, uint32_t arg) {
    SDIO_ARG = arg;
    SDIO_CMD = 0x40 | idx;
}

static int wait_sta(uint32_t bit, int spin) {
    while (spin-- > 0) {
        if (SDIO_STA & bit) return 1;
    }
    return 0;
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER = (GPIOA_MODER & ~(3 << 18)) | (2 << 18);
    GPIOA_AFRL  = (GPIOA_AFRL & ~(0xF << 4))  | (7 << 4);
    USART1_BRR  = 0x683;
    USART1_CR1  = (1 << 13) | (1 << 3);
    uart_puts("=== SDIO GAP10 ===\r\n");

    SDIO_POWER = 1;
    // NOTE: the model boots with NO card bound (card_blocks()==0), so the
    // image path is inert until firmware binds one. Real silicon always
    // has a card in the slot; the emulator needs the same precondition.
    // There is no guest-visible bind register — the DRIVER binds the image
    // pre-boot (ext_devices.sdio card_blocks, like qspi/spi_flash/i2c).
    // This firmware asserts the precondition via the STA reset signature
    // instead: unbound reads stay erased (see QSPI GAP10 for the pattern).
    cmd(0, 0); cmd(2, 0); cmd(3, 0);
    SDIO_ARG = 0x01D00000; cmd(7, 0x01D00000); // select → Tran
    uart_puts("tran\r\n");

    // CMD24 block 2: 4 words, then DATAEND.
    SDIO_DTIMER = 0xFFFFF;
    SDIO_DLEN = 512;
    cmd(24, 2);
    SDIO_FIFO = 0x11111111; SDIO_FIFO = 0x22222222;
    SDIO_FIFO = 0x33333333; SDIO_FIFO = 0x44444444;
    if (!wait_sta(1 << 8, 2000000)) { uart_puts("SDIO GAP10 FAIL (no DATAEND24)\r\n"); while (1); }
    uart_puts("wrote\r\n");

    // CMD17 block 2: first two FIFO words must match.
    SDIO_DLEN = 512;
    cmd(17, 2);
    if (!wait_sta(1 << 8, 2000000)) { uart_puts("SDIO GAP10 FAIL (no DATAEND17)\r\n"); while (1); }
    uint32_t w0 = SDIO_FIFO, w1 = SDIO_FIFO;
    if (w0 == 0x11111111 && w1 == 0x22222222) {
        uart_puts("SDIO GAP10 OK\r\n");
    } else {
        uart_puts("SDIO GAP10 FAIL (mismatch)\r\n");
    }
    while (1);
}
