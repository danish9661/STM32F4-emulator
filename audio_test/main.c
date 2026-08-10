// I2S/audio peripheral test (polling, no interrupts):
//  Phase A: DMA1 Stream0 PERIPH->MEM from I2S1_DR (0x4001300C). The WAV
//           source must be loaded into the model first (audio_load_wav on
//           the JS side); each 4-byte DMA chunk = one 16-bit sample + the
//           I2S_SR byte pair (the model answers 4-aligned chunk reads), so
//           samples live at buf[4*i] and buf[4*i+1]. Prints sample count +
//           a sum checksum over the samples.
//  Phase B: register writes to I2S1_DR (TX path) — each 16-bit write is
//           pushed into the model's capture FIFO (audio_take_capture on
//           the JS side asserts it).
#define I2S_BASE    0x40013000   // SPI1/I2S1 shared block
#define I2S_CR1     (*(volatile unsigned int *)(I2S_BASE + 0x00))
#define I2S_CR2     (*(volatile unsigned int *)(I2S_BASE + 0x04))
#define I2S_DR      (*(volatile unsigned int *)(I2S_BASE + 0x0C))
#define I2S_I2SCFGR (*(volatile unsigned int *)(I2S_BASE + 0x1C))

#define DMA1_BASE   0x40026000
#define DMA1_LISR   (*(volatile unsigned int *)(DMA1_BASE + 0x00))
#define DMA_S0CR    (*(volatile unsigned int *)(DMA1_BASE + 0x10))
#define DMA_S0NDTR  (*(volatile unsigned int *)(DMA1_BASE + 0x14))
#define DMA_S0PAR   (*(volatile unsigned int *)(DMA1_BASE + 0x18))
#define DMA_S0M0AR  (*(volatile unsigned int *)(DMA1_BASE + 0x1C))

#define RCC_AHB1ENR (*(volatile unsigned int *)0x40023830)
#define RCC_APB2ENR (*(volatile unsigned int *)0x40023844)

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define RX_N 64
#define TX_N 16

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // RCC APB2 USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
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

static void uart_hex32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        unsigned int nib = (v >> (i * 4)) & 0xF;
        uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

static void uart_dec(unsigned int v) {
    char buf[12]; int i = 0;
    if (v == 0) { uart_putchar('0'); return; }
    while (v) { buf[i++] = '0' + (v % 10); v /= 10; }
    while (i) uart_putchar(buf[--i]);
}

int main(void) {
    static unsigned char rxbuf[RX_N * 4];
    static unsigned short txbuf[TX_N];

    uart_init();
    uart_puts("=== Audio Test ===\r\n");
    RCC_AHB1ENR |= (1 << 21); // DMA1 clock
    RCC_APB2ENR |= (1 << 12); // SPI1/I2S1 clock

    // ===== Phase A: DMA PERIPH->MEM from I2S1_DR =====
    I2S_I2SCFGR = 1;              // I2SMOD: the shared block as I2S
    uart_puts("I2S1 DMA RX start\r\n");
    DMA_S0PAR = I2S_BASE + 0x0C;  // peripheral = I2S1_DR
    DMA_S0M0AR = (unsigned int)rxbuf;
    DMA_S0NDTR = RX_N;
    DMA_S0CR = (1 << 13) | (1 << 11) | 1; // MSIZE=16, PSIZE=16, PERIPH->MEM, EN
    // TCIF0 in this model's LISR sits at bit 4 of the stream-0 field.
    for (int i = 0; i < 2000000 && !(DMA1_LISR & (1 << 4)); i++); // TCIF0
    if (!(DMA1_LISR & (1 << 4))) {
        uart_puts("DMA RX timeout\r\n=== Audio Test: FAIL ===\r\n");
        while (1);
    }
    // PINC=0: the model re-reads I2S1_DR for every 4-byte chunk, so samples
    // sit at the start of each 4-byte group in the buffer.
    {
        unsigned int sum = 0;
        for (int i = 0; i < RX_N; i++)
            sum += (unsigned short)(rxbuf[2 * i] | (rxbuf[2 * i + 1] << 8));
        uart_puts("RX n=");
        uart_dec(RX_N);
        uart_puts(" sum=");
        uart_hex32(sum);
        uart_puts("\r\n");
        if (RX_N != 64 || sum != 0x93C40) {
            uart_puts("DMA RX FAIL\r\n=== Audio Test: FAIL ===\r\n");
            while (1);
        }
    }
    uart_puts("DMA RX OK\r\n");

    // ===== Phase B: I2S1_DR write path -> capture FIFO =====
    uart_puts("I2S1 TX start\r\n");
    for (int i = 0; i < TX_N; i++) {
        txbuf[i] = 1000 + i;
        I2S_DR = txbuf[i];
    }
    uart_puts("TX n=");
    uart_dec(TX_N);
    uart_puts(" OK\r\n");
    uart_puts("=== Audio Test: done ===\r\n");
    while (1);
}