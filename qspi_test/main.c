// qspi_test: minimal firmware exercising the modeled QUADSPI peripheral at
// 0xA0001000. Performs indirect write then read of several 32-bit words and
// verifies the round-trip, printing the result over USART1 (115200 8N1).
// A QSPI flash image must be registered by the driver (qspi_register_flash)
// before the firmware runs; we write known values and read them back.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

// QUADSPI (modeled at 0xA0001000, conventional base; F4 silicon lacks it).
#define QSPI_BASE   0xA0001000
#define QSPI_CR     (*(volatile unsigned int *)(QSPI_BASE + 0x00))
#define QSPI_DCR    (*(volatile unsigned int *)(QSPI_BASE + 0x04))
#define QSPI_SR     (*(volatile unsigned int *)(QSPI_BASE + 0x08))
#define QSPI_FCR    (*(volatile unsigned int *)(QSPI_BASE + 0x0C))
#define QSPI_DLR    (*(volatile unsigned int *)(QSPI_BASE + 0x10))
#define QSPI_CCR    (*(volatile unsigned int *)(QSPI_BASE + 0x14))
#define QSPI_AR     (*(volatile unsigned int *)(QSPI_BASE + 0x18))
#define QSPI_DR     (*(volatile unsigned int *)(QSPI_BASE + 0x20))

#define QSPI_SR_TC  (1u << 1)
#define QSPI_SR_BUSY (1u << 5)

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);                       // GPIOA clock
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // USART1 clock (APB2)
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA;   // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
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
    char buf[12];
    int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

static unsigned int uart_hex(unsigned int v) {
    char buf[10];
    int i = 0;
    const char *h = "0123456789ABCDEF";
    if (v == 0) { uart_putchar('0'); return 0; }
    while (v) { buf[i++] = h[v & 0xF]; v >>= 4; }
    while (i) uart_putchar(buf[--i]);
    return 0;
}

// Indirect write of a 32-bit word to the given flash address.
static int qspi_write(unsigned int addr, unsigned int val) {
    unsigned int t;
    QSPI_CR = 1;                 // enable
    QSPI_DCR = 0;
    QSPI_AR = addr;
    QSPI_DLR = 3;                // 4 bytes (DLR = len-1)
    QSPI_CCR = (0u << 28) | (1u << 24) | 0x02; // FMODE=write, DMODE=1-line, cmd 0x02
    QSPI_DR = val;               // pushes the word, completes the transfer
    t = 0;
    while (!(QSPI_SR & QSPI_SR_TC) && t < 1000000) t++;
    QSPI_FCR = QSPI_SR_TC;       // clear TC
    return (t >= 1000000) ? -1 : 0;
}

// Indirect read of a 32-bit word from the given flash address.
static int qspi_read(unsigned int addr, unsigned int *out) {
    unsigned int t;
    QSPI_CR = 1;
    QSPI_DCR = 0;
    QSPI_AR = addr;
    QSPI_DLR = 3;
    QSPI_CCR = (1u << 28) | (1u << 24) | 0x03; // FMODE=read, DMODE=1-line, cmd 0x03
    unsigned int v = QSPI_DR;    // pops the word, completes the transfer
    t = 0;
    while (!(QSPI_SR & QSPI_SR_TC) && t < 1000000) t++;
    QSPI_FCR = QSPI_SR_TC;
    if (t >= 1000000) return -1;
    *out = v;
    return 0;
}

int main(void) {
    uart_init();
    uart_puts("=== QSPI Test ===\r\n");
    uart_puts("QUADSPI @ 0xA0001000\r\n");

    // Values to round-trip through the (driver-provided) flash backend.
    unsigned int vals[4] = { 0xDEADBEEF, 0x12345678, 0x00FF00FF, 0xAAAAAAAA };
    int ok = 1;
    int i;
    for (i = 0; i < 4; i++) {
        if (qspi_write(i * 4, vals[i]) != 0) {
            uart_puts("write timeout @");
            uart_u32(i * 4);
            uart_puts("\r\n");
            ok = 0;
            break;
        }
        unsigned int r = 0;
        if (qspi_read(i * 4, &r) != 0) {
            uart_puts("read timeout @");
            uart_u32(i * 4);
            uart_puts("\r\n");
            ok = 0;
            break;
        }
        uart_puts("addr ");
        uart_hex(i * 4);
        uart_puts(" wrote ");
        uart_hex(vals[i]);
        uart_puts(" read ");
        uart_hex(r);
        uart_puts("\r\n");
        if (r != vals[i]) {
            uart_puts("MISMATCH\r\n");
            ok = 0;
            break;
        }
    }

    if (ok) {
        uart_puts("QSPI OK\r\n");
    } else {
        uart_puts("QSPI FAIL\r\n");
    }
    uart_puts("QSPI Test done\r\n");

    for (;;) {
        // idle so the driver can stop the run once markers are observed
        __asm__ volatile ("nop");
    }
}
