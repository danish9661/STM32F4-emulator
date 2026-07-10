#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))
#define GPIOC_BASE  0x40020800
#define GPIOC_MODER (*(volatile unsigned int *)(GPIOC_BASE + 0x00))
#define GPIOC_AFRL  (*(volatile unsigned int *)(GPIOC_BASE + 0x20))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define I2S2_BASE   0x40003800
#define SPI2_CR1    (*(volatile unsigned int *)(I2S2_BASE + 0x00))
#define SPI2_CR2    (*(volatile unsigned int *)(I2S2_BASE + 0x04))
#define SPI2_SR     (*(volatile unsigned int *)(I2S2_BASE + 0x08))
#define SPI2_DR     (*(volatile unsigned int *)(I2S2_BASE + 0x0C))
#define SPI2_CRCPR  (*(volatile unsigned int *)(I2S2_BASE + 0x10))
#define SPI2_RXCRCR (*(volatile unsigned int *)(I2S2_BASE + 0x14))
#define SPI2_TXCRCR (*(volatile unsigned int *)(I2S2_BASE + 0x18))
#define SPI2_I2SCFGR (*(volatile unsigned int *)(I2S2_BASE + 0x1C))
#define SPI2_I2SPR  (*(volatile unsigned int *)(I2S2_BASE + 0x20))

#define I2S2EXT_BASE 0x40003400
#define I2S2EXT_CR1  (*(volatile unsigned int *)(I2S2EXT_BASE + 0x00))
#define I2S2EXT_CR2  (*(volatile unsigned int *)(I2S2EXT_BASE + 0x04))
#define I2S2EXT_SR   (*(volatile unsigned int *)(I2S2EXT_BASE + 0x08))
#define I2S2EXT_DR   (*(volatile unsigned int *)(I2S2EXT_BASE + 0x0C))
#define I2S2EXT_I2SCFGR (*(volatile unsigned int *)(I2S2EXT_BASE + 0x1C))
#define I2S2EXT_I2SPR (*(volatile unsigned int *)(I2S2EXT_BASE + 0x20))

#define I2S3EXT_BASE 0x40004000
#define I2S3EXT_CR1  (*(volatile unsigned int *)(I2S3EXT_BASE + 0x00))
#define I2S3EXT_CR2  (*(volatile unsigned int *)(I2S3EXT_BASE + 0x04))
#define I2S3EXT_SR   (*(volatile unsigned int *)(I2S3EXT_BASE + 0x08))
#define I2S3EXT_DR   (*(volatile unsigned int *)(I2S3EXT_BASE + 0x0C))
#define I2S3EXT_I2SCFGR (*(volatile unsigned int *)(I2S3EXT_BASE + 0x1C))
#define I2S3EXT_I2SPR  (*(volatile unsigned int *)(I2S3EXT_BASE + 0x20))

#define SAI_BASE    0x40015800
#define SAI_GCR     (*(volatile unsigned int *)(SAI_BASE + 0x00))
#define SAI_A_CR1   (*(volatile unsigned int *)(SAI_BASE + 0x04))
#define SAI_A_CR2   (*(volatile unsigned int *)(SAI_BASE + 0x08))
#define SAI_A_FRCR  (*(volatile unsigned int *)(SAI_BASE + 0x0C))
#define SAI_A_SLOTR (*(volatile unsigned int *)(SAI_BASE + 0x10))
#define SAI_A_IM    (*(volatile unsigned int *)(SAI_BASE + 0x14))
#define SAI_A_SR    (*(volatile unsigned int *)(SAI_BASE + 0x18))
#define SAI_A_CLRFR (*(volatile unsigned int *)(SAI_BASE + 0x1C))
#define SAI_A_DR    (*(volatile unsigned int *)(SAI_BASE + 0x20))
#define SAI_B_CR1   (*(volatile unsigned int *)(SAI_BASE + 0x24))

#define NVIC_ISER0  (*(volatile unsigned int *)0xE000E100)
#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)
#define NVIC_ISPR0  (*(volatile unsigned int *)0xE000E100 + 0x100)
#define NVIC_ISPR1  (*(volatile unsigned int *)0xE000E104 + 0x100)

static volatile int pass_count = 0;
static volatile int fail_count = 0;

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
    uart_puts(" vs ");
    uart_puthex(expected);
    uart_puts(")\r\n");
    if (cond) pass_count++; else fail_count++;
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    RCC_AHB1ENR |= (1 << 2); // GPIOC
    uart_init();
    uart_puts("I2S SAI audio test\r\n");
    uart_puts("\r\n=== SPI2 (I2S2) ===\r\n");

    // SPI2 is at 0x40003800, shared with I2S2
    // CR1 default
    test_val("SPI2_CR1 default", SPI2_CR1, 0x0);
    // Write CR1
    SPI2_CR1 = 0xF7;
    test_val("SPI2_CR1 write 0xF7", SPI2_CR1, 0xF7);
    // SR default
    test_val("SPI2_SR default", SPI2_SR, 0x3);

    // CRCPR, RXCRCR, TXCRCR
    SPI2_CRCPR = 0xABCD;
    test_val("SPI2_CRCPR", SPI2_CRCPR, 0xABCD);
    SPI2_RXCRCR = 0x1234;
    test_val("SPI2_RXCRCR", SPI2_RXCRCR, 0x1234);
    SPI2_TXCRCR = 0x5678;
    test_val("SPI2_TXCRCR", SPI2_TXCRCR, 0x5678);

    // Enable I2S mode first
    SPI2_I2SCFGR = 1;
    test_val("SPI2_I2SCFGR (I2SMOD)", SPI2_I2SCFGR, 0x1);
    SPI2_I2SPR = 0x1AB;
    test_val("SPI2_I2SPR", SPI2_I2SPR, 0x1AB);

    // Audio data via DR (write/read) - with I2SMOD=1
    SPI2_DR = 0x1234;
    unsigned int dr1 = SPI2_DR;
    SPI2_DR = 0x5678;
    unsigned int dr2 = SPI2_DR;
    test("SPI2 DR audio data changes", dr1 != dr2);
    uart_puts("    DR samples: ");
    uart_puthex(dr1);
    uart_puthex(dr2);
    uart_puts("\r\n");

    // Multiple audio writes
    uart_puts("\r\n  Audio samples:");
    for (int i = 0; i < 8; i++) {
        SPI2_DR = 0x1000 + i;
        unsigned int sample = SPI2_DR;
        uart_puts(" ");
        uart_puthex(sample);
    }
    uart_puts("\r\n");

    // SR toggles on each read; 2nd read gives TXE state
    (void)SPI2_SR;
    test("SPI2 SR TXE after DR", SPI2_SR & 0x2);

    uart_puts("\r\n=== I2S2ext (0x40003400) ===\r\n");
    test_val("I2S2ext CR1 default", I2S2EXT_CR1, 0x0);
    I2S2EXT_CR1 = 0x5678;
    test_val("I2S2ext CR1 write 0x5678", I2S2EXT_CR1, 0x5678);
    I2S2EXT_CR2 = 0x2;
    test_val("I2S2ext CR2 write 0x2", I2S2EXT_CR2, 0x2);
    test_val("I2S2ext SR default", I2S2EXT_SR, 0x3);
    I2S2EXT_I2SCFGR = 0xDEF;
    test_val("I2S2ext I2SCFGR", I2S2EXT_I2SCFGR, 0xDEF);
    I2S2EXT_I2SPR = 0x2AB;
    test_val("I2S2ext I2SPR", I2S2EXT_I2SPR, 0x2AB);

    // I2S2ext audio data
    uart_puts("\r\n  I2S2ext audio:");
    for (int i = 0; i < 6; i++) {
        I2S2EXT_DR = 0x2000 + i;
        unsigned int sample = I2S2EXT_DR;
        uart_puts(" ");
        uart_puthex(sample);
    }
    uart_puts("\r\n");

    uart_puts("\r\n=== I2S3ext (0x40004000) ===\r\n");
    test_val("I2S3ext CR1 default", I2S3EXT_CR1, 0x0);
    I2S3EXT_CR1 = 0x9ABC;
    test_val("I2S3ext CR1 write 0x9ABC", I2S3EXT_CR1, 0x9ABC);
    I2S3EXT_CR2 = 0x2;
    test_val("I2S3ext CR2 write 0x2", I2S3EXT_CR2, 0x2);
    test_val("I2S3ext SR default", I2S3EXT_SR, 0x3);
    I2S3EXT_I2SCFGR = 0x789;
    test_val("I2S3ext I2SCFGR", I2S3EXT_I2SCFGR, 0x789);

    // I2S3ext audio data  
    uart_puts("\r\n  I2S3ext audio:");
    for (int i = 0; i < 6; i++) {
        I2S3EXT_DR = 0x3000 + i;
        unsigned int sample = I2S3EXT_DR;
        uart_puts(" ");
        uart_puthex(sample);
    }
    uart_puts("\r\n");

    uart_puts("\r\n=== SAI (0x40015800) ===\r\n");

    // SAI GCR
    SAI_GCR = 0x7;
    test_val("SAI GCR", SAI_GCR, 0x7);

    // SAI Block A
    test_val("SAI A_CR1 default", SAI_A_CR1, 0x40);
    SAI_A_CR1 = 0x3F3FFFFF;
    test_val("SAI A_CR1 write max", SAI_A_CR1, 0x3F3FFFFF);
    SAI_A_CR2 = 0x7FFF;
    test_val("SAI A_CR2", SAI_A_CR2, 0x7FFF);
    test_val("SAI A_FRCR default", SAI_A_FRCR, 0x7);
    SAI_A_FRCR = 0x7FFFF;
    test_val("SAI A_FRCR write", SAI_A_FRCR, 0x7FFFF);
    SAI_A_SLOTR = 0x1FFFFF;
    test_val("SAI A_SLOTR", SAI_A_SLOTR, 0x1FFFFF);
    test_val("SAI A_IM default", SAI_A_IM, 0x0);
    test_val("SAI A_SR default", SAI_A_SR, 0x8);

    // CLRFR test
    SAI_A_CLRFR = 0x77;
    test_val("SAI A_CLRFR write 0x77", SAI_A_CLRFR, 0x77);
    test_val("SAI A_SR after CLRFR", SAI_A_SR, 0x8);

    // SAI A audio data
    uart_puts("\r\n  SAI A audio data:\r\n");
    SAI_A_DR = 0xDEAD;
    unsigned int sai_sr_after_write = SAI_A_SR;
    unsigned int sai_dr1 = SAI_A_DR;
    test_val("SAI A_SR after DR write", sai_sr_after_write, 0x13);
    test_val("SAI A_DR readback", sai_dr1, 0xDEAD);

    SAI_A_DR = 0xBEEF;
    unsigned int sai_sr_after_write2 = SAI_A_SR;
    unsigned int sai_dr2 = SAI_A_DR;
    test_val("SAI A_SR after 2nd DR write", sai_sr_after_write2, 0x13);
    test_val("SAI A_DR after 2nd write", sai_dr2, 0xBEEF);

    // CLRFR after DR writes
    SAI_A_CLRFR = 0x77;
    test_val("SAI A_SR after CLRFR", SAI_A_SR, 0x8);

    // SAI Block B
    test_val("SAI B_CR1 default", SAI_B_CR1, 0x40);
    SAI_B_CR1 = 0x3F3F0000;
    test_val("SAI B_CR1 write", SAI_B_CR1, 0x3F3F0000);

    // Interrupt test - SAI A
    uart_puts("\r\n=== Interrupt test ===\r\n");

    // Enable SAI A interrupt (AFDETIE = bit 1)
    SAI_A_IM = 0x2;
    test_val("SAI A_IM set", SAI_A_IM, 0x2);
    SAI_A_CLRFR = 0x77;
    // Write DR with IM enabled
    SAI_A_DR = 0xFEED;
    test_val("SAI A_SR after DR (IM=2)", SAI_A_SR, 0x13);
    // Check NVIC pending for IRQ87
    unsigned int iser2 = *(volatile unsigned int *)0xE000E108;
    (void)iser2;

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
