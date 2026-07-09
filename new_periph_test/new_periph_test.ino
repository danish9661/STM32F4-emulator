#define USART1_BASE  0x40011000
#define USART_SR     (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR     (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR    (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1    (*(volatile uint32_t *)(USART1_BASE + 0x0C))

#define PWR_CR       (*(volatile uint32_t *)0x40007000)
#define PWR_CSR      (*(volatile uint32_t *)0x40007004)

#define WWDG_CR      (*(volatile uint32_t *)0x40002C00)
#define WWDG_CFR     (*(volatile uint32_t *)0x40002C04)
#define WWDG_SR      (*(volatile uint32_t *)0x40002C08)

#define IWDG_KR      (*(volatile uint32_t *)0x40003000)
#define IWDG_PR      (*(volatile uint32_t *)0x40003004)
#define IWDG_RLR     (*(volatile uint32_t *)0x40003008)
#define IWDG_SR      (*(volatile uint32_t *)0x4000300C)

#define RTC_TR       (*(volatile uint32_t *)0x40002800)
#define RTC_DR       (*(volatile uint32_t *)0x40002804)
#define RTC_CR       (*(volatile uint32_t *)0x40002808)
#define RTC_ISR      (*(volatile uint32_t *)0x4000280C)
#define RTC_PRER     (*(volatile uint32_t *)0x40002810)
#define RTC_BKP0     (*(volatile uint32_t *)0x40002850)

#define CRC_DR       (*(volatile uint32_t *)0x40023000)
#define CRC_IDR      (*(volatile uint32_t *)0x40023004)
#define CRC_CR       (*(volatile uint32_t *)0x40023008)

#define RNG_CR       (*(volatile uint32_t *)0x50060800)
#define RNG_SR       (*(volatile uint32_t *)0x50060804)
#define RNG_DR       (*(volatile uint32_t *)0x50060808)

#define DAC_CR       (*(volatile uint32_t *)0x40007400)
#define DAC_DHR12R1  (*(volatile uint32_t *)0x40007408)
#define DAC_DOR1     (*(volatile uint32_t *)0x4000742C)

#define CAN_MCR      (*(volatile uint32_t *)0x40006400)
#define CAN_MSR      (*(volatile uint32_t *)0x40006404)
#define CAN_TSR      (*(volatile uint32_t *)0x40006408)
#define CAN_IER      (*(volatile uint32_t *)0x40006414)
#define CAN_ESR      (*(volatile uint32_t *)0x40006418)
#define CAN_BTR      (*(volatile uint32_t *)0x4000641C)

#define SDIO_POWER   (*(volatile uint32_t *)0x40012C00)
#define SDIO_CLKCR   (*(volatile uint32_t *)0x40012C04)
#define SDIO_CMD     (*(volatile uint32_t *)0x40012C0C)
#define SDIO_STA     (*(volatile uint32_t *)0x40012C34)
#define SDIO_ICR     (*(volatile uint32_t *)0x40012C38)

#define DCMI_CR      (*(volatile uint32_t *)0x50050000)
#define DCMI_SR      (*(volatile uint32_t *)0x50050004)

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

void init_periphs() {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

void setup() {
    init_periphs();
    tx_s("=== NEW PERIPH TEST ===\n");

    // PWR
    PWR_CR = 0x1FFF;
    CHECK(PWR_CR == 0x1FFF, "PWR CR write/read");
    CHECK(PWR_CSR == 0, "PWR CSR default 0");
    PWR_CR = 0x00;
    CHECK(PWR_CR == 0x00, "PWR CR cleared");

    // WWDG
    CHECK(WWDG_CR == 0x7F, "WWDG CR default");
    WWDG_CR = 0xAA;
    CHECK(WWDG_CR == 0xAA, "WWDG CR write");
    WWDG_CFR = 0x7FFF;
    CHECK(WWDG_CFR == 0x7FFF, "WWDG CFR write");
    CHECK(WWDG_SR == 0, "WWDG SR default 0");

    // IWDG
    CHECK(IWDG_SR == 0, "IWDG SR default");
    IWDG_KR = 0x5555;
    IWDG_PR = 0x05;
    CHECK(IWDG_PR == 0x05, "IWDG PR write with KR=5555");
    IWDG_RLR = 0xABC;
    CHECK(IWDG_RLR == 0xABC, "IWDG RLR write");
    IWDG_KR = 0xAAAA;
    IWDG_KR = 0xCCCC;

    // RTC
    CHECK(RTC_ISR & 1, "RTC ISR init mode");
    RTC_TR = 0x00592359;
    CHECK(RTC_TR == 0x00592359, "RTC TR write");
    RTC_DR = 0x00210631;
    CHECK(RTC_DR == 0x00210631, "RTC DR write");
    RTC_BKP0 = 0xA5A5A5A5;
    CHECK(RTC_BKP0 == 0xA5A5A5A5, "RTC BKP0 write/read");
    CHECK(RTC_PRER == 0x007F00FF, "RTC PRER default");

    // CRC — real CRC32 calculation
    CRC_CR = 1;
    CRC_DR = 0x12345678;
    uint32_t crc_val = CRC_DR;
    CHECK(crc_val != 0x12345678 && crc_val != 0xFFFFFFFF, "CRC computes");
    CRC_IDR = 0xAB;
    CHECK(CRC_IDR == 0xAB, "CRC IDR write");
    CRC_CR = 1;
    CHECK(CRC_DR == 0xFFFFFFFF, "CRC reset via CR");

    // RNG
    RNG_CR = 4;
    CHECK(RNG_SR & 1, "RNG SR DRDY after enable");
    uint32_t rv = RNG_DR;
    CHECK(!(RNG_SR & 1), "RNG DR clears DRDY");

    // DAC
    DAC_CR = 0x3F3FFFFF;
    CHECK(DAC_CR == 0x3F3FFFFF, "DAC CR write");
    DAC_DHR12R1 = 0x7FF;
    uint32_t dor = DAC_DOR1;
    CHECK(dor == 0x7FF, "DAC DHR12R1 -> DOR1");
    DAC_DHR12R1 = 0;
    CHECK(DAC_DOR1 == 0, "DAC DOR1 follows DHR12R1");

    // CAN
    CHECK(CAN_MCR & 2, "CAN MCR SLEEP bit");
    CAN_MCR = 0x10;
    CHECK((CAN_MCR & 0x7F3F) == 0x10, "CAN MCR write");
    CAN_BTR = 0x3FFFFFFF;
    CHECK(CAN_BTR == 0x3FFFFFFF, "CAN BTR write");
    CAN_IER = 0x7FF;
    CHECK(CAN_IER == 0x7FF, "CAN IER write");
    CHECK(CAN_ESR == 0, "CAN ESR default 0");

    // SDIO
    SDIO_CLKCR = 0x3FFF;
    CHECK(SDIO_CLKCR == 0x3FFF, "SDIO CLKCR write");
    SDIO_CMD = 0x40 | 5;
    CHECK(SDIO_STA & (1 << 6), "SDIO CMDSENT after CMD");
    SDIO_ICR = (1 << 6);
    CHECK(!(SDIO_STA & (1 << 6)), "SDIO ICR clears CMDSENT");

    // DCMI
    DCMI_CR = 0x7FFF3FFF;
    CHECK(DCMI_CR == 0x7FFF3FFF, "DCMI CR write");
    CHECK(DCMI_SR == 0, "DCMI SR default 0");

    tx_s("---- SUMMARY ----\n");
    tx_s("PASS: "); tx_hex(pass); tx_s("\n");
    tx_s("FAIL: "); tx_hex(fail); tx_s("\n");
    tx_s("=== DONE ===\n");
}

void loop() {}
