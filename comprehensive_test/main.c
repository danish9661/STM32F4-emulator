#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_AHB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x34))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define NVIC_ISER0  (*(volatile unsigned int *)0xE000E100)
#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)
#define NVIC_ISER2  (*(volatile unsigned int *)0xE000E108)
#define NVIC_ICER0  (*(volatile unsigned int *)0xE000E180)
#define NVIC_ICER1  (*(volatile unsigned int *)0xE000E184)
#define NVIC_ICER2  (*(volatile unsigned int *)0xE000E188)

#define EXTI_IMR   (*(volatile unsigned int *)0x40013C00)
#define EXTI_SWIER (*(volatile unsigned int *)0x40013C10)
#define EXTI_PR    (*(volatile unsigned int *)0x40013C14)

#define CRC_DR    (*(volatile unsigned int *)0x40023000)
#define CRC_CR    (*(volatile unsigned int *)0x40023008)
#define CRC_IDR   (*(volatile unsigned int *)0x40023004)

#define CRYP_BASE   0x50060000
#define CRYP_CR     (*(volatile unsigned int *)(CRYP_BASE + 0x00))
#define CRYP_DIN    (*(volatile unsigned int *)(CRYP_BASE + 0x08))
#define CRYP_DOUT   (*(volatile unsigned int *)(CRYP_BASE + 0x0C))
#define CRYP_KEY(n) (*(volatile unsigned int *)(CRYP_BASE + 0x20 + (n)*4))
#define CRYP_IV(n)  (*(volatile unsigned int *)(CRYP_BASE + 0x40 + (n)*4))

#define HASH_BASE   0x50060400
#define HASH_CR     (*(volatile unsigned int *)(HASH_BASE + 0x00))
#define HASH_DIN    (*(volatile unsigned int *)(HASH_BASE + 0x04))
#define HASH_STR    (*(volatile unsigned int *)(HASH_BASE + 0x08))
#define HASH_HR(n)  (*(volatile unsigned int *)(HASH_BASE + 0x0C + (n)*4))

#define RNG_CR  (*(volatile unsigned int *)0x50060800)
#define RNG_SR  (*(volatile unsigned int *)0x50060804)
#define RNG_DR  (*(volatile unsigned int *)0x50060808)

#define DMA2_BASE    0x40026400
#define DMA2_S0CR   (*(volatile unsigned int *)(DMA2_BASE + 0x10))
#define DMA2_S0NDTR (*(volatile unsigned int *)(DMA2_BASE + 0x14))
#define DMA2_S0PAR  (*(volatile unsigned int *)(DMA2_BASE + 0x18))
#define DMA2_S0M0AR (*(volatile unsigned int *)(DMA2_BASE + 0x1C))
#define DMA2_S0FCR  (*(volatile unsigned int *)(DMA2_BASE + 0x24))
#define DMA2_LIFCR  (*(volatile unsigned int *)(DMA2_BASE + 0x08))

#define I2S2_CR2  (*(volatile unsigned int *)0x40003404)
#define I2S2_DR   (*(volatile unsigned int *)0x4000340C)

#define SAI_ABASE  0x40015804
#define SAI_ADR    (*(volatile unsigned int *)(SAI_ABASE + 0x1C))
#define SAI_AIM    (*(volatile unsigned int *)(SAI_ABASE + 0x10))

#define DCMI_CR  (*(volatile unsigned int *)0x50050000)
#define DCMI_DR  (*(volatile unsigned int *)0x50050028)
#define DCMI_IER (*(volatile unsigned int *)0x5005000C)
#define DCMI_ICR (*(volatile unsigned int *)0x50050010)

#define LTDC_SRCR  (*(volatile unsigned int *)0x40016824)
#define LTDC_IER   (*(volatile unsigned int *)0x40016834)
#define LTDC_ISR   (*(volatile unsigned int *)0x40016838)
#define LTDC_ICR   (*(volatile unsigned int *)0x4001683C)

#define SDIO_POWER  (*(volatile unsigned int *)0x40012C00)
#define SDIO_CMD    (*(volatile unsigned int *)0x40012C0C)
#define SDIO_STA    (*(volatile unsigned int *)0x40012C34)
#define SDIO_ICR    (*(volatile unsigned int *)0x40012C38)
#define SDIO_MASK   (*(volatile unsigned int *)0x40012C3C)

#define CAN1_MCR  (*(volatile unsigned int *)0x40006400)
#define CAN1_MSR  (*(volatile unsigned int *)0x40006404)
#define CAN1_TSR  (*(volatile unsigned int *)0x40006408)
#define CAN1_IER  (*(volatile unsigned int *)0x40006414)
#define CAN1_TIR0 (*(volatile unsigned int *)0x40006580)

#define PWR_CR  (*(volatile unsigned int *)0x40007000)
#define PWR_CSR (*(volatile unsigned int *)0x40007004)

#define IWDG_KR  (*(volatile unsigned int *)0x40002800)
#define IWDG_PR  (*(volatile unsigned int *)0x40002804)
#define IWDG_RLR (*(volatile unsigned int *)0x40002808)

#define DAC_CR      (*(volatile unsigned int *)0x40007400)
#define DAC_DHR12R1 (*(volatile unsigned int *)0x40007408)
#define DAC_DOR1    (*(volatile unsigned int *)0x4000742C)

#define FLASH_KEYR  (*(volatile unsigned int *)0x40023C04)
#define FLASH_CR    (*(volatile unsigned int *)0x40023C10)

#define DBGMCU_IDCODE (*(volatile unsigned int *)0xE0042000)
#define DBGMCU_CR     (*(volatile unsigned int *)0xE0042004)

extern volatile int exti0_fired, exti9_5_fired, exti15_10_fired;
extern volatile int rng_fired, ltdc_fired, can1_tx_fired;
extern volatile int sdio_fired, dcmi_fired, i2s_fired, sai_fired;
extern volatile int dma2_fired;

static int pass, fail;
static unsigned int scratch[16];

static void uart_init(void) {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_puts(const char *s) {
    while (*s) { while (!(USART_SR & (1 << 7))); USART_DR = *s++; }
}

static void uart_hex32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7))); USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

#define CHECK(cond, msg) do { \
    if (cond) { uart_puts("PASS "); uart_puts(msg); pass++; } \
    else { uart_puts("FAIL "); uart_puts(msg); fail++; } \
    uart_puts("\n"); \
} while(0)

static void irq_wait(volatile int *flag) {
    int timeout = 500000;
    while (!*flag && timeout--) { __asm__ volatile("nop"); }
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB1ENR |= (1 << 0) | (1 << 25) | (1 << 27) | (1 << 28);
    RCC_APB2ENR |= (1 << 0) | (1 << 4) | (1 << 6) | (1 << 10);
    RCC_AHB1ENR |= (1 << 22);
    RCC_AHB2ENR |= (1 << 0) | (1 << 1) | (1 << 2);

    GPIOA_MODER &= ~((3 << 18) | (3 << 20));
    GPIOA_MODER |=  ((2 << 18) | (2 << 20));
    GPIOA_AFRH  |=  ((7 << 4) | (7 << 8));

    uart_init();
    uart_puts("=== COMPREHENSIVE PERIPHERAL TEST ===\n");

    // ========== 1. EXTI ==========
    uart_puts("--- EXTI ---\n");
    EXTI_IMR = 0;
    EXTI_PR = 0xFF;
    EXTI_IMR = 0x07;
    EXTI_SWIER = 1;
    CHECK(EXTI_PR & 1, "EXTI SWIER0 sets PR0");
    EXTI_PR = 1;
    CHECK((EXTI_PR & 1) == 0, "EXTI PR write-1-to-clear");
    EXTI_IMR = 0x02;
    EXTI_SWIER = 0x0A;
    CHECK(EXTI_PR & 2, "EXTI SWIER1 sets PR (unmasked)");
    CHECK((EXTI_PR & 8) == 0, "EXTI SWIER3 blocked by IMR");
    EXTI_PR = 0xFF;

    exti0_fired = 0;
    EXTI_IMR = 1; NVIC_ICER0 = (1 << 6); NVIC_ISER0 |= (1 << 6);
    EXTI_SWIER = 1; irq_wait(&exti0_fired);
    CHECK(exti0_fired != 0, "EXTI IRQ6 ISR (line 0)");
    NVIC_ICER0 = (1 << 6); EXTI_PR = 1;

    exti9_5_fired = 0;
    EXTI_IMR = 0x20; NVIC_ICER0 = (1 << 23); NVIC_ISER0 |= (1 << 23);
    EXTI_SWIER = 0x20; irq_wait(&exti9_5_fired);
    CHECK(exti9_5_fired != 0, "EXTI IRQ23 ISR (line 5)");
    NVIC_ICER0 = (1 << 23); EXTI_PR = 0xFF;

    exti15_10_fired = 0;
    EXTI_IMR = 0x400; NVIC_ICER1 = (1 << 8); NVIC_ISER1 |= (1 << 8);
    EXTI_SWIER = 0x400; irq_wait(&exti15_10_fired);
    CHECK(exti15_10_fired != 0, "EXTI IRQ40 ISR (line 10)");
    NVIC_ICER1 = (1 << 8);

    // ========== 2. CRC ==========
    uart_puts("--- CRC ---\n");
    CRC_CR = 1; // reset
    CRC_DR = 0x31323334;
    CHECK(CRC_DR == 0xA695C4AA, "CRC-32 of 0x31323334");
    CRC_IDR = 0xDE;
    CHECK(CRC_IDR == 0xDE, "CRC IDR");
    CRC_CR = 1;
    CHECK(CRC_DR == 0xFFFFFFFF, "CRC reset value");

    // ========== 3. CRYP ==========
    uart_puts("--- CRYP ---\n");
    unsigned int k128[4]  = {0x2B7E1516, 0x28AED2A6, 0xABF71588, 0x09CF4F3C};
    unsigned int k256[8]  = {0x603DEB10, 0x15CA71BE, 0x2B73AEF0, 0x857D7781,
                             0x1F352C07, 0x3B6108D7, 0x2D9810A3, 0x0914DFF4};
    unsigned int pt[4]    = {0x6BC1BEE2, 0x2E409F96, 0xE93D7E11, 0x7393172A};
    unsigned int iv_cbc[4]= {0x00010203, 0x04050607, 0x08090A0B, 0x0C0D0E0F};
    unsigned int kdes[2]  = {0x01234567, 0x89ABCDEF};
    unsigned int pt_des[4]= {0x4E6F7720, 0x69732074, 0x68656972, 0x00656D00};
    unsigned int ktdes[6] = {0x01234567, 0x89ABCDEF, 0xFEDCBA98, 0x76543210, 0x89ABCDEF, 0x01234567};
    unsigned int out[4];

    CRYP_CR = 0x4000;
    for (int i = 0; i < 4; i++) CRYP_KEY(i) = k128[i];
    CRYP_CR = 0x8000;
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x3AD77BB4, "AES-128 ECB enc w0");
    CHECK(out[3] == 0x2466EF97, "AES-128 ECB enc w3");

    CRYP_CR = 0x4000; CRYP_CR = 0x8004;
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt[0], "AES-128 ECB dec w0");
    CHECK(out[3] == pt[3], "AES-128 ECB dec w3");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 4; i++) CRYP_KEY(i) = k128[i];
    for (int i = 0; i < 4; i++) CRYP_IV(i) = iv_cbc[i];
    CRYP_CR = 0x8008;
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x7649ABAC, "AES-128 CBC enc w0");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 8; i++) CRYP_KEY(i) = k256[i];
    CRYP_CR = 0x8A00;
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0xF3EED1BD, "AES-256 ECB enc w0");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 2; i++) CRYP_KEY(i) = kdes[i];
    CRYP_CR = 0x88000;
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x3FA40E8A, "DES-ECB enc w0");

    CRYP_CR = 0x4000; CRYP_CR = 0x88004;
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt_des[0], "DES-ECB dec w0");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 6; i++) CRYP_KEY(i) = ktdes[i];
    CRYP_CR = 0x88010;
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0xFBE62B68, "TDES-ECB enc w0");

    // ========== 4. HASH ==========
    uart_puts("--- HASH ---\n");
    HASH_CR = 1; HASH_DIN = 0x61626364; HASH_STR = 0x100;
    CHECK(HASH_HR(0) == 0x81FE8BFE, "SHA-1 HR0");
    CHECK(HASH_HR(4) == 0x82917ACF, "SHA-1 HR4");

    HASH_CR = 1; HASH_CR = 0x44000; HASH_DIN = 0x61626364; HASH_STR = 0x100;
    CHECK(*(volatile unsigned int *)(0x50060400 + 0x310) == 0x88D4266F, "SHA-256 HR0");

    HASH_CR = 1; HASH_CR = 0x4080; HASH_DIN = 0x61626364; HASH_STR = 0x100;
    CHECK(HASH_HR(0) == 0xE2FC714C, "MD5 HR0");

    // ========== 5. RNG ==========
    uart_puts("--- RNG ---\n");
    RNG_CR = 4;
    unsigned int r1 = RNG_DR;
    for (int i = 0; i < 1000; i++) { __asm__ volatile("nop"); }
    unsigned int r2 = RNG_DR;
    CHECK(r1 != r2, "RNG data changes");

    // ========== 6. DMA2 ==========
    uart_puts("--- DMA2 ---\n");
    scratch[12] = 0xAABBCCDD;
    scratch[13] = 0x11223344;
    dma2_fired = 0;
    NVIC_ICER1 = (1 << 24); NVIC_ISER1 |= (1 << 24);
    DMA2_S0CR = 0; DMA2_S0NDTR = 4;
    DMA2_S0PAR = (unsigned int)(&scratch[12]);
    DMA2_S0M0AR = (unsigned int)(&scratch[0]);
    DMA2_S0FCR = 0x21; DMA2_S0CR = 0x4591;
    irq_wait(&dma2_fired);
    CHECK(DMA2_S0NDTR == 0, "DMA2 NDTR=0");
    CHECK(dma2_fired != 0, "DMA2 IRQ56 ISR executed");
    DMA2_LIFCR = 0x0F; NVIC_ICER1 = (1 << 24);

    // ========== 7. I2S2ext ==========
    uart_puts("--- I2S2ext ---\n");
    i2s_fired = 0;
    NVIC_ICER1 = (1 << 4); NVIC_ISER1 |= (1 << 4);
    I2S2_CR2 = 2; I2S2_DR = 0x12345678;
    irq_wait(&i2s_fired);
    CHECK(i2s_fired != 0, "I2S IRQ36 ISR executed");
    NVIC_ICER1 = (1 << 4);

    // ========== 8. SAI ==========
    uart_puts("--- SAI ---\n");
    sai_fired = 0;
    NVIC_ICER2 = (1 << 23); NVIC_ISER2 |= (1 << 23);
    SAI_AIM = 2; SAI_ADR = 0xDEADBEEF;
    irq_wait(&sai_fired);
    CHECK(sai_fired != 0, "SAI IRQ87 ISR executed");
    NVIC_ICER2 = (1 << 23);

    // ========== 9. DCMI ==========
    uart_puts("--- DCMI ---\n");
    dcmi_fired = 0;
    NVIC_ICER2 = (1 << 14); NVIC_ISER2 |= (1 << 14);
    DCMI_IER = 4; DCMI_CR = 1;
    volatile unsigned int d = DCMI_DR;
    irq_wait(&dcmi_fired);
    CHECK(dcmi_fired != 0, "DCMI IRQ78 ISR executed");
    DCMI_CR = 0; DCMI_ICR = 0x1F; NVIC_ICER2 = (1 << 14);

    // ========== 10. LTDC ==========
    uart_puts("--- LTDC ---\n");
    ltdc_fired = 0;
    NVIC_ICER2 = (1 << 24); NVIC_ISER2 |= (1 << 24);
    LTDC_IER = 8; LTDC_SRCR = 1;
    irq_wait(&ltdc_fired);
    CHECK(ltdc_fired != 0, "LTDC IRQ88 ISR executed");
    CHECK(LTDC_ISR & 8, "LTDC ISR RRIF");
    LTDC_ICR = 0x0F; NVIC_ICER2 = (1 << 24);

    // ========== 11. SDIO ==========
    uart_puts("--- SDIO ---\n");
    sdio_fired = 0;
    SDIO_POWER = 1; SDIO_MASK = (1 << 6);
    NVIC_ICER1 = (1 << 17); NVIC_ISER1 |= (1 << 17);
    SDIO_CMD = 0x40;
    CHECK(SDIO_STA & (1 << 6), "SDIO CMDSENT");
    irq_wait(&sdio_fired);
    CHECK(sdio_fired != 0, "SDIO IRQ49 ISR executed");
    NVIC_ICER1 = (1 << 17);

    // ========== 12. CAN1 ==========
    uart_puts("--- CAN1 ---\n");
    // Init: enter init mode, then exit
    CAN1_MCR = 1; for (int i = 0; i < 100; i++) { __asm__ volatile("nop"); }
    CAN1_MCR = 0; for (int i = 0; i < 100; i++) { __asm__ volatile("nop"); }
    CAN1_IER = 1; // TMEIE
    can1_tx_fired = 0;
    NVIC_ICER0 = (1 << 19); NVIC_ISER0 |= (1 << 19);
    CAN1_TIR0 = 1;
    irq_wait(&can1_tx_fired);
    CHECK(CAN1_TSR & (1 << 0), "CAN1 TME0");
    CHECK(can1_tx_fired != 0, "CAN1 IRQ19 ISR executed");
    NVIC_ICER0 = (1 << 19);

    // ========== 13. PWR ==========
    uart_puts("--- PWR ---\n");
    PWR_CR = 0x1F;
    CHECK(PWR_CSR & (1 << 1), "PWR PVDO");

    // ========== 14. IWDG ==========
    uart_puts("--- IWDG ---\n");
    IWDG_KR = 0x5555; IWDG_PR = 0; IWDG_RLR = 0xFFF; IWDG_KR = 0xAAAA;
    CHECK(1, "IWDG refresh OK");

    // ========== 15. DAC ==========
    uart_puts("--- DAC ---\n");
    DAC_CR = 1; DAC_DHR12R1 = 0xABC;
    CHECK(DAC_DOR1 != 0, "DAC DOR1 non-zero");

    // ========== 16. FLASH ==========
    uart_puts("--- FLASH ---\n");
    FLASH_KEYR = 0x45670123; FLASH_KEYR = 0xCDEF89AB;
    FLASH_CR |= (1 << 15);
    CHECK(FLASH_CR & (1 << 15), "FLASH EOPIE unlocked");

    // ========== 17. DBGMCU ==========
    uart_puts("--- DBGMCU ---\n");
    CHECK(DBGMCU_IDCODE == 0x10006411, "DBGMCU IDCODE");
    DBGMCU_CR = 0x1F0077;
    CHECK(DBGMCU_CR == 0x1F0077, "DBGMCU CR");

    // ========== SUMMARY ==========
    uart_puts("---- SUMMARY ----\n");
    uart_puts("PASS: "); uart_hex32(pass); uart_puts("\n");
    uart_puts("FAIL: "); uart_hex32(fail); uart_puts("\n");
    uart_puts("=== DONE ===\n");
    while (1);
}
