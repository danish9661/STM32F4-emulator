// oled_test: SSD1306 128x64 OLED over I2C1 (PB8 SCL, PB9 SDA, AF4).
// Draws "F407 OLED" text + color-bar-style pattern into the display RAM.
// The JS hardware layer (site/emulator.js OLED device) parses the I2C
// traffic and renders the framebuffer on a canvas.

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

#define OLED_ADDR   0x3C

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

static void i2c_init(void) {
    RCC_APB1ENR |= (1 << 21);                    // I2C1 clock
    RCC_AHB1ENR |= (1 << 1);                     // GPIOB clock
    GPIOB_MODER = (GPIOB_MODER & ~((3u << 16) | (3u << 18))) | ((2u << 16) | (2u << 18));
    GPIOB_AFRL  = (GPIOB_AFRL & ~0xFF00) | (4 << 12) | (4 << 16); // PB8/PB9 AF4
    I2C_CR1 = 0;
    I2C_CR2 = 42;                                // 42 MHz -> ~400 kHz
    I2C_CR1 = (1 << 0);                          // PE
}

// Write `len` bytes to the OLED in one I2C transaction (START, addr, data).
// The model's I2C state machine mirrors the real F407: after the address
// byte, SR1 then SR2 must be read for the AddrSent->Active transition.
static void i2c_oled_write(const unsigned char *data, int len) {
    I2C_CR1 |= (1 << 8);                         // START
    while (!(I2C_SR1 & (1 << 0)));               // SB
    I2C_DR = (OLED_ADDR << 1);                   // address (write)
    while (!(I2C_SR1 & (1 << 1)));               // ADDR
    (void)I2C_SR1;                               // read SR1 (latch)
    (void)I2C_SR2;                               // read SR2 (Active)
    for (int i = 0; i < len; i++) {
        while (!(I2C_SR1 & (1 << 6)));           // model TX-complete bit
        I2C_DR = data[i];
    }
    I2C_CR1 |= (1 << 9);                         // STOP
    I2C_CR1 &= ~(1 << 8);
}

static void oled_cmd(unsigned char cmd) {
    const unsigned char b[2] = { 0x00, cmd };
    i2c_oled_write(b, 2);
}

static void oled_data(const unsigned char *data, int len) {
    static unsigned char buf[129];
    buf[0] = 0x40;                               // data control byte
    for (int i = 0; i < len && i < 128; i++) buf[1 + i] = data[i];
    i2c_oled_write(buf, 1 + (len > 128 ? 128 : len));
}

// 5x7 font for a handful of characters
static const unsigned char font5x7[][5] = {
    {0x7E,0x09,0x09,0x09,0x7E},  // 0x41 'A'
    {0x7F,0x49,0x49,0x49,0x36},  // 0x42 'B'
    {0x3E,0x41,0x41,0x41,0x22},  // 0x43 'C'
    {0x7F,0x41,0x41,0x22,0x1C},  // 0x44 'D'
    {0x7F,0x49,0x49,0x49,0x41},  // 0x45 'E'
    {0x7F,0x09,0x09,0x09,0x01},  // 0x46 'F'
    {0x3E,0x41,0x41,0x51,0x72},  // 0x47 'G'
    {0x7F,0x08,0x08,0x08,0x7F},  // 0x48 'H'
    {0x00,0x41,0x7F,0x41,0x00},  // 0x49 'I'
    {0x20,0x40,0x41,0x3F,0x01},  // 0x4A 'J'
    {0x7F,0x08,0x14,0x22,0x41},  // 0x4B 'K'
    {0x7F,0x40,0x40,0x40,0x40},  // 0x4C 'L'
    {0x7F,0x02,0x0C,0x02,0x7F},  // 0x4D 'M'
    {0x7F,0x04,0x08,0x10,0x7F},  // 0x4E 'N'
    {0x3E,0x41,0x41,0x41,0x3E},  // 0x4F 'O'
    {0x7F,0x09,0x09,0x09,0x06},  // 0x50 'P'
    {0x3E,0x41,0x51,0x21,0x5E},  // 0x51 'Q'
    {0x7F,0x09,0x19,0x29,0x46},  // 0x52 'R'
    {0x46,0x49,0x49,0x49,0x31},  // 0x53 'S'
    {0x01,0x01,0x7F,0x01,0x01},  // 0x54 'T'
    {0x3F,0x40,0x40,0x40,0x3F},  // 0x55 'U'
    {0x1F,0x20,0x40,0x20,0x1F},  // 0x56 'V'
    {0x7F,0x20,0x18,0x20,0x7F},  // 0x57 'W'
    {0x63,0x14,0x08,0x14,0x63},  // 0x58 'X'
    {0x03,0x04,0x78,0x04,0x03},  // 0x59 'Y'
    {0x61,0x51,0x49,0x45,0x43},  // 0x5A 'Z'
    {0x00,0x36,0x36,0x00,0x00},  // 0x20 ' '
    {0x00,0x00,0x7F,0x00,0x00},  // 0x21 '!'
    {0x00,0x5B,0x00,0x00,0x00},  // 0x22 '"'
    {0x14,0x7F,0x14,0x7F,0x14},  // 0x23 '#'
    {0x24,0x2A,0x7F,0x2A,0x12},  // 0x24 '$'
    {0x23,0x13,0x08,0x64,0x62},  // 0x25 '%'
    {0x36,0x49,0x55,0x22,0x50},  // 0x26 '&'
    {0x00,0x05,0x03,0x00,0x00},  // 0x27 '''
    {0x00,0x1C,0x22,0x41,0x00},  // 0x28 '('
    {0x00,0x41,0x22,0x1C,0x00},  // 0x29 ')'
    {0x14,0x08,0x3E,0x08,0x14},  // 0x2A '*'
    {0x08,0x08,0x3E,0x08,0x08},  // 0x2B '+'
    {0x00,0x50,0x30,0x00,0x00},  // 0x2C ','
    {0x08,0x08,0x08,0x08,0x08},  // 0x2D '-'
    {0x00,0x60,0x60,0x00,0x00},  // 0x2E '.'
    {0x20,0x10,0x08,0x04,0x02},  // 0x2F '/'
    {0x3E,0x51,0x49,0x45,0x3E},  // 0x30 '0'
    {0x00,0x42,0x7F,0x40,0x00},  // 0x31 '1'
    {0x42,0x61,0x51,0x49,0x46},  // 0x32 '2'
    {0x21,0x41,0x45,0x4B,0x31},  // 0x33 '3'
    {0x18,0x14,0x12,0x7F,0x10},  // 0x34 '4'
    {0x27,0x45,0x45,0x45,0x39},  // 0x35 '5'
    {0x3C,0x4A,0x49,0x49,0x30},  // 0x36 '6'
    {0x01,0x71,0x09,0x05,0x03},  // 0x37 '7'
    {0x36,0x49,0x49,0x49,0x36},  // 0x38 '8'
    {0x06,0x49,0x49,0x29,0x1E},  // 0x39 '9'
    {0x00,0x36,0x36,0x00,0x00},  // 0x3A ':'
};

static void oled_putchar(unsigned char c, int x, int page) {
    const unsigned char *g = font5x7[0];         // default 'A'
    if (c >= 0x41 && c <= 0x5A) g = font5x7[c - 0x41];
    else if (c >= 0x30 && c <= 0x39) g = font5x7[0x30 - 0x41 + (c - 0x30)];
    else if (c == ' ') g = font5x7[0x5A - 0x41 + 1];
    else if (c == '!') g = font5x7[0x5A - 0x41 + 2];
    else if (c == ':') g = font5x7[0x5A - 0x41 + 3];
    unsigned char line[7];
    oled_cmd(0xB0 | page);
    oled_cmd(x & 0x0F);
    oled_cmd(0x10 | ((x >> 4) & 0x0F));
    for (int i = 0; i < 5; i++) line[i] = g[i];
    line[5] = 0; line[6] = 0;
    oled_data(line, 6);
}

static void oled_puts(const char *s, int x, int page) {
    while (*s) { oled_putchar(*s++, x, page); x += 6; }
}

static void oled_fill(unsigned char pattern) {
    unsigned char pagebuf[128];
    for (int p = 0; p < 8; p++) {
        oled_cmd(0xB0 | p);
        oled_cmd(0x00);
        oled_cmd(0x10);
        for (int i = 0; i < 128; i++) pagebuf[i] = pattern;
        oled_data(pagebuf, 128);
    }
}

static void oled_init(void) {
    oled_cmd(0xAE);          // display off
    oled_cmd(0x8D); oled_cmd(0x14);  // charge pump on
    oled_cmd(0xA8); oled_cmd(0x3F);  // multiplex 1/64
    oled_cmd(0xD3); oled_cmd(0x00);  // display offset 0
    oled_cmd(0x40);          // start line 0
    oled_cmd(0xA1);          // segment remap
    oled_cmd(0xC8);          // COM scan direction
    oled_cmd(0xDA); oled_cmd(0x12);  // COM pins
    oled_cmd(0x81); oled_cmd(0xCF);  // contrast
    oled_cmd(0xD5); oled_cmd(0x80);  // clock div
    oled_cmd(0xD9); oled_cmd(0xF1);  // precharge
    oled_cmd(0xDB); oled_cmd(0x40);  // VCOMH
    oled_cmd(0xA4);          // resume to RAM content
    oled_cmd(0xA6);          // normal display (not inverted)
    oled_cmd(0x20); oled_cmd(0x00);  // horizontal addressing? no: page mode
    oled_cmd(0xAF);          // display on
}

int main(void) {
    uart_init();
    uart_puts("OLED test\r\n");
    uart_puts("SSD1306 128x64 @ I2C1 (0x3C)\r\n");
    uart_puts("OLED init\r\n");
    i2c_init();
    oled_init();
    uart_puts("OLED init done\r\n");

    oled_fill(0x00);         // clear
    oled_puts("F407 OLED", 4, 0);
    oled_puts("Hello from", 4, 3);
    oled_puts("STM32F407", 4, 5);

    // bottom bar: solid block
    unsigned char bar[128];
    for (int i = 0; i < 128; i++) bar[i] = 0xFF;
    oled_cmd(0xB0 | 7);
    oled_cmd(0x00);
    oled_cmd(0x10);
    oled_data(bar, 128);

    uart_puts("OLED draw done\r\n");
    for (;;) delay_ms(1000);
}
