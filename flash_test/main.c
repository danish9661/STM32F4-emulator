#define FLASH_BASE 0x40023C00
#define FLASH_ACR    (*(volatile unsigned int *)(FLASH_BASE + 0x00))
#define FLASH_KEYR   (*(volatile unsigned int *)(FLASH_BASE + 0x04))
#define FLASH_SR     (*(volatile unsigned int *)(FLASH_BASE + 0x0C))
#define FLASH_CR     (*(volatile unsigned int *)(FLASH_BASE + 0x10))

#define FLASH_SR_EOP  (1 << 0)
#define FLASH_SR_BSY  (1 << 16)

#define FLASH_CR_PG   (1 << 0)
#define FLASH_CR_SER  (1 << 1)
#define FLASH_CR_MER  (1 << 2)
#define FLASH_CR_STRT (1 << 16)
#define FLASH_CR_LOCK (1 << 31)

#define RCC_BASE    0x40023800
#define RCC_APB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x44))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define SECTOR5_BASE 0x08020000
#define SECTOR5_DATA ((volatile unsigned int *)SECTOR5_BASE)

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

static void flash_unlock(void) {
    FLASH_KEYR = 0x45670123;
    FLASH_KEYR = 0xCDEF89AB;
}

static void flash_erase_sector(unsigned int sector) {
    FLASH_SR = FLASH_SR; // synchronize
    FLASH_CR = FLASH_CR_SER | ((sector & 0xF) << 3) | FLASH_CR_STRT;
    while (FLASH_SR & FLASH_SR_BSY);
    FLASH_SR = FLASH_SR_EOP; // clear EOP
}

int main(void) {
    uart_init();
    uart_puts("\r\n=== FLASH program/erase test ===\r\n");

    test("locked by default", (FLASH_CR & FLASH_CR_LOCK) != 0);

    flash_unlock();
    test("unlocked", (FLASH_CR & FLASH_CR_LOCK) == 0);

    // Erase sector 5 (0x08020000, 128KB)
    uart_puts("  erasing sector 5...\r\n");
    flash_erase_sector(5);
    test("sector 5 erased", SECTOR5_DATA[0] == 0xFFFFFFFF);

    // Program 4 words
    FLASH_CR = FLASH_CR_PG;
    SECTOR5_DATA[0] = 0x11223344;
    SECTOR5_DATA[1] = 0x55667788;
    SECTOR5_DATA[2] = 0x99AABBCC;
    SECTOR5_DATA[3] = 0x00000001;
    FLASH_CR = 0; // PG off
    test("programmed w0", SECTOR5_DATA[0] == 0x11223344);
    test("programmed w1", SECTOR5_DATA[1] == 0x55667788);
    test("programmed w2", SECTOR5_DATA[2] == 0x99AABBCC);
    test("programmed w3", SECTOR5_DATA[3] == 0x00000001);

    // Program a second time (non-erased flash keeps 1s)
    FLASH_CR = FLASH_CR_PG;
    SECTOR5_DATA[0] = 0x11223344;
    FLASH_CR = 0;
    test("reprogram same value", SECTOR5_DATA[0] == 0x11223344);

    // Erase again and verify gone
    flash_erase_sector(5);
    test("re-erased", SECTOR5_DATA[0] == 0xFFFFFFFF);

    // Relock
    FLASH_CR = FLASH_CR_LOCK;
    test("relocked", (FLASH_CR & FLASH_CR_LOCK) != 0);

    // A write while locked must not stick
    FLASH_CR = FLASH_CR_PG; // ignored while locked
    SECTOR5_DATA[1] = 0xDEADBEEF;
    test("locked write ignored", SECTOR5_DATA[1] == 0xFFFFFFFF);

    uart_puts("\r\n--- SUMMARY ---\r\n");
    uart_puts("PASS: ");
    uart_puthex(pass_count);
    uart_puts("\r\n");
    uart_puts("FAIL: ");
    uart_puthex(fail_count);
    uart_puts("\r\n");
    uart_puts("FLASH TEST DONE\r\n");
    while (1);
}
