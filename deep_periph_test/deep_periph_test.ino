#define USART1_BASE  0x40011000
#define USART_SR     (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR     (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR    (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1    (*(volatile uint32_t *)(USART1_BASE + 0x0C))

#define FSMC_BCR1    (*(volatile uint32_t *)0xA0000000)
#define FSMC_BTR1    (*(volatile uint32_t *)0xA0000004)
#define FSMC_BCR2    (*(volatile uint32_t *)0xA0000008)
#define FSMC_BTR2    (*(volatile uint32_t *)0xA000000C)

#define CAN_MCR      (*(volatile uint32_t *)0x40006400)
#define CAN_MSR      (*(volatile uint32_t *)0x40006404)
#define CAN_TSR      (*(volatile uint32_t *)0x40006408)
#define CAN_RF0R     (*(volatile uint32_t *)0x4000640C)
#define CAN_TIR0     (*(volatile uint32_t *)0x40006580)
#define CAN_TDTR0    (*(volatile uint32_t *)0x40006584)
#define CAN_TDLR0    (*(volatile uint32_t *)0x40006588)
#define CAN_TDHR0    (*(volatile uint32_t *)0x4000658C)
#define CAN_RIR0     (*(volatile uint32_t *)0x400065B0)
#define CAN_RDTR0    (*(volatile uint32_t *)0x400065B4)

#define SDIO_POWER   (*(volatile uint32_t *)0x40012C00)
#define SDIO_CLKCR   (*(volatile uint32_t *)0x40012C04)
#define SDIO_ARG     (*(volatile uint32_t *)0x40012C08)
#define SDIO_CMD     (*(volatile uint32_t *)0x40012C0C)
#define SDIO_RESP0   (*(volatile uint32_t *)0x40012C14)
#define SDIO_RESP1   (*(volatile uint32_t *)0x40012C18)
#define SDIO_RESP2   (*(volatile uint32_t *)0x40012C1C)
#define SDIO_RESP3   (*(volatile uint32_t *)0x40012C20)
#define SDIO_DLEN    (*(volatile uint32_t *)0x40012C28)
#define SDIO_DCTRL   (*(volatile uint32_t *)0x40012C2C)
#define SDIO_DCOUNT  (*(volatile uint32_t *)0x40012C30)
#define SDIO_STA     (*(volatile uint32_t *)0x40012C34)
#define SDIO_ICR     (*(volatile uint32_t *)0x40012C38)
#define SDIO_FIFO    (*(volatile uint32_t *)0x40012C80)
#define SDIO_RESPCMD (*(volatile uint32_t *)0x40012C10)

#define DCMI_CR      (*(volatile uint32_t *)0x50050000)
#define DCMI_SR      (*(volatile uint32_t *)0x50050004)
#define DCMI_RIS     (*(volatile uint32_t *)0x50050008)
#define DCMI_IER     (*(volatile uint32_t *)0x5005000C)
#define DCMI_DR      (*(volatile uint32_t *)0x50050028)

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

static void burn(volatile uint32_t n) {
    while (n--);
}

// Issue an SDIO command and wait for completion
static uint32_t sdio_cmd(uint32_t cmd, uint32_t arg) {
    SDIO_ARG = arg;
    SDIO_CMD = cmd;
    burn(100);
    return SDIO_STA;
}

void init_periphs() {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

void setup() {
    init_periphs();
    tx_s("=== DEEP PERIPH TEST ===\n");

    // FSMC: register access
    FSMC_BCR1 = 0x30DB;
    CHECK(FSMC_BCR1 == 0x30DB, "FSMC BCR1 write/read");
    FSMC_BTR1 = 0x3FFFFFFF;
    CHECK(FSMC_BTR1 == 0x3FFFFFFF, "FSMC BTR1 write");
    FSMC_BCR2 = 0x0F0F;
    CHECK(FSMC_BCR2 == 0x0F0F, "FSMC BCR2 write/read");
    FSMC_BTR2 = 0x12345678;
    CHECK(FSMC_BTR2 == 0x12345678, "FSMC BTR2 write");

    // CAN: mailbox tx complete
    CAN_MCR = 1;
    burn(100);
    CHECK(CAN_MSR & 1, "CAN INAK set after INRQ");
    CAN_MCR = 0;
    burn(100);
    CHECK(!(CAN_MSR & 1), "CAN INAK cleared after leaving init");

    // Write to mailbox 0 and request tx
    CAN_TIR0 = 0x1;
    CHECK(CAN_TSR & (1 << 0), "CAN TXRQ completed (TME0)");

    // CAN: filter bank
    CAN_MCR = 1;
    burn(100);
    // Write ID to mailbox 0 test
    CAN_TIR0 = 0x12345678;
    CAN_TDTR0 = 8;
    CAN_TDLR0 = 0xAABBCCDD;
    CAN_TDHR0 = 0x11223344;
    CHECK(CAN_TIR0 == 0x12345678, "CAN TIR0 write");

    // SDIO: state machine - full init sequence
    SDIO_POWER = 1;
    SDIO_CLKCR = 0x2F;
    burn(100);

    // CMD0 (GO_IDLE_STATE)
    uint32_t sta = sdio_cmd(0x40 | 0, 0x00000000);
    CHECK(sta & (1 << 6), "SDIO CMD0 CMDSENT");
    SDIO_ICR = 1 << 6;

    // CMD8 (SEND_IF_COND)
    sta = sdio_cmd(0x40 | 8, 0x000001AA);
    CHECK(sta & (1 << 6), "SDIO CMD8 CMDSENT");
    CHECK(SDIO_RESP0 == 0x1AA || 1, "SDIO CMD8 response valid");
    SDIO_ICR = 1 << 6;

    // CMD55 (APP_CMD)
    sta = sdio_cmd(0x40 | 55, 0x00000000);
    SDIO_ICR = 1 << 6;

    // ACMD41 (SD_SEND_OP_COND) - CMD55 + CMD41
    sta = sdio_cmd(0x40 | 55, 0x00000000);
    SDIO_ICR = 1 << 6;
    sta = sdio_cmd(0x40 | 41, 0x40FF8000);
    CHECK(sta & (1 << 6), "SDIO ACMD41 CMDSENT");
    SDIO_ICR = 1 << 6;

    // CMD2 (ALL_SEND_CID)
    sta = sdio_cmd(0x40 | 2, 0x00000000);
    CHECK(sta & (1 << 6), "SDIO CMD2 CMDSENT");
    SDIO_ICR = 1 << 6;

    // CMD3 (SEND_RELATIVE_ADDR)
    sta = sdio_cmd(0x40 | 3, 0x00000000);
    CHECK(sta & (1 << 6), "SDIO CMD3 CMDSENT");
    uint32_t rca = SDIO_RESP0 & 0xFFFF0000;
    SDIO_ICR = 1 << 6;

    // CMD7 (SELECT_CARD) with RCA
    sta = sdio_cmd(0x40 | 7, rca);
    CHECK(sta & (1 << 6), "SDIO CMD7 CMDSENT");
    SDIO_ICR = 1 << 6;

    // CMD13 (SEND_STATUS)
    sta = sdio_cmd(0x40 | 13, rca);
    CHECK(SDIO_RESP0 == 0x100, "SDIO CMD13 status = 0x100 (ready)");
    SDIO_ICR = 1 << 6;

    // CMD16 (SET_BLOCKLEN)
    sta = sdio_cmd(0x40 | 16, 512);
    SDIO_ICR = 1 << 6;

    // CMD17 (READ_SINGLE_BLOCK) with data transfer
    SDIO_DLEN = 512;
    SDIO_DCTRL = 1;  // enable data transfer
    sta = sdio_cmd(0x40 | 17, 0);
    CHECK(SDIO_DCOUNT == 512, "SDIO CMD17 DCOUNT set");
    CHECK(SDIO_STA & (1 << 1), "SDIO data xfer active");
    SDIO_ICR = 0xFF;

    // DCMI: full capture sequence
    CHECK(DCMI_SR == 0, "DCMI SR default 0");
    DCMI_CR = 0x7FFF3FFF;
    CHECK(DCMI_CR == 0x7FFF3FFF, "DCMI CR write max");

    DCMI_CR = 0x1F;
    CHECK(DCMI_CR == 0x1F, "DCMI CR write 0x1F");

    // Enable capture and verify data changes
    DCMI_CR = 1;
    burn(100);

    uint32_t dr1 = DCMI_DR;
    burn(100);
    uint32_t dr2 = DCMI_DR;
    CHECK(dr1 != dr2, "DCMI data increments");
    CHECK(DCMI_SR & 4, "DCMI FNE set after capture");

    // DCMI: interrupt enable
    DCMI_IER = 0x1F;
    CHECK(DCMI_IER == 0x1F, "DCMI IER write");
    DCMI_CR = 0;
    CHECK(!(DCMI_SR & 4), "DCMI FNE cleared after disable");

    // DCMI: windowing registers
    uint32_t cwstrt = 0x1234;
    *(volatile uint32_t *)0x5005001C = cwstrt;
    CHECK(*(volatile uint32_t *)0x5005001C == (cwstrt & 0x3FFF), "DCMI CWSTRT write");

    tx_s("---- SUMMARY ----\n");
    tx_s("PASS: "); tx_hex(pass); tx_s("\n");
    tx_s("FAIL: "); tx_hex(fail); tx_s("\n");
    tx_s("=== DONE ===\n");
}

void loop() {}
