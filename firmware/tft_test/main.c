// tft_test: ILI9341 240x320 TFT over SPI2 (PB12 CS, PB11 DC, PB13 SCK,
// PB14 MISO, PB15 MOSI AF5). Initializes the controller and fills the
// screen with a 4-color quadrant pattern. The JS hardware layer
// (site/emulator.js TFT device) parses the SPI byte stream using the DC
// line and renders the framebuffer on a canvas.

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_APB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x40))

#define GPIOB_BASE  0x40020400
#define GPIOB_MODER (*(volatile unsigned int *)(GPIOB_BASE + 0x00))
#define GPIOB_AFRL  (*(volatile unsigned int *)(GPIOB_BASE + 0x20))
#define GPIOB_AFRH  (*(volatile unsigned int *)(GPIOB_BASE + 0x24))
#define GPIO_BSRR(base) (*(volatile unsigned int *)((base) + 0x18))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

#define SPI2_BASE   0x40003800
#define SPI2_CR1    (*(volatile unsigned int *)(SPI2_BASE + 0x00))
#define SPI2_SR     (*(volatile unsigned int *)(SPI2_BASE + 0x08))
#define SPI2_DR     (*(volatile unsigned int *)(SPI2_BASE + 0x0C))

#define CS_PIN  12
#define DC_PIN  11

#define TFT_W   240
#define TFT_H   320

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

static void delay_ms(int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

static void cs_low(void) { GPIO_BSRR(GPIOB_BASE) = (1 << (CS_PIN + 16)); }
static void cs_high(void) { GPIO_BSRR(GPIOB_BASE) = (1 << CS_PIN); }
static void dc_cmd(void)  { GPIO_BSRR(GPIOB_BASE) = (1 << (DC_PIN + 16)); }
static void dc_data(void) { GPIO_BSRR(GPIOB_BASE) = (1 << DC_PIN); }

static void spi2_xfer(unsigned char byte) {
    SPI2_DR = byte;
    while (!(SPI2_SR & (1 << 0)));   // RXNE
    (void)SPI2_DR;
}

static void tft_cmd(unsigned char cmd) {
    dc_cmd();
    spi2_xfer(cmd);
}

static void tft_data(const unsigned char *data, int len) {
    dc_data();
    for (int i = 0; i < len; i++) spi2_xfer(data[i]);
}

static void tft_init(void) {
    cs_low();
    tft_cmd(0x01); delay_ms(5);      // SWRESET
    tft_cmd(0x11); delay_ms(10);     // SLPOUT
    tft_cmd(0x36); { unsigned char d[1] = { 0x00 }; tft_data(d, 1); }  // MADCTL
    tft_cmd(0x3A); { unsigned char d[1] = { 0x55 }; tft_data(d, 1); }  // COLMOD RGB565
    tft_cmd(0x29);                    // DISPON
    cs_high();
}

static void tft_fill_quadrants(void) {
    cs_low();
    tft_cmd(0x2A); { unsigned char d[4] = { 0x00, 0x00, 0x00, 0xEF }; tft_data(d, 4); }
    tft_cmd(0x2B); { unsigned char d[4] = { 0x00, 0x00, 0x01, 0x3F }; tft_data(d, 4); }
    tft_cmd(0x2C);
    dc_data();
    for (int y = 0; y < TFT_H; y++) {
        for (int x = 0; x < TFT_W; x++) {
            unsigned int c;
            if (x < TFT_W / 2 && y < TFT_H / 2) c = 0xF800;        // red
            else if (x >= TFT_W / 2 && y < TFT_H / 2) c = 0x07E0;  // green
            else if (x < TFT_W / 2) c = 0x001F;                    // blue
            else c = 0xFFFF;                                       // white
            spi2_xfer((c >> 8) & 0xFF);
            spi2_xfer(c & 0xFF);
        }
    }
    cs_high();
}

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_AHB1ENR |= (1 << 1);
    RCC_APB1ENR |= (1 << 14);        // SPI2

    uart_init();
    uart_puts("TFT ILI9341 test\r\n");
    uart_puts("SPI2 240x320 RGB565\r\n");

    // PB12 CS, PB11 DC outputs
    GPIOB_MODER = (GPIOB_MODER & ~((3u << 22) | (3u << 24))) | ((1u << 22) | (1u << 24));
    cs_high();
    dc_data();

    // PB13 SCK, PB14 MISO, PB15 MOSI -> AF5
    GPIOB_AFRH = (GPIOB_AFRH & ~0xFFF0) | (5 << 20) | (5 << 24) | (5 << 28);
    GPIOB_MODER = (GPIOB_MODER & ~((3u << 26) | (3u << 28) | (3u << 30))) | ((2u << 26) | (2u << 28) | (2u << 30));

    SPI2_CR1 = (1 << 2) | (1 << 8) | (1 << 9) | (2 << 3);  // MSTR | SSI | SSM | BR /4
    SPI2_CR1 |= (1 << 6);                                  // SPE

    uart_puts("TFT init\r\n");
    tft_init();
    uart_puts("TFT init done\r\n");
    tft_fill_quadrants();
    uart_puts("TFT fill done\r\n");
    for (;;) delay_ms(1000);
}
