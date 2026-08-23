// Deep-sleep demo: exercises the emulator's low-power (WFI/STOP) model.
// Arms the RTC alarm ~3s ahead, enters STOP (SLEEPDEEP) via WFI, and the
// emulator halts the core until the RTC alarm wakes it. After wakeup the
// firmware confirms via PWR->CSR WUF and blinks to show it is alive.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))
#define RCC_BDCR    (*(volatile unsigned int *)(RCC_BASE + 0x70))

#define PWR_BASE    0x40007000
#define PWR_CR      (*(volatile unsigned int *)(PWR_BASE + 0x00))
#define PWR_CSR     (*(volatile unsigned int *)(PWR_BASE + 0x04))

#define RTC_BASE    0x40002800
#define RTC_TR      (*(volatile unsigned int *)(RTC_BASE + 0x00))
#define RTC_CR      (*(volatile unsigned int *)(RTC_BASE + 0x08))
#define RTC_ISR     (*(volatile unsigned int *)(RTC_BASE + 0x0C))
#define RTC_PRER    (*(volatile unsigned int *)(RTC_BASE + 0x10))
#define RTC_ALRMAR  (*(volatile unsigned int *)(RTC_BASE + 0x1C))
#define RTC_WPR     (*(volatile unsigned int *)(RTC_BASE + 0x24))

#define SCB_SCR     (*(volatile unsigned int *)0xE000ED10)
#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)
#define NVIC_ICPR1  (*(volatile unsigned int *)0xE000E184)

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_ODR   (*(volatile unsigned int *)(GPIOA_BASE + 0x14))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define LED_PIN     5

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    *(volatile unsigned int *)0x40023844 |= (1 << 4);
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA;
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70;
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_u32(unsigned int v) {
    char buf[12]; int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

static void led_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    GPIOA_MODER = (GPIOA_MODER & ~(3u << (LED_PIN * 2))) | (1u << (LED_PIN * 2));
    GPIOA_ODR &= ~(1u << LED_PIN);
}

static void led_set(int on) {
    if (on) GPIOA_ODR |= (1u << LED_PIN);
    else    GPIOA_ODR &= ~(1u << LED_PIN);
}

// Add 3 seconds in BCD (seconds units only; assumes < 0x57 to start).
static unsigned int bcd_add3(unsigned int bcd) {
    unsigned int units = bcd & 0x0F;
    unsigned int tens = (bcd >> 4) & 0x07;
    units += 3;
    if (units >= 10) { units -= 10; tens += 1; }
    return (tens << 4) | units;
}

// Defined in startup.c; never runs here (demo polls WUF instead of using the
// interrupt pump), but provided so the vector table links and the alarm can be
// cleared if the pump is enabled.
void RTC_Alarm_IRQHandler(void) {
    uart_puts("ISR: RTC alarm\r\n");
    RTC_ISR &= ~(1u << 8);   // clear ALRAF
    RTC_CR &= ~(1u << 8);    // disable ALRAE (no re-fire)
    NVIC_ICPR1 |= (1 << 9);  // clear NVIC pending
}

int main(void) {
    uart_init();
    uart_puts("=== Deep Sleep Demo ===\r\n");
    uart_puts("STM32F407 low-power (WFI/STOP) + RTC alarm wakeup\r\n");

    led_init();

    // 1) Enable PWR and unlock the backup domain / RTC registers.
    RCC_APB1ENR |= (1u << 28);          // PWREN
    PWR_CR |= (1u << 8);                 // DBP: allow RTC register writes
    RTC_WPR = 0xCA;                      // unlock RTC write protection
    RTC_WPR = 0x53;

    // 2) Configure the RTC so 1 second of virtual time == WAKE_STEP instructions.
    RCC_BDCR |= (1u << 15);              // RTCEN
    RTC_PRER = (0u << 16) | 119999u;     // (async+1)*(sync+1) = 120000 ticks/sec
    RTC_ISR &= ~1u;                      // start the RTC counter (model gate)

    // 3) Arm the alarm ~3 seconds ahead (match seconds only).
    unsigned int cur_sec = RTC_TR & 0x7F;
    unsigned int alarm = bcd_add3(cur_sec);
    RTC_ALRMAR = (alarm & 0x7F)
               | (1u << 31) | (1u << 23) | (1u << 15); // MSK4/3/2 don't-care
    RTC_ISR &= ~(1u << 8);               // clear any stale ALRAF
    RTC_CR |= (1u << 8) | (1u << 12);    // ALRAE + ALRAIE

    // 4) Enable the RTC_Alarm interrupt as the wakeup source (IRQ 41 -> ISER1.9).
    NVIC_ISER1 |= (1u << 9);

    // 5) Enter STOP: PDDS=0, SLEEPDEEP=1 in SCB_SCR.
    PWR_CR &= ~(1u << 1);                // PDDS=0 -> STOP (not STANDBY)
    SCB_SCR |= (1u << 2);                // SLEEPDEEP

    uart_puts("arming RTC alarm ~3s ahead\r\n");
    uart_puts("entering STOP (deep sleep)...\r\n");

    // 6) Wait For Interrupt. The emulator halts the core here until the RTC
    //    alarm fires, then resumes.
    asm volatile (".short 0xBF30" : : : "memory"); // WFI

    // 7) Woke up. Confirm via the PWR wakeup flag.
    uart_puts("WOKE FROM STOP\r\n");
    if (PWR_CSR & (1u << 2)) {
        uart_puts("Wakeup flag (WUF) set\r\n");
    }
    // Tidy up so we don't immediately re-wake if we ever WFI again.
    PWR_CR |= (1u << 2);                 // CWUF: clear WUF
    RTC_ISR &= ~(1u << 8);               // clear ALRAF
    RTC_CR &= ~(1u << 8);                // disable ALRAE
    NVIC_ICPR1 |= (1u << 9);             // clear NVIC pending

    uart_puts("alive: blinking\r\n");
    unsigned int tick = 0;
    for (;;) {
        led_set(1);
        uart_puts("tick "); uart_u32(tick); uart_puts(" LED=ON\r\n");
        for (volatile int i = 0; i < 400000; i++);
        led_set(0);
        uart_puts("tick "); uart_u32(tick); uart_puts(" LED=OFF\r\n");
        for (volatile int i = 0; i < 400000; i++);
        tick++;
    }
}
