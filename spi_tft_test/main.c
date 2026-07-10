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
#define GPIOB_AFRH  (*(volatile unsigned int *)(GPIOB_BASE + 0x24))
#define GPIOC_BASE  0x40020800
#define GPIOC_MODER (*(volatile unsigned int *)(GPIOC_BASE + 0x00))
#define GPIOC_AFRL  (*(volatile unsigned int *)(GPIOC_BASE + 0x20))
#define GPIO_BSRR(base) (*(volatile unsigned int *)((base) + 0x18))
#define GPIO_ODR(base)  (*(volatile unsigned int *)((base) + 0x14))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define SPI2_BASE   0x40003800
#define SPI2_CR1    (*(volatile unsigned int *)(SPI2_BASE + 0x00))
#define SPI2_CR2    (*(volatile unsigned int *)(SPI2_BASE + 0x04))
#define SPI2_SR     (*(volatile unsigned int *)(SPI2_BASE + 0x08))
#define SPI2_DR     (*(volatile unsigned int *)(SPI2_BASE + 0x0C))
#define SPI2_CRCPR  (*(volatile unsigned int *)(SPI2_BASE + 0x10))
#define SPI2_RXCRCR (*(volatile unsigned int *)(SPI2_BASE + 0x14))
#define SPI2_TXCRCR (*(volatile unsigned int *)(SPI2_BASE + 0x18))
#define SPI2_I2SCFGR (*(volatile unsigned int *)(SPI2_BASE + 0x1C))
#define SPI2_I2SPR  (*(volatile unsigned int *)(SPI2_BASE + 0x20))

#define SPI3_BASE   0x40003C00
#define SPI3_CR1    (*(volatile unsigned int *)(SPI3_BASE + 0x00))
#define SPI3_CR2    (*(volatile unsigned int *)(SPI3_BASE + 0x04))
#define SPI3_SR     (*(volatile unsigned int *)(SPI3_BASE + 0x08))
#define SPI3_DR     (*(volatile unsigned int *)(SPI3_BASE + 0x0C))

#define SPI_CR1_CPHA   (1 << 0)
#define SPI_CR1_CPOL   (1 << 1)
#define SPI_CR1_MSTR   (1 << 2)
#define SPI_CR1_BR     (7 << 3)
#define SPI_CR1_SPE    (1 << 6)
#define SPI_CR1_LSBFIRST (1 << 7)
#define SPI_CR1_SSI    (1 << 8)
#define SPI_CR1_SSM    (1 << 9)
#define SPI_CR1_DFF    (1 << 11)

#define SPI_CR2_RXDMAEN (1 << 0)
#define SPI_CR2_TXDMAEN (1 << 1)
#define SPI_CR2_SSOE   (1 << 2)
#define SPI_CR2_TXEIE  (1 << 7)
#define SPI_CR2_RXNEIE (1 << 6)

#define SPI_SR_RXNE   (1 << 0)
#define SPI_SR_TXE    (1 << 1)
#define SPI_SR_BSY    (1 << 7)

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

static void test_val(const char *name, unsigned int actual, unsigned int expected) {
    int cond = (actual == expected);
    uart_puts("  ");
    uart_puts(cond ? "PASS" : "FAIL");
    uart_puts(" ");
    uart_puts(name);
    uart_puts(" (");
    uart_puthex(actual);
    if (!cond) { uart_puts(" vs "); uart_puthex(expected); }
    uart_puts(")\r\n");
    if (cond) pass_count++; else fail_count++;
}

// Software CS via GPIO
static void cs_low(void) { GPIO_BSRR(GPIOB_BASE) = (1 << (12 + 16)); } // PB12 reset
static void cs_high(void) { GPIO_BSRR(GPIOB_BASE) = (1 << 12); }      // PB12 set

// SPI2 transfer one byte
static unsigned char spi2_xfer(unsigned char byte) {
    SPI2_DR = byte;
    while (!(SPI2_SR & SPI_SR_RXNE));
    return (unsigned char)SPI2_DR;
}

// SPI2 transfer 16-bit
static unsigned short spi2_xfer16(unsigned short word) {
    volatile unsigned int tmp;
    SPI2_DR = word;
    while (!(SPI2_SR & SPI_SR_RXNE));
    tmp = SPI2_DR;
    return (unsigned short)tmp;
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    RCC_AHB1ENR |= (1 << 1); // GPIOB
    RCC_APB1ENR |= (1 << 14); // SPI2
    RCC_APB1ENR |= (1 << 15); // SPI3

    uart_init();
    uart_puts("SPI TFT LCD test\r\n");
    uart_puts("\r\n=== SPI2 Master 8-bit ===\r\n");

    // Configure PB12 as output (CS)
    GPIOB_MODER &= ~(3 << 24);
    GPIOB_MODER |=  (1 << 24);
    cs_high();

    // Configure PB13 (SCK), PB14 (MISO), PB15 (MOSI) as AF5
    GPIOB_AFRH &= ~(0xFFF << 0);
    GPIOB_AFRH |=  (5 << (4 * 5)) | (5 << (4 * 6)) | (5 << (4 * 7));
    GPIOB_MODER &= ~((3 << 26) | (3 << 28) | (3 << 30));
    GPIOB_MODER |=  ((2 << 26) | (2 << 28) | (2 << 30));

    // SPI2 default registers
    test_val("SPI2 CR1 default", SPI2_CR1, 0x0);
    test_val("SPI2 CR2 default", SPI2_CR2, 0x0);
    test_val("SPI2 SR default", SPI2_SR, 0x3);

    // Configure SPI2 master: BR=div2, CPHA=0, CPOL=0, SSM=1, SSI=1
    SPI2_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (2 << 3);
    SPI2_CR1 |= SPI_CR1_SPE;
    unsigned int cr1_8bit = SPI2_CR1 & 0xF7F;
    test_val("SPI2 CR1 master 8bit", cr1_8bit,
             SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (2 << 3) | SPI_CR1_SPE);
    // Verify SR after enable (SR toggles on read; 2nd read gives TXE)
    (void)SPI2_SR;
    test("SPI2 SR TXE after SPE", SPI2_SR & SPI_SR_TXE);

    // SPI write/read 8-bit
    uart_puts("\r\n  SPI2 8-bit transfers:\r\n");
    cs_low();
    unsigned char r1 = spi2_xfer(0x90); // read command
    unsigned char r2 = spi2_xfer(0x00); // dummy
    unsigned char r3 = spi2_xfer(0x00); // dummy
    unsigned char r4 = spi2_xfer(0x00); // JEDEC ID MSB
    cs_high();
    uart_puts("  0x90 read: ");
    uart_puthex(r1);
    uart_puthex(r2);
    uart_puthex(r3);
    uart_puthex(r4);
    uart_puts("\r\n");

    // CRCPR, RXCRCR, TXCRCR verify
    SPI2_CRCPR = 0x7;
    test_val("SPI2 CRCPR", SPI2_CRCPR, 0x7);
    SPI2_RXCRCR = 0xAA;
    test_val("SPI2 RXCRCR", SPI2_RXCRCR, 0xAA);
    SPI2_TXCRCR = 0xBB;
    test_val("SPI2 TXCRCR", SPI2_TXCRCR, 0xBB);

    uart_puts("\r\n=== SPI2 Master 16-bit ===\r\n");

    // Change to 16-bit mode
    SPI2_CR1 = 0; // disable
    SPI2_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (2 << 3) | SPI_CR1_DFF;
    SPI2_CR1 |= SPI_CR1_SPE;
    unsigned int cr1_16bit = SPI2_CR1 & 0xF7F;
    test_val("SPI2 CR1 master 16bit", cr1_16bit,
             SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (2 << 3) | SPI_CR1_DFF | SPI_CR1_SPE);

    // 16-bit transfer
    cs_low();
    unsigned short w16 = spi2_xfer16(0xAABB);
    cs_high();
    uart_puts("  16-bit xfer: ");
    uart_puthex(w16);
    uart_puts("\r\n");

    // CR2 TXEIE/RXNEIE
    SPI2_CR2 = SPI_CR2_TXEIE | SPI_CR2_RXNEIE | SPI_CR2_SSOE;
    test_val("SPI2 CR2 interrupts", SPI2_CR2, SPI_CR2_TXEIE | SPI_CR2_RXNEIE | SPI_CR2_SSOE);

    // Back to 8-bit
    SPI2_CR1 = 0;
    SPI2_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (2 << 3);
    SPI2_CR1 |= SPI_CR1_SPE;

    // Bulk write/read (TFT display init sequence)
    uart_puts("\r\n  TFT init-like sequence:\r\n");
    const unsigned char init_seq[] = {
        0x11, // SLPOUT
        0x36, 0x00, // MADCTL
        0x3A, 0x05, // COLMOD = 16bit
        0x21, // INVON
        0x13, // NORON
        0x29, // DISPON
    };
    int i;
    cs_low();
    for (i = 0; i < 6; i++) {
        unsigned char r = spi2_xfer(init_seq[i]);
        uart_puts("  "); uart_puthex(init_seq[i]); uart_puts(" -> "); uart_puthex(r); uart_puts("\r\n");
    }
    cs_high();

    uart_puts("\r\n=== SPI3 External Flash ===\r\n");

    // SPI3 default check
    test_val("SPI3 CR1 default", SPI3_CR1, 0x0);

    // Configure SPI3 master (PB3=SCK, PB4=MISO, PB5=MOSI, AF6)
    RCC_AHB1ENR |= (1 << 1);
    GPIOB_MODER &= ~((3 << 6) | (3 << 8) | (3 << 10));
    GPIOB_MODER |=  ((2 << 6) | (2 << 8) | (2 << 10));
    GPIOB_AFRL &= ~(0xFFF << 12);
    GPIOB_AFRL |=  (6 << (4 * 3)) | (6 << (4 * 4)) | (6 << (4 * 5));

    SPI3_CR1 = SPI_CR1_MSTR | SPI_CR1_SSM | SPI_CR1_SSI | (4 << 3);
    SPI3_CR1 |= SPI_CR1_SPE;
    test("SPI3 SR TXE after SPE", SPI3_SR & SPI_SR_TXE);

    // SPI3 read JEDEC ID
    cs_low();
    unsigned char j1 = 0, j2 = 0, j3 = 0, j4 = 0;
    SPI3_DR = 0x9F; while (!(SPI3_SR & SPI_SR_RXNE)); j1 = SPI3_DR;
    SPI3_DR = 0x00; while (!(SPI3_SR & SPI_SR_RXNE)); j2 = SPI3_DR;
    SPI3_DR = 0x00; while (!(SPI3_SR & SPI_SR_RXNE)); j3 = SPI3_DR;
    SPI3_DR = 0x00; while (!(SPI3_SR & SPI_SR_RXNE)); j4 = SPI3_DR;
    cs_high();
    uart_puts("  JEDEC ID: ");
    uart_puthex((j1 << 24) | (j2 << 16) | (j3 << 8) | j4);
    uart_puts("\r\n");
    test("SPI3 JEDEC ID non-zero", j1 != 0 || j2 != 0 || j3 != 0);

    // SPI3 read status (2nd byte is status register value)
    cs_low();
    SPI3_DR = 0x05; while (!(SPI3_SR & SPI_SR_RXNE)); j1 = SPI3_DR;
    SPI3_DR = 0x00; while (!(SPI3_SR & SPI_SR_RXNE)); j2 = SPI3_DR;
    cs_high();
    test("SPI3 read status reg 1", j2 == 0x00); // not busy, not write enabled

    // SPI3 write enable then read status
    cs_low();
    SPI3_DR = 0x06; while (!(SPI3_SR & SPI_SR_RXNE)); j1 = SPI3_DR;
    cs_high();
    cs_low();
    SPI3_DR = 0x05; while (!(SPI3_SR & SPI_SR_RXNE)); j1 = SPI3_DR;
    SPI3_DR = 0x00; while (!(SPI3_SR & SPI_SR_RXNE)); j2 = SPI3_DR;
    cs_high();
    uart_puts("  Status reg after WEL: ");
    uart_puthex(j2);
    uart_puts("\r\n");

    uart_puts("\r\n--- SUMMARY ---\r\n");
    uart_puts("PASS: ");
    uart_puthex(pass_count);
    uart_puts("\r\n");
    uart_puts("FAIL: ");
    uart_puthex(fail_count);
    uart_puts("\r\n");
    uart_puts("DONE\r\n");
    while (1);
}
