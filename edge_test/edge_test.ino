#define USART1_BASE  0x40011000
#define USART_SR     (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR     (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR    (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1    (*(volatile uint32_t *)(USART1_BASE + 0x0C))

#define DMA2_BASE    0x40026400
#define DMA2_LISR    (*(volatile uint32_t *)(DMA2_BASE + 0x00))
#define DMA2_HISR    (*(volatile uint32_t *)(DMA2_BASE + 0x04))
#define DMA2_LIFCR   (*(volatile uint32_t *)(DMA2_BASE + 0x08))
#define DMA2_HIFCR   (*(volatile uint32_t *)(DMA2_BASE + 0x0C))
#define DMA2_S0_CR   (*(volatile uint32_t *)(DMA2_BASE + 0x10))
#define DMA2_S0_NDTR (*(volatile uint32_t *)(DMA2_BASE + 0x14))
#define DMA2_S0_PAR  (*(volatile uint32_t *)(DMA2_BASE + 0x18))
#define DMA2_S0_M0AR (*(volatile uint32_t *)(DMA2_BASE + 0x1C))
#define DMA2_S0_M1AR (*(volatile uint32_t *)(DMA2_BASE + 0x20))
#define DMA2_S0_FCR  (*(volatile uint32_t *)(DMA2_BASE + 0x24))
#define DMA2_S4_CR   (*(volatile uint32_t *)(DMA2_BASE + 0x70))
#define DMA2_S4_NDTR (*(volatile uint32_t *)(DMA2_BASE + 0x74))
#define DMA2_S4_PAR  (*(volatile uint32_t *)(DMA2_BASE + 0x78))
#define DMA2_S4_M0AR (*(volatile uint32_t *)(DMA2_BASE + 0x7C))

#define TIM2_BASE    0x40000000
#define TIM_CR1      (*(volatile uint32_t *)(TIM2_BASE + 0x00))
#define TIM_SR       (*(volatile uint32_t *)(TIM2_BASE + 0x10))
#define TIM_EGR      (*(volatile uint32_t *)(TIM2_BASE + 0x14))
#define TIM_CNT      (*(volatile uint32_t *)(TIM2_BASE + 0x24))
#define TIM_PSC      (*(volatile uint32_t *)(TIM2_BASE + 0x28))
#define TIM_ARR      (*(volatile uint32_t *)(TIM2_BASE + 0x2C))
#define TIM_CCR1     (*(volatile uint32_t *)(TIM2_BASE + 0x34))
#define TIM_CCR2     (*(volatile uint32_t *)(TIM2_BASE + 0x38))
#define TIM_CCR3     (*(volatile uint32_t *)(TIM2_BASE + 0x3C))
#define TIM_CCR4     (*(volatile uint32_t *)(TIM2_BASE + 0x40))
#define TIM_CCER     (*(volatile uint32_t *)(TIM2_BASE + 0x20))
#define TIM_DIER     (*(volatile uint32_t *)(TIM2_BASE + 0x0C))

#define I2C1_BASE    0x40005400
#define I2C_CR1      (*(volatile uint32_t *)(I2C1_BASE + 0x00))
#define I2C_CR2      (*(volatile uint32_t *)(I2C1_BASE + 0x04))
#define I2C_DR       (*(volatile uint32_t *)(I2C1_BASE + 0x10))
#define I2C_SR1      (*(volatile uint32_t *)(I2C1_BASE + 0x14))
#define I2C_SR2      (*(volatile uint32_t *)(I2C1_BASE + 0x18))

#define SPI3_BASE    0x40003C00
#define SPI_CR1      (*(volatile uint32_t *)(SPI3_BASE + 0x00))
#define SPI_CR2      (*(volatile uint32_t *)(SPI3_BASE + 0x04))
#define SPI_SR       (*(volatile uint32_t *)(SPI3_BASE + 0x08))
#define SPI_DR       (*(volatile uint32_t *)(SPI3_BASE + 0x0C))

#define NVIC_ISER1   (*(volatile uint32_t *)0xE000E104)
#define NVIC_ISER2   (*(volatile uint32_t *)0xE000E108)

static int pass, fail;

static void tx_c(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}

static void tx_s(const char *s) {
    while (*s) tx_c(*s++);
}

static void tx_hex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7)));
        USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

#define CHECK(cond, msg) do { \
    if (cond) { tx_s("PASS "); tx_s(msg); tx_s("\n"); pass++; } \
    else { tx_s("FAIL "); tx_s(msg); tx_s("\n"); fail++; } \
} while(0)

static void i2c_start(void) {
    I2C_CR1 |= (1 << 8);
}

static void i2c_stop(void) {
    I2C_CR1 |= (1 << 9);
}

static void i2c_send_byte(uint8_t b) {
    I2C_DR = b;
}

static uint8_t i2c_read_byte(void) {
    return I2C_DR;
}

static int i2c_write_eeprom(uint8_t addr, uint8_t data) {
    I2C_CR1 |= (1 << 8);
    while (!(I2C_SR1 & 1));
    I2C_DR = 0xA0;
    while (!(I2C_SR1 & 2));
    uint32_t sr2 = I2C_SR2;
    (void)sr2;
    while (!(I2C_SR1 & (1 << 6)));
    I2C_DR = addr;
    while (!(I2C_SR1 & (1 << 6)));
    I2C_DR = data;
    while (!(I2C_SR1 & (1 << 6)));
    I2C_CR1 |= (1 << 9);
    return (I2C_SR1 & (1 << 9)) ? -1 : 0;
}

static int i2c_read_eeprom(uint8_t addr, uint8_t *out) {
    I2C_CR1 |= (1 << 8);
    while (!(I2C_SR1 & 1));
    I2C_DR = 0xA0;
    while (!(I2C_SR1 & 2));
    uint32_t sr2 = I2C_SR2;
    (void)sr2;
    while (!(I2C_SR1 & (1 << 6)));
    I2C_DR = addr;
    while (!(I2C_SR1 & (1 << 6)));
    I2C_CR1 |= (1 << 8);
    while (!(I2C_SR1 & 1));
    I2C_DR = 0xA1;
    while (!(I2C_SR1 & 2));
    I2C_CR1 &= ~(1 << 10);
    sr2 = I2C_SR2;
    (void)sr2;
    while (!(I2C_SR1 & (1 << 5)));
    *out = I2C_DR;
    I2C_CR1 |= (1 << 9);
    return (I2C_SR1 & (1 << 9)) ? -1 : 0;
}

volatile uint32_t dma_src[4] = {0xDEADBEEF, 0xCAFEBABE, 0x12345678, 0x87654321};
volatile uint32_t dma_dst[4] = {0, 0, 0, 0};
volatile uint32_t dma_src8[8] = {0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22};
volatile uint32_t dma_dst8[8] = {0, 0, 0, 0, 0, 0, 0, 0};

volatile int dma2_irq_fired;

extern "C" void DMA2_Stream0_IRQHandler(void) {
    dma2_irq_fired = 1;
    DMA2_LIFCR = (1 << 4) | (1 << 3);
}

void setup() {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
    tx_s("=== EDGE TEST ===\n");

    // ===== DMA =====
    tx_s("--- DMA ---\n");

    DMA2_S0_PAR  = (uint32_t)&dma_src[0];
    DMA2_S0_M0AR = (uint32_t)&dma_dst[0];
    DMA2_S0_NDTR = 4;
    DMA2_S0_CR   = (1<<14) | (1<<11) | (1<<10) | (1<<9) | (1<<7) | 1;
    CHECK(DMA2_S0_NDTR == 0, "DMA NDTR cleared after xfer");
    CHECK(dma_dst[0] == 0xDEADBEEF, "DMA dst[0]");
    CHECK(dma_dst[1] == 0xCAFEBABE, "DMA dst[1]");
    CHECK(dma_dst[2] == 0x12345678, "DMA dst[2]");
    CHECK(dma_dst[3] == 0x87654321, "DMA dst[3]");

    CHECK((DMA2_LISR & (1 << 4)) != 0, "DMA TCIF set");
    CHECK((DMA2_LISR & (1 << 3)) != 0, "DMA HTIF set");
    DMA2_LIFCR = (1 << 4);
    CHECK((DMA2_LISR & (1 << 4)) == 0, "DMA TCIF cleared by LIFCR");
    DMA2_LIFCR = (1 << 3);
    CHECK((DMA2_LISR & (1 << 3)) == 0, "DMA HTIF cleared by LIFCR");

    // DMA 16-bit transfer (MSIZE=PSIZE=16-bit via bits 11:10=01, 9:8=01)
    DMA2_S4_PAR  = (uint32_t)&dma_src8[0];
    DMA2_S4_M0AR = (uint32_t)&dma_dst8[0];
    DMA2_S4_NDTR = 8;
    DMA2_S4_CR   = (1<<13) | (1<<10) | (1<<9) | (1<<7) | 1;
    CHECK(dma_dst8[0] == 0xAA, "DMA 16bit dst[0]");

    // DMA re-enable for second transfer (must rewrite NDTR)
    dma_dst[0] = 0; dma_dst[1] = 0; dma_dst[2] = 0; dma_dst[3] = 0;
    DMA2_S0_NDTR = 4;
    DMA2_S0_CR   = (1<<14) | (1<<11) | (1<<10) | (1<<9) | (1<<7) | 1;
    CHECK(dma_dst[0] == 0xDEADBEEF, "DMA second xfer dst[0]");

    // DMA interrupt — poll with timeout since NVIC fires async
    dma2_irq_fired = 0;
    dma_dst[0] = 0; dma_dst[1] = 0; dma_dst[2] = 0; dma_dst[3] = 0;
    NVIC_ISER1 |= (1 << 24);
    DMA2_S0_NDTR = 4;
    DMA2_S0_CR   = (1<<14) | (1<<11) | (1<<10) | (1<<9) | (1<<7) | (1<<5) | (1<<4) | 1;
    int timeout = 100000;
    while (dma2_irq_fired == 0 && timeout-- > 0);
    CHECK(dma2_irq_fired != 0, "DMA IRQ fired");

    // ===== TIM =====
    tx_s("--- TIM ---\n");

    CHECK(TIM_CNT == 0, "TIM CNT default 0");
    TIM_PSC = 0;
    TIM_ARR = 10000;
    TIM_CR1 = 1;
    TIM_CCR1 = 0x1234;
    CHECK(TIM_CCR1 == 0x1234, "TIM CCR1 write");
    TIM_CCR2 = 0x5678;
    CHECK(TIM_CCR2 == 0x5678, "TIM CCR2 write");
    TIM_CCR3 = 0x9ABC;
    CHECK(TIM_CCR3 == 0x9ABC, "TIM CCR3 write");
    TIM_CCR4 = 0xDEF0;
    CHECK(TIM_CCR4 == 0xDEF0, "TIM CCR4 write");

    uint32_t cnt = TIM_CNT;
    CHECK(cnt > 0, "TIM CNT advances after enable");

    TIM_EGR = 1;
    // CNT advances on every TIM access; UG resets to 0 but read itself
    // adds ~1-3 ticks, so check near-zero rather than exactly 0
    CHECK(TIM_CNT <= 3, "TIM UG resets CNT");

    TIM_ARR = 100;
    TIM_SR = 0;
    int uif_timeout = 100000;
    while (!(TIM_SR & 1) && uif_timeout-- > 0);
    CHECK(uif_timeout > 0, "TIM UIF set on overflow");
    TIM_SR = 0;
    CHECK((TIM_SR & 1) == 0, "TIM UIF cleared by write 0");

    // ===== I2C =====
    tx_s("--- I2C ---\n");

    uint8_t val;
    CHECK(i2c_write_eeprom(0x20, 0xA5) == 0, "I2C EEPROM write");
    CHECK(i2c_read_eeprom(0x20, &val) == 0 && val == 0xA5, "I2C EEPROM read back");

    CHECK(i2c_write_eeprom(0x20, 0x5A) == 0, "I2C EEPROM overwrite");
    CHECK(i2c_read_eeprom(0x20, &val) == 0 && val == 0x5A, "I2C EEPROM read after overwrite");

    // Invalid address NACK — emulator sets AF (bit 9), not ADDR
    I2C_CR1 |= (1 << 8);
    while (!(I2C_SR1 & 1));
    I2C_DR = 0xFE;
    uint32_t sr1;
    int i2c_nack_timeout = 10000;
    do { sr1 = I2C_SR1; } while (!(sr1 & (1 << 9)) && i2c_nack_timeout-- > 0);
    CHECK((sr1 & (1 << 9)) != 0, "I2C NACK on invalid address");

    // SWRST preserves PE
    I2C_CR1 = 0x8001;
    CHECK((I2C_CR1 & 1) != 0, "I2C SWRST preserves PE");

    // ===== SPI =====
    tx_s("--- SPI ---\n");

    SPI_CR1 = (1 << 6) | (1 << 2) | (1 << 1);
    SPI_DR = 0x9F;
    uint8_t j1 = SPI_DR;
    SPI_DR = 0;
    uint8_t j2 = SPI_DR;
    SPI_DR = 0;
    uint8_t j3 = SPI_DR;
    CHECK(j1 == 0xEF && j2 == 0x40 && j3 == 0x16, "SPI flash JEDEC ID");

    // SPI device ID
    SPI_DR = 0x90;
    SPI_DR = 0;
    SPI_DR = 0;
    SPI_DR = 0;
    uint8_t d1 = SPI_DR;
    SPI_DR = 0;
    uint8_t d2 = SPI_DR;
    CHECK(d1 == 0xAA && d2 == 0xBB, "SPI flash device ID");

    // SPI SR toggling
    CHECK((SPI_SR & 3) != 0, "SPI SR data ready");
    CHECK((SPI_SR & 3) == 0, "SPI SR toggles");

    // SPI 16-bit mode
    SPI_CR1 = (1 << 6) | (1 << 11) | (1 << 2) | (1 << 1);
    SPI_DR = 0x9F00;
    uint32_t j16 = SPI_DR;
    uint8_t hi = (j16 >> 8) & 0xFF;
    uint8_t lo = j16 & 0xFF;
    CHECK(hi == 0xEF && lo == 0x40, "SPI 16-bit JEDEC hi/lo");

    tx_s("---- SUMMARY ----\n");
    tx_s("PASS: "); tx_hex(pass); tx_s("\n");
    tx_s("FAIL: "); tx_hex(fail); tx_s("\n");
    tx_s("=== DONE ===\n");
}

void loop() {}
