// rtc_test: DS3231 RTC over I2C1 (PB8 SCL, PB9 SDA, AF4) at 0x68.
// Sets the time registers (pointer + auto-increment write), reads them
// back with a pointer-then-read transaction, verifies, and reads the
// temperature register pair. The register file lives in the model
// (ext_devices i2c_regfile); this exercises real DS3231 bus semantics:
// the pointer persists across transactions and reads stream sequentially.
//
// NOTE: startup.c does NOT zero .bss — keep globals static-initialized.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))

#define GPIOB_BASE  0x40020400
#define GPIOB_MODER (*(volatile unsigned int *)(GPIOB_BASE + 0x00))
#define GPIOB_AFRL  (*(volatile unsigned int *)(GPIOB_BASE + 0x20))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define I2C1_BASE   0x40005400
#define I2C_CR1     (*(volatile unsigned int *)(I2C1_BASE + 0x00))
#define I2C_CR2     (*(volatile unsigned int *)(I2C1_BASE + 0x04))
#define I2C_DR      (*(volatile unsigned int *)(I2C1_BASE + 0x10))
#define I2C_SR1     (*(volatile unsigned int *)(I2C1_BASE + 0x14))
#define I2C_SR2     (*(volatile unsigned int *)(I2C1_BASE + 0x18))

#define RTC_ADDR    0x68

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

static void uart_put2(unsigned char v) {
    uart_putchar('0' + (v / 10));
    uart_putchar('0' + (v % 10));
}

static void delay_ms(int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

static void i2c_init(void) {
    RCC_APB1ENR |= (1 << 21);                    // I2C1 clock
    RCC_AHB1ENR |= (1 << 1);                     // GPIOB clock
    GPIOB_MODER = (GPIOB_MODER & ~((3u << 16) | (3u << 18))) | ((2u << 16) | (2u << 18));
    GPIOB_AFRL  = (GPIOB_AFRL & ~0xFF00) | (4 << 12) | (4 << 16); // PB8/PB9 AF4
    I2C_CR1 = 0;
    I2C_CR2 = 42;                                // 42 MHz -> ~400 kHz
    I2C_CR1 = (1 << 0);                          // PE
}

// One write transaction: START, addr(0), then len bytes (first byte is the
// register pointer; the model's regfile auto-increments afterwards).
static void i2c_write(const unsigned char *data, int len) {
    I2C_CR1 |= (1 << 8);                         // START
    while (!(I2C_SR1 & (1 << 0)));               // SB
    I2C_DR = (RTC_ADDR << 1);                    // address (write)
    while (!(I2C_SR1 & (1 << 1)));               // ADDR
    (void)I2C_SR1;                               // read SR1 (latch)
    (void)I2C_SR2;                               // read SR2 (Active)
    for (int i = 0; i < len; i++) {
        while (!(I2C_SR1 & (1 << 6)));           // TXE
        I2C_DR = data[i];
    }
    I2C_CR1 |= (1 << 9);                         // STOP
    I2C_CR1 &= ~(1 << 8);
}

// One read transaction: START, addr(1), then len bytes (streams from the
// current register pointer, which persists from the last write transaction).
static void i2c_read(unsigned char *buf, int len) {
    I2C_CR1 |= (1 << 8);                         // START
    while (!(I2C_SR1 & (1 << 0)));               // SB
    I2C_DR = (RTC_ADDR << 1) | 1;                // address (read)
    while (!(I2C_SR1 & (1 << 1)));               // ADDR
    (void)I2C_SR1;                               // read SR1 (latch)
    (void)I2C_SR2;                               // read SR2 (Active, RXNE armed)
    for (int i = 0; i < len; i++) {
        while (!(I2C_SR1 & (1 << 5)));           // RXNE
        buf[i] = (unsigned char)I2C_DR;
    }
    I2C_CR1 |= (1 << 9);                         // STOP
    I2C_CR1 &= ~(1 << 8);
}

static unsigned char bin2bcd(unsigned char v) { return ((v / 10) << 4) | (v % 10); }
static unsigned char bcd2bin(unsigned char b) { return ((b >> 4) * 10) + (b & 0x0F); }

static unsigned char rtc_regs[8];

int main(void) {
    uart_init();
    uart_puts("RTC test\r\n");
    uart_puts("DS3231 @ I2C1 (0x68)\r\n");
    uart_puts("RTC init\r\n");
    i2c_init();

    // Set time: pointer 0x00 + 7 BCD bytes in one transaction.
    rtc_regs[0] = 0x00;
    rtc_regs[1] = bin2bcd(30);   // sec
    rtc_regs[2] = bin2bcd(45);   // min
    rtc_regs[3] = bin2bcd(10);   // hour
    rtc_regs[4] = 3;             // dow (raw, not BCD)
    rtc_regs[5] = bin2bcd(15);   // day
    rtc_regs[6] = bin2bcd(7);    // month
    rtc_regs[7] = bin2bcd(26);   // year
    i2c_write(rtc_regs, 8);
    uart_puts("RTC set done\r\n");

    // Read back: pointer transaction, then a streaming read transaction.
    rtc_regs[0] = 0x00;
    i2c_write(rtc_regs, 1);
    unsigned char t[7];
    i2c_read(t, 7);
    uart_puts("RTC read done\r\n");

    uart_puts("RTC time=");
    uart_put2(bcd2bin(t[2])); uart_putchar(':');
    uart_put2(bcd2bin(t[1])); uart_putchar(':');
    uart_put2(bcd2bin(t[0])); uart_putchar(' ');
    uart_puts("DOW="); uart_putchar('0' + (t[3] & 0x07)); uart_putchar(' ');
    uart_put2(bcd2bin(t[4])); uart_putchar('/');
    uart_put2(bcd2bin(t[5])); uart_putchar('/');
    uart_put2(bcd2bin(t[6])); uart_puts("\r\n");

    if (t[0] == rtc_regs[1] && t[1] == rtc_regs[2] && t[2] == rtc_regs[3] &&
        t[3] == rtc_regs[4] && t[4] == rtc_regs[5] && t[5] == rtc_regs[6] &&
        t[6] == rtc_regs[7]) {
        uart_puts("RTC verify OK\r\n");
    } else {
        uart_puts("RTC verify FAIL\r\n");
    }

    // Temperature: pointer 0x11, read MSB + LSB (0x25 C steps).
    rtc_regs[0] = 0x11;
    i2c_write(rtc_regs, 1);
    unsigned char t2[2];
    i2c_read(t2, 2);
    unsigned char tmsb = t2[0];
    unsigned char frac = (t2[1] >> 6) * 25;      // 0/25/50/75 hundredths
    unsigned char tmag = tmsb & 0x7F;
    uart_puts("RTC temp=");
    if (tmsb & 0x80) uart_putchar('-');
    uart_putchar('0' + (tmag / 10));
    uart_putchar('0' + (tmag % 10));
    uart_putchar('.');
    uart_putchar('0' + (frac / 10));
    uart_putchar('0' + (frac % 10));
    uart_puts("\r\n");

    uart_puts("RTC test done\r\n");
    for (;;) delay_ms(1000);
}
