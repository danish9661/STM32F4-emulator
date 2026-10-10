// GAP10 SDIO: full-init CMD17 -> CMD24 -> CMD17 round-trip through the
// bound card image (parent-e2e order: a pre-read's DATAEND is sticky, so
// the write wait must genuinely reach the completion window — it only
// does when no completion flag is pre-set at arm time).
// Walks Idle→Tran the hardware way (CMD0/8, CMD55+ACMD41 OCR poll, CMD2/
// 3/7, CMD16 block length), reads block 0 erased via CMD17, writes 128
// words via CMD24, reads the block back via CMD17, compares.
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
#define SDIO_RESP0 (*(volatile uint32_t*)(SDIO_BASE + 0x14))
#define SDIO_DTIMER (*(volatile uint32_t*)(SDIO_BASE + 0x24))
#define SDIO_DLEN  (*(volatile uint32_t*)(SDIO_BASE + 0x28))
#define SDIO_DCOUNT (*(volatile uint32_t*)(SDIO_BASE + 0x30))
#define SDIO_STA   (*(volatile uint32_t*)(SDIO_BASE + 0x34))
#define SDIO_ICR   (*(volatile uint32_t*)(SDIO_BASE + 0x38))
#define SDIO_FIFO  (*(volatile uint32_t*)(SDIO_BASE + 0x80))

#define STA_CCRCFAIL (1u << 0)
#define STA_DCRCFAIL (1u << 1)
#define STA_CTIMEOUT (1u << 2)
#define STA_DTIMEOUT (1u << 3)
#define STA_TXUNDERR (1u << 4)
#define STA_RXOVERR  (1u << 5)
#define STA_CMDREND  (1u << 6)
#define STA_DATAEND  (1u << 8)
#define STA_DBCKEND  (1u << 10)

static void uart_putc(char c) {
    while (!(USART1_SR & (1 << 7))) {}
    USART1_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putc(*s++); }

static void cmd(uint8_t idx, uint32_t arg) {
    SDIO_ARG = arg;
    SDIO_CMD = 0x40 | idx;
}

static void fail(const char *msg) {
    uart_puts("SDIO GAP10 FAIL (");
    uart_puts(msg);
    uart_puts(")\r\n");
    while (1);
}

static void wait_dataend(const char *who) {
    int spin = 2000000;
    while (spin-- > 0) {
        uint32_t sta = SDIO_STA;
        if (sta & (STA_DCRCFAIL | STA_DTIMEOUT | STA_RXOVERR | STA_TXUNDERR))
            fail(who);
        if (sta & STA_DATAEND)
            return;
    }
    fail(who);
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
    // image path is inert until the driver binds one. Real silicon always
    // has a card in the slot; the emulator needs the same precondition.
    // There is no guest-visible bind register — the DRIVER binds the image
    // pre-boot (ext_devices.sdio card_blocks, like qspi/spi_flash/i2c).
    cmd(0, 0);
    cmd(8, 0x1AA);
    if (SDIO_RESP0 != 0x1AA) fail("cmd8");
    // ACMD41 OCR poll (CMD55 prefix each round, like every SD driver).
    for (int i = 0; i < 8; i++) {
        cmd(55, 0);
        cmd(41, 0x80100000);
        if (SDIO_RESP0 & (1u << 31)) break;
        if (i == 7) fail("acmd41");
    }
    cmd(2, 0); cmd(3, 0);
    SDIO_ARG = 0x01D00000; cmd(7, 0x01D00000); // select → Tran
    cmd(16, 512);
    uart_puts("tran\r\n");

    // CMD17 block 0: must read erased (proves the bind + read path).
    SDIO_ICR = 0xFFFFFFFF;
    SDIO_DTIMER = 0xFFFFFF;
    SDIO_DLEN = 512;
    cmd(17, 0);
    wait_dataend("no DATAEND17a");
    for (int i = 0; i < 128; i++) {
        if (SDIO_FIFO != 0xFFFFFFFF) fail("not erased");
    }
    uart_puts("erased\r\n");

    // CMD24 block 0: 128 words. No completion flag may be pre-set at arm
    // (a pre-set DBCKEND/DATAEND would end the wait before the commit
    // window — the write would vanish while flags look normal).
    SDIO_ICR = 0xFFFFFFFF;
    SDIO_DTIMER = 0xFFFFFF;
    SDIO_DLEN = 512;
    cmd(24, 0);
    if (SDIO_STA & (STA_DATAEND | STA_DBCKEND)) fail("preset@arm");
    if (SDIO_STA & (STA_DCRCFAIL | STA_DTIMEOUT)) fail("err@arm");
    if (SDIO_DCOUNT != 512) fail("dcount@arm");
    for (int i = 0; i < 128; i++) {
        uint32_t w = (i == 0) ? 0xA5A5A5A5u
            : (i == 1) ? 0x7F7F7FDAu : (0xA5000000u + (uint32_t)i);
        SDIO_FIFO = w;
    }
    if (SDIO_DCOUNT != 0) fail("dcount@staged");
    wait_dataend("no DATAEND24");
    uart_puts("wrote\r\n");

    // CMD17 block 0: the staged words must come back.
    SDIO_ICR = 0xFFFFFFFF;
    SDIO_DLEN = 512;
    cmd(17, 0);
    wait_dataend("no DATAEND17b");
    for (int i = 0; i < 128; i++) {
        uint32_t want = (i == 0) ? 0xA5A5A5A5u
            : (i == 1) ? 0x7F7F7FDAu : (0xA5000000u + (uint32_t)i);
        if (SDIO_FIFO != want) fail("mismatch");
    }
    uart_puts("SDIO GAP10 OK\r\n");
    while (1);
}
