#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))
#define GPIOB_BASE  0x40020400
#define GPIOB_MODER (*(volatile unsigned int *)(GPIOB_BASE + 0x00))
#define GPIOB_AFRL  (*(volatile unsigned int *)(GPIOB_BASE + 0x20))
#define GPIO_BSRR(base) (*(volatile unsigned int *)((base) + 0x18))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define SPI3_BASE   0x40003C00
#define SPI3_CR1    (*(volatile unsigned int *)(SPI3_BASE + 0x00))
#define SPI3_SR     (*(volatile unsigned int *)(SPI3_BASE + 0x08))
#define SPI3_DR     (*(volatile unsigned int *)(SPI3_BASE + 0x0C))

#define SPI_CR1_MSTR   (1 << 2)
#define SPI_CR1_BR     (7 << 3)
#define SPI_CR1_SPE    (1 << 6)
#define SPI_CR1_SSI    (1 << 8)
#define SPI_CR1_SSM    (1 << 9)
#define SPI_SR_RXNE   (1 << 0)
#define SPI_SR_TXE    (1 << 1)

static int pass_count = 0;
static int fail_count = 0;

static void uart_init(void) {
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER &= ~((3 << 18) | (3 << 20));
    GPIOA_MODER |=  ((2 << 18) | (2 << 20));
    GPIOA_AFRH  |=  ((7 << 4) | (7 << 8));
    USART_CR1 = 0;
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_puts(const char *s) {
    while (*s) {
        while (!(USART_SR & (1 << 7)));
        USART_DR = *s++;
    }
}

static void uart_puthex(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7)));
        USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

static void test(const char *name, int cond) {
    uart_puts("  ");
    uart_puts(cond ? "PASS" : "FAIL");
    uart_puts(" ");
    uart_puts(name);
    uart_puts("\r\n");
    if (cond) pass_count++; else fail_count++;
}

static void cs_low(void) { GPIO_BSRR(GPIOB_BASE) = (1 << (12 + 16)); } // PB12 reset
static void cs_high(void) { GPIO_BSRR(GPIOB_BASE) = (1 << 12); }      // PB12 set

static unsigned char spi3_xfer(unsigned char byte) {
    SPI3_DR = byte;
    while (!(SPI3_SR & SPI_SR_RXNE));
    return (unsigned char)SPI3_DR;
}

static unsigned char flash_read_byte(void) { return spi3_xfer(0x00); }

int main(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    RCC_AHB1ENR |= (1 << 1); // GPIOB
    RCC_APB1ENR |= (1 << 15); // SPI3

    uart_init();
    uart_puts("\r\n=== SPI3 External Flash (write/erase) ===\r\n");

    // PB12 CS output
    GPIOB_MODER &= ~(3 << 24);
    GPIOB_MODER |=  (1 << 24);
    cs_high();

    // PB3=SCK, PB4=MISO, PB5=MOSI AF6
    GPIOB_MODER &= ~((3 << 6) | (3 << 8) | (3 << 10));
    GPIOB_MODER |=  ((2 << 6) | (2 << 8) | (2 << 10));
    GPIOB_AFRL &= ~(0xFFF << 12);
    GPIOB_AFRL |=  (6 << (4 * 3)) | (6 << (4 * 4)) | (6 << (4 * 5));

    SPI3_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (4 << 3);
    SPI3_CR1 |= SPI_CR1_SPE;

    // JEDEC ID
    cs_low();
    spi3_xfer(0x9F);
    unsigned char j0 = flash_read_byte();
    unsigned char j1 = flash_read_byte();
    unsigned char j2 = flash_read_byte();
    cs_high();
    uart_puts("  JEDEC ID: ");
    uart_puthex((j0 << 16) | (j1 << 8) | j2);
    uart_puts("\r\n");
    test("jedec manufacturer EF", j0 == 0xEF);
    test("jedec device", j1 == 0x40 && j2 == 0x15);

    // Write enable + status
    cs_low(); spi3_xfer(0x06); cs_high();
    cs_low(); spi3_xfer(0x05); unsigned char st = flash_read_byte(); cs_high();
    uart_puts("  status after WEL: ");
    uart_puthex(st);
    uart_puts("\r\n");
    test("WEL set (0x02)", st == 0x02);

    // Page program 16 bytes at 0x000000
    const unsigned char payload[16] = {
        'H','e','l','l','o',' ','S','T','M','3','2','F','4','0','7','!'
    };
    cs_low();
    spi3_xfer(0x02);          // PageProgram
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x00); // addr 0x000000
    int i;
    for (i = 0; i < 16; i++) spi3_xfer(payload[i]);
    cs_high();

    // Status after program: WEL cleared
    cs_low(); spi3_xfer(0x05); st = flash_read_byte(); cs_high();
    test("WEL cleared after program", st == 0x00);

    // Read back
    cs_low();
    spi3_xfer(0x03);
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x00);
    int ok = 1;
    for (i = 0; i < 16; i++) {
        unsigned char b = flash_read_byte();
        if (b != payload[i]) ok = 0;
    }
    cs_high();
    test("readback matches payload", ok);

    // Program another 8 bytes at 0x000010 (same page region)
    const unsigned char payload2[8] = { 'S','P','I','-','F','L','A','S' };
    cs_low(); spi3_xfer(0x06); cs_high();   // WriteEnable
    cs_low();
    spi3_xfer(0x02);
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x10);
    for (i = 0; i < 8; i++) spi3_xfer(payload2[i]);
    cs_high();

    cs_low();
    spi3_xfer(0x03);
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x10);
    ok = 1;
    for (i = 0; i < 8; i++) {
        unsigned char b = flash_read_byte();
        if (b != payload2[i]) ok = 0;
    }
    cs_high();
    test("second program readback", ok);

    // Verify byte at 0x000000 unchanged by the second program
    cs_low();
    spi3_xfer(0x03);
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x00);
    unsigned char b0 = flash_read_byte();
    cs_high();
    test("first payload intact", b0 == 'H');

    // Sector erase (4k) at 0x000000
    cs_low(); spi3_xfer(0x06); cs_high();   // WriteEnable
    cs_low();
    spi3_xfer(0x20);          // SectorErase4k
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x00);
    cs_high();

    cs_low(); spi3_xfer(0x05); st = flash_read_byte(); cs_high();
    test("WEL cleared after erase", st == 0x00);

    // Read back: all 0xFF
    cs_low();
    spi3_xfer(0x03);
    spi3_xfer(0x00); spi3_xfer(0x00); spi3_xfer(0x00);
    ok = 1;
    for (i = 0; i < 32; i++) {
        if (flash_read_byte() != 0xFF) ok = 0;
    }
    cs_high();
    test("erased to 0xFF", ok);

    uart_puts("\r\n--- SUMMARY ---\r\n");
    uart_puts("PASS: ");
    uart_puthex(pass_count);
    uart_puts("\r\n");
    uart_puts("FAIL: ");
    uart_puthex(fail_count);
    uart_puts("\r\n");
    uart_puts("SPI FLASH TEST DONE\r\n");
    while (1);
}
