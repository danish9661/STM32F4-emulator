extern int main(void);
extern void _start(void);

// ISR flags — set by handlers, checked by tests
volatile int exti0_fired, exti1_fired, exti2_fired;
volatile int exti9_5_fired, exti15_10_fired;
volatile int rng_fired;
volatile int ltdc_fired;
volatile int can1_tx_fired;
volatile int sdio_fired;
volatile int dcmi_fired;
volatile int i2s_fired;
volatile int sai_fired;
volatile int dma2_fired;

__attribute__((interrupt)) void Default_Handler(void) { while (1); }

void EXTI0_IRQHandler(void) {
    exti0_fired = 1;
    *(volatile unsigned int *)0x40013C14 = 1; // EXTI_PR bit 0
}
void EXTI1_IRQHandler(void) { exti1_fired = 1; *(volatile unsigned int *)0x40013C14 = 2; }
void EXTI2_IRQHandler(void) { exti2_fired = 1; *(volatile unsigned int *)0x40013C14 = 4; }
void EXTI9_5_IRQHandler(void) { exti9_5_fired = 1; *(volatile unsigned int *)0x40013C14 = 0x3E0; }
void EXTI15_10_IRQHandler(void) { exti15_10_fired = 1; *(volatile unsigned int *)0x40013C14 = 0xFC00; }
void RNG_IRQHandler(void) { rng_fired = 1; }
void LTDC_IRQHandler(void) { ltdc_fired = 1; }
void CAN1_TX_IRQHandler(void) { can1_tx_fired = 1; }
void SDIO_IRQHandler(void) { sdio_fired = 1; *(volatile unsigned int *)0x40012C38 = 0xFFFFFFFF; }
void DCMI_IRQHandler(void) { dcmi_fired = 1; *(volatile unsigned int *)0x50050010 = 0x1F; }
void SPI2_IRQHandler(void) { i2s_fired = 1; }
void SAI_IRQHandler(void) { sai_fired = 1; }
void DMA2_Stream0_IRQHandler(void) {
    dma2_fired = 1;
    *(volatile unsigned int *)0x40026408 = 0x1F; // DMA2 LIFCR clear stream 0
}

__attribute__((used, section(".vectors")))
void (* const vector_table[97 + 16])(void) = {
    // System exceptions (16)
    (void (*)(void))0x20020000, // SP
    _start,                     // Reset
    Default_Handler,            // NMI
    Default_Handler,            // HardFault
    Default_Handler,            // MemManage
    Default_Handler,            // BusFault
    Default_Handler,            // UsageFault
    0, 0, 0, 0,                // Reserved
    Default_Handler,            // SVC
    Default_Handler,            // DebugMon
    0,                          // Reserved
    Default_Handler,            // PendSV
    Default_Handler,            // SysTick
    // IRQ handlers (97)
    Default_Handler,            // 0: WWDG
    Default_Handler,            // 1: PVD
    Default_Handler,            // 2: TAMP_STAMP
    Default_Handler,            // 3: RTC_WKUP
    Default_Handler,            // 4: FLASH
    Default_Handler,            // 5: RCC
    EXTI0_IRQHandler,           // 6: EXTI0
    EXTI1_IRQHandler,           // 7: EXTI1
    EXTI2_IRQHandler,           // 8: EXTI2
    Default_Handler,            // 9: EXTI3
    Default_Handler,            // 10: EXTI4
    Default_Handler,            // 11: DMA1_Stream0
    Default_Handler,            // 12: DMA1_Stream1
    Default_Handler,            // 13: DMA1_Stream2
    Default_Handler,            // 14: DMA1_Stream3
    Default_Handler,            // 15: DMA1_Stream4
    Default_Handler,            // 16: DMA1_Stream5
    Default_Handler,            // 17: DMA1_Stream6
    Default_Handler,            // 18: ADC
    CAN1_TX_IRQHandler,         // 19: CAN1_TX
    Default_Handler,            // 20: CAN1_RX0
    Default_Handler,            // 21: CAN1_RX1
    Default_Handler,            // 22: CAN1_SCE
    EXTI9_5_IRQHandler,         // 23: EXTI9_5
    Default_Handler,            // 24: TIM1_BRK_TIM9
    Default_Handler,            // 25: TIM1_UP_TIM10
    Default_Handler,            // 26: TIM1_TRG_COM_TIM11
    Default_Handler,            // 27: TIM1_CC
    Default_Handler,            // 28: TIM2
    Default_Handler,            // 29: TIM3
    Default_Handler,            // 30: TIM4
    Default_Handler,            // 31: I2C1_EV
    Default_Handler,            // 32: I2C1_ER
    Default_Handler,            // 33: I2C2_EV
    Default_Handler,            // 34: I2C2_ER
    Default_Handler,            // 35: SPI1
    SPI2_IRQHandler,            // 36: SPI2
    Default_Handler,            // 37: SPI3
    Default_Handler,            // 38: USART1
    Default_Handler,            // 39: USART2
    EXTI15_10_IRQHandler,       // 40: EXTI15_10
    Default_Handler,            // 41: RTC_Alarm
    Default_Handler,            // 42: OTG_FS_WKUP
    Default_Handler,            // 43: TIM8_BRK_TIM12
    Default_Handler,            // 44: TIM8_UP_TIM13
    Default_Handler,            // 45: TIM8_TRG_COM_TIM14
    Default_Handler,            // 46: TIM8_CC
    Default_Handler,            // 47: DMA1_Stream7
    Default_Handler,            // 48: FSMC
    SDIO_IRQHandler,            // 49: SDIO
    Default_Handler,            // 50: TIM5
    Default_Handler,            // 51: SPI3
    Default_Handler,            // 52: UART4
    Default_Handler,            // 53: UART5
    Default_Handler,            // 54: TIM6_DAC
    Default_Handler,            // 55: TIM7
    DMA2_Stream0_IRQHandler,    // 56: DMA2_Stream0
    Default_Handler,            // 57: DMA2_Stream1
    Default_Handler,            // 58: DMA2_Stream2
    Default_Handler,            // 59: DMA2_Stream3
    Default_Handler,            // 60: DMA2_Stream4
    Default_Handler,            // 61: CAN2_TX
    Default_Handler,            // 62: CAN2_RX0
    Default_Handler,            // 63: CAN2_RX1
    Default_Handler,            // 64: CAN2_SCE
    Default_Handler,            // 65: OTG_FS
    Default_Handler,            // 66: DMA2_Stream5
    Default_Handler,            // 67: DMA2_Stream6
    Default_Handler,            // 68: DMA2_Stream7
    Default_Handler,            // 69: USART6
    Default_Handler,            // 70: I2C3_EV
    Default_Handler,            // 71: I2C3_ER
    Default_Handler,            // 72: OTG_HS_EP1_OUT
    Default_Handler,            // 73: OTG_HS_EP1_IN
    Default_Handler,            // 74: OTG_HS_WKUP
    Default_Handler,            // 75: OTG_HS
    Default_Handler,            // 76: DCMI
    Default_Handler,            // 77:
    DCMI_IRQHandler,            // 78: DCMI (on some maps)
    Default_Handler,            // 79: CRYP
    RNG_IRQHandler,             // 80: HASH_RNG
    Default_Handler,            // 81: FPU
    Default_Handler,            // 82:
    Default_Handler,            // 83:
    Default_Handler,            // 84:
    Default_Handler,            // 85:
    Default_Handler,            // 86:
    SAI_IRQHandler,             // 87: SAI
    LTDC_IRQHandler,            // 88: LTDC
    Default_Handler,            // 89: LTDC_ER
    Default_Handler,            // 90: DMA2D
    Default_Handler,            // 91:
    Default_Handler,            // 92:
    Default_Handler,            // 93:
    Default_Handler,            // 94:
    Default_Handler,            // 95:
    Default_Handler,            // 96:
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
