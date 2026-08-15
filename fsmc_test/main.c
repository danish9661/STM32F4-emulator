// FSMC memory-mapped display test (polling, no interrupts).
//
// Drives an Intel-8080-mode display on FSMC BANK1 the way real firmware
// does: the bank's data window is just memory, and one address line is
// wired to the display's RS/DC pin, so a store to the bank base is a
// COMMAND and a store to base|(1<<RS_SHIFT) is DATA. Nothing about the
// protocol lives in the peripheral model — the JS device on the other side
// of the bank tap (site/test_fsmc.mjs) decodes these accesses.
//
// Sequence:
//   1. enable the FSMC clock, configure BCR1/BTR1 for a 16-bit SRAM-like
//      bank, and print the register read-back (proves the control regs are
//      real storage, not the data window).
//   2. issue CASET + RAMWR with a small pixel burst.
//   3. issue RDDID and read the bank back: the JS device answers that read
//      from its own queue, which is the direction that used to be a stub.
//   4. write to an UNTAPPED bank (BANK4) and read it back: must be inert.
#define RCC_AHB3ENR (*(volatile unsigned int *)0x40023838)

// FSMC control registers live at 0xA0000000 (peripheral offset 0x40000000).
#define FSMC_BCR1   (*(volatile unsigned int *)0xA0000000)
#define FSMC_BTR1   (*(volatile unsigned int *)0xA0000004)

// BANK1 data window. A16 is the RS/DC line: low = command, high = data.
#define RS_SHIFT    17
#define LCD_CMD     (*(volatile unsigned short *)0x60000000)
#define LCD_DATA    (*(volatile unsigned short *)(0x60000000 + (1u << RS_SHIFT)))
// Untapped bank, for the inertness check. NOTE the model splits the four
// banks every 0x1000_0000 (BANK1 0x6000_0000, BANK2 0x7000_0000, BANK3
// 0x8000_0000, BANK4 0x9000_0000), not at real silicon's 64 MB NOR/SRAM
// sub-bank boundaries — 0x6C00_0000 is still BANK1 here and would be read
// as a command write by the display on the other side.
#define BANK4_CELL  (*(volatile unsigned short *)0x90000000)

static void uart_init(void);
static void uart_puts(const char *s);
static void uart_hex16(unsigned int v);

static void lcd_cmd(unsigned short c) { LCD_CMD = c; }
static void lcd_data(unsigned short d) { LCD_DATA = d; }

int main(void) {
    uart_init();
    uart_puts("=== FSMC Test ===\r\n");

    RCC_AHB3ENR |= (1u << 0);          // FSMC clock

    // 16-bit bus, SRAM type, bank enabled. The model stores these verbatim
    // (BTR1 is masked to 30 bits), which is all a display driver needs.
    FSMC_BCR1 = (1u << 0) | (1u << 4) | (1u << 12);   // MBKEN | MWID=16 | WREN
    FSMC_BTR1 = 0x00001053;

    uart_puts("BCR1=");
    uart_hex16(FSMC_BCR1 & 0xFFFF);
    uart_puts(" BTR1=");
    uart_hex16(FSMC_BTR1 & 0xFFFF);
    uart_puts("\r\n");
    if ((FSMC_BCR1 & 0x1011) != 0x1011) {
        uart_puts("FSMC regs FAIL\r\n=== FSMC Test: FAIL ===\r\n");
        while (1);
    }

    // ── an ILI9341-style window set + pixel burst ──
    lcd_cmd(0x2A);                      // CASET
    lcd_data(0x0000);
    lcd_data(0x00EF);
    lcd_cmd(0x2C);                      // RAMWR
    for (int i = 0; i < 6; i++) lcd_data(0xF800 + i);
    uart_puts("burst sent\r\n");

    // ── a read the device answers ──
    lcd_cmd(0x04);                      // RDDID
    // The JS device sees the command and queues its reply; give it a moment
    // of guest time so the host loop can run its handler between steps.
    for (volatile unsigned int i = 0; i < 200000; i++);
    unsigned short id = LCD_DATA;
    uart_puts("id=");
    uart_hex16(id);
    uart_puts("\r\n");
    if (id != 0x9341) {
        uart_puts("FSMC read FAIL\r\n=== FSMC Test: FAIL ===\r\n");
        while (1);
    }
    uart_puts("FSMC read OK\r\n");

    // ── an untapped bank must be inert, not an alias of bank 1 ──
    BANK4_CELL = 0xDEAD;
    unsigned short back = BANK4_CELL;
    uart_puts("bank4=");
    uart_hex16(back);
    uart_puts("\r\n");
    if (back != 0) {
        uart_puts("FSMC bank4 FAIL\r\n=== FSMC Test: FAIL ===\r\n");
        while (1);
    }

    uart_puts("=== FSMC Test: done ===\r\n");
    while (1);
}

static void uart_init(void) {
    *(volatile unsigned int *)0x40023830 |= (1 << 0); // GPIOA
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
    *(volatile unsigned int *)0x40011008 = 16000000 / 115200;
    *(volatile unsigned int *)0x4001100C = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(*(volatile unsigned int *)0x40011000 & (1 << 7)));
    *(volatile unsigned int *)0x40011004 = c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_hex16(unsigned int v) {
    for (int i = 3; i >= 0; i--) {
        unsigned int nib = (v >> (i * 4)) & 0xF;
        uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}
