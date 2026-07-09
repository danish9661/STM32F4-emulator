extern "C" void init(void) {}

#define RCC_CR     (*(volatile uint32_t*)0x40023800)
#define RCC_CFGR   (*(volatile uint32_t*)0x40023808)
#define RCC_AHB1ENR (*(volatile uint32_t*)0x40023830)
#define RCC_APB1ENR (*(volatile uint32_t*)0x40023840)
#define RCC_APB2ENR (*(volatile uint32_t*)0x40023844)

#define GPIOA_MODER (*(volatile uint32_t*)0x40020000)
#define GPIOA_AFRL  (*(volatile uint32_t*)0x40020020)
#define GPIOA_AFRH  (*(volatile uint32_t*)0x40020024)
#define GPIOB_MODER (*(volatile uint32_t*)0x40020400)
#define GPIOB_AFRL  (*(volatile uint32_t*)0x40020420)

#define USART1_SR  (*(volatile uint32_t*)0x40011000)
#define USART1_DR  (*(volatile uint32_t*)0x40011004)
#define USART1_BRR (*(volatile uint32_t*)0x40011008)
#define USART1_CR1 (*(volatile uint32_t*)0x4001100C)

#define SPI1_CR1  (*(volatile uint32_t*)0x40013000)
#define SPI1_CR2  (*(volatile uint32_t*)0x40013004)
#define SPI1_SR   (*(volatile uint32_t*)0x40013008)
#define SPI1_DR   (*(volatile uint32_t*)0x4001300C)

#define I2C1_BASE (0x40005400)
#define I2C1_CR1  (*(volatile uint32_t*)(I2C1_BASE + 0x00))
#define I2C1_SR1  (*(volatile uint32_t*)(I2C1_BASE + 0x14))
#define I2C1_SR2  (*(volatile uint32_t*)(I2C1_BASE + 0x18))
#define I2C1_DR   (*(volatile uint32_t*)(I2C1_BASE + 0x10))

#define DMA2_BASE    (0x40026400)
#define DMA2_S0_CR   (*(volatile uint32_t*)(DMA2_BASE + 0x10))
#define DMA2_S0_NDTR (*(volatile uint32_t*)(DMA2_BASE + 0x14))
#define DMA2_S0_PAR  (*(volatile uint32_t*)(DMA2_BASE + 0x18))
#define DMA2_S0_M0AR (*(volatile uint32_t*)(DMA2_BASE + 0x1C))
#define DMA2_LISR    (*(volatile uint32_t*)(DMA2_BASE + 0x00))
#define DMA2_HISR    (*(volatile uint32_t*)(DMA2_BASE + 0x04))
#define DMA2_LIFCR   (*(volatile uint32_t*)(DMA2_BASE + 0x08))

#define STK_CTRL  (*(volatile uint32_t*)0xE000E010)
#define STK_LOAD  (*(volatile uint32_t*)0xE000E014)
#define STK_VAL   (*(volatile uint32_t*)0xE000E018)

#define NVIC_ISER1 (*(volatile uint32_t*)0xE000E104)

#define FSMC_BCR1 (*(volatile uint32_t*)0xA0000000)
#define FSMC_DATA (*(volatile uint32_t*)0x60000000)

static uint32_t pass, fail;
static volatile uint32_t dma_irq_fired;

extern "C" void DMA2_Stream0_IRQHandler(void) {
    dma_irq_fired = 1;
    DMA2_LIFCR = (1 << 4);
}

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    GPIOA_MODER &= ~((3 << 18) | (3 << 20));
    GPIOA_MODER |=  ((2 << 18) | (2 << 20));
    GPIOA_AFRH  |=  ((7 << 4) | (7 << 8));
    USART1_BRR = 364;
    USART1_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void putc(char c) {
    while (!(USART1_SR & (1 << 7)));
    USART1_DR = c;
}

static void tx_str(const char *s) { while (*s) putc(*s++); }

static void tx_hex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        putc(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

static void tx_dec(uint32_t v) {
    char buf[12]; int i = 11; buf[11] = 0;
    do { buf[--i] = '0' + (v % 10); v /= 10; } while (v);
    tx_str(&buf[i]);
}

#define CHECK(cond, msg) do { \
    if (cond) { tx_str("  PASS "); tx_str(msg); putc('\n'); pass++; } \
    else { tx_str("  FAIL "); tx_str(msg); putc('\n'); fail++; } \
} while(0)

static void i2c_start(void) { I2C1_CR1 |= (1 << 8); }
static void i2c_stop(void)  { I2C1_CR1 |= (1 << 9); }
static void i2c_wait_sb(void) { while (!(I2C1_SR1 & 1)); }
static void i2c_wait_addr(void) { while (!(I2C1_SR1 & (1 << 1))); }
static void i2c_wait_txe(void) { while (!(I2C1_SR1 & (1 << 6))); }
static void i2c_wait_rxne(void) { while (!(I2C1_SR1 & (1 << 5))); }
static void i2c_clear_addr(void) { volatile uint32_t sr = I2C1_SR2; (void)sr; }
static void i2c_send_byte(uint8_t b) { I2C1_DR = b; }
static uint8_t i2c_read_byte(void) { return I2C1_DR; }

static int i2c_eeprom_write_byte(uint8_t mem_addr, uint8_t data) {
    i2c_start();
    i2c_wait_sb();
    i2c_send_byte(0xA0);
    i2c_wait_addr();
    i2c_clear_addr();
    i2c_wait_txe();
    i2c_send_byte(mem_addr);
    i2c_wait_txe();
    i2c_send_byte(data);
    i2c_wait_txe();
    i2c_stop();
    return (I2C1_SR1 & (1 << 9)) ? -1 : 0;
}

static int i2c_eeprom_read_byte(uint8_t mem_addr, uint8_t *out) {
    i2c_start();
    i2c_wait_sb();
    i2c_send_byte(0xA0);
    i2c_wait_addr();
    i2c_clear_addr();
    i2c_wait_txe();
    i2c_send_byte(mem_addr);
    i2c_wait_txe();
    i2c_start();
    i2c_wait_sb();
    i2c_send_byte(0xA1);
    i2c_wait_addr();
    I2C1_CR1 &= ~(1 << 10);
    i2c_clear_addr();
    i2c_wait_rxne();
    *out = i2c_read_byte();
    i2c_stop();
    return (I2C1_SR1 & (1 << 9)) ? -1 : 0;
}

void setup() {
    uart_init();
    RCC_AHB1ENR |= (1 << 0) | (1 << 1);
    RCC_APB1ENR |= (1 << 21) | (1 << 0);
    RCC_APB2ENR |= (1 << 12);
    RCC_AHB1ENR |= (1 << 22);

    tx_str("=== PERIPHERAL TEST ===\n");

    // 1. RCC
    CHECK(RCC_CR == 0xFFFFFFFF, "RCC_CR=0xFFFFFFFF");
    CHECK(RCC_CFGR == 8, "RCC_CFGR SWS=HSI");

    // 2. SysTick
    STK_LOAD = 1000;
    STK_CTRL = 1;
    uint32_t sy1 = STK_VAL;
    volatile int d;
    for (d = 0; d < 100; d++);
    uint32_t sy2 = STK_VAL;
    CHECK(sy1 != sy2, "SysTick VAL changed after delay");

    // 3. SPI1
    GPIOA_MODER &= ~((3 << 10) | (3 << 12) | (3 << 14));
    GPIOA_MODER |=  ((2 << 10) | (2 << 12) | (2 << 14));
    GPIOA_AFRL  |=  ((5 << 20) | (5 << 24) | (5 << 28));
    SPI1_CR1 = 0x0347;
    SPI1_DR = 0xA5;
    CHECK(SPI1_SR & 1, "SPI1 TXE set");
    SPI1_DR = 0x5A;
    SPI1_SR;
    CHECK(SPI1_SR & 1, "SPI1 TXE set (2nd)");

    // 4. I2C1 register reset test
    GPIOB_MODER &= ~((3 << 12) | (3 << 14));
    GPIOB_MODER |=  ((2 << 12) | (2 << 14));
    GPIOB_AFRL  |=  ((4 << 24) | (4 << 28));
    I2C1_CR1 = 0x8001;
    I2C1_DR = 0x42;
    CHECK(I2C1_SR1 == 0, "I2C1_SR1 = 0 after reset");
    CHECK(I2C1_SR2 == 0, "I2C1_SR2 = 0 after reset");

    // 5. I2C EEPROM test
    I2C1_CR1 = 0x8001;
    {
        int ret = i2c_eeprom_write_byte(0x10, 0xAB);
        CHECK(ret == 0, "I2C EEPROM write ACK");
        uint8_t val;
        ret = i2c_eeprom_read_byte(0x10, &val);
        CHECK(ret == 0, "I2C EEPROM read ACK");
        CHECK(val == 0xAB, "I2C EEPROM readback=0xAB");
    }
    {
        int ret = i2c_eeprom_write_byte(0x10, 0xCD);
        CHECK(ret == 0, "I2C EEPROM overwrite ACK");
        uint8_t val;
        ret = i2c_eeprom_read_byte(0x10, &val);
        CHECK(ret == 0, "I2C EEPROM readback ACK");
        CHECK(val == 0xCD, "I2C EEPROM overwrite readback=0xCD");
    }
    {
        uint8_t val;
        int ret = i2c_eeprom_read_byte(0x00, &val);
        CHECK(ret == 0, "I2C EEPROM addr 00 ACK");
        CHECK(val == 0xFF, "I2C EEPROM addr 00 = 0xFF (unwritten)");
    }

    // 6. DMA2: memory-to-memory copy
    volatile uint32_t src[4] __attribute__((aligned(16))) = {0xDEADBEEF, 0xCAFEBABE, 0x12345678, 0x87654321};
    volatile uint32_t dst[4] __attribute__((aligned(16))) = {0, 0, 0, 0};
    DMA2_S0_PAR = (uint32_t)&src[0];
    DMA2_S0_M0AR = (uint32_t)&dst[0];
    DMA2_S0_NDTR = 4;
    DMA2_S0_CR = (1 << 14) | (1 << 11) | (1 << 10) | (1 << 9) | (1 << 7) | 1;
    for (d = 0; d < 100; d++);
    CHECK(DMA2_S0_NDTR == 0, "DMA NDTR=0");
    CHECK(dst[0] == 0xDEADBEEF, "dst0=DEADBEEF");
    CHECK(dst[1] == 0xCAFEBABE, "dst1=CAFEBABE");
    CHECK(dst[2] == 0x12345678, "dst2=12345678");
    CHECK(dst[3] == 0x87654321, "dst3=87654321");

    // 7. DMA LISR status bits
    CHECK(DMA2_LISR != 0, "DMA LISR non-zero after xfer");
    CHECK(DMA2_LISR & (1 << 4), "DMA LISR TCIF set");
    CHECK(DMA2_LISR & (1 << 3), "DMA LISR HTIF set");
    DMA2_LIFCR = (1 << 4);
    CHECK((DMA2_LISR & (1 << 4)) == 0, "DMA LISR TCIF cleared via LIFCR");

    // 8. DMA interrupt test
    dma_irq_fired = 0;
    NVIC_ISER1 |= (1 << 24);
    DMA2_S0_PAR = (uint32_t)&src[0];
    DMA2_S0_M0AR = (uint32_t)&dst[0];
    DMA2_S0_NDTR = 4;
    DMA2_S0_CR = (1 << 14) | (1 << 11) | (1 << 10) | (1 << 9) | (1 << 7) | (1 << 5) | (1 << 4) | 1;
    for (d = 0; d < 1000; d++);
    CHECK(dma_irq_fired != 0, "DMA TC IRQ fired");

    // 9. FSMC
    CHECK(FSMC_BCR1 == 0, "FSMC_BCR1=0");
    FSMC_DATA = 0x12345678;
    CHECK(FSMC_DATA == 0 || FSMC_DATA == 0x12345678, "FSMC bank1=OK");

    tx_str("---- SUMMARY ----\n");
    tx_str("PASS: "); tx_dec(pass); putc('\n');
    tx_str("FAIL: "); tx_dec(fail); putc('\n');
    tx_str("=== DONE ===\n");
    while (1);
}

void loop() {}
