#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))
#define RCC_AHB2ENR (*(volatile unsigned int *)(RCC_BASE + 0x34))

#define GPIOA_BASE  0x40020000
#define GPIOA_MODER (*(volatile unsigned int *)(GPIOA_BASE + 0x00))
#define GPIOA_AFRH  (*(volatile unsigned int *)(GPIOA_BASE + 0x24))

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

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

static int pass, fail;

static void uart_init(void) {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_puts(const char *s) {
    while (*s) { while (!(USART_SR & (1 << 7))); USART_DR = *s++; }
}

#define CHECK(cond, msg) do { \
    uart_puts(cond ? "PASS " : "FAIL "); uart_puts(msg); uart_puts("\n"); \
    if (cond) pass++; else fail++; \
} while(0)

int main(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_AHB2ENR |= (1 << 0) | (1 << 1);

    GPIOA_MODER &= ~((3 << 18) | (3 << 20));
    GPIOA_MODER |=  ((2 << 18) | (2 << 20));
    GPIOA_AFRH  |=  ((7 << 4) | (7 << 8));

    uart_init();
    uart_puts("=== CRYPTO DEEP TEST ===\n");

    unsigned int k128[4]  = {0x2B7E1516, 0x28AED2A6, 0xABF71588, 0x09CF4F3C};
    unsigned int pt[4]    = {0x6BC1BEE2, 0x2E409F96, 0xE93D7E11, 0x7393172A};
    unsigned int kdes[2]  = {0x01234567, 0x89ABCDEF};
    unsigned int pt_des[4]= {0x4E6F7720, 0x69732074, 0x68656972, 0x00656D00};
    unsigned int ktdes[6] = {0x01234567, 0x89ABCDEF, 0xFEDCBA98, 0x76543210, 0x89ABCDEF, 0x01234567};
    unsigned int iv_des[2]= {0x12345678, 0x90ABCDEF};
    unsigned int iv_ctr[4]= {0xF0F1F2F3, 0xF4F5F6F7, 0xF8F9FAFB, 0xFCFDFEFF};
    unsigned int out[4];

    // ===== AES-128 CTR (NIST SP 800-38A) =====
    uart_puts("--- AES-128 CTR ---\n");
    CRYP_CR = 0x4000;
    for (int i = 0; i < 4; i++) CRYP_KEY(i) = k128[i];
    for (int i = 0; i < 4; i++) CRYP_IV(i) = iv_ctr[i];
    CRYP_CR = 0x8010; // enable, ALGOMODE=010 (CTR)
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x874D6191, "AES-CTR w0");
    CHECK(out[1] == 0xB620E326, "AES-CTR w1");
    CHECK(out[2] == 0x1BEF6864, "AES-CTR w2");
    CHECK(out[3] == 0x990DB6CE, "AES-CTR w3");

    // CTR decrypt == encrypt (same operation)
    CRYP_CR = 0x4000;
    CRYP_CR = 0x8010;
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt[0], "AES-CTR decrypt w0");
    CHECK(out[1] == pt[1], "AES-CTR decrypt w1");
    CHECK(out[2] == pt[2], "AES-CTR decrypt w2");
    CHECK(out[3] == pt[3], "AES-CTR decrypt w3");

    // ===== DES-ECB (NIST-like) =====
    uart_puts("--- DES ECB ---\n");
    CRYP_CR = 0x4000;
    for (int i = 0; i < 2; i++) CRYP_KEY(i) = kdes[i];
    CRYP_CR = 0x88000; // enable + bit19 → DES-ECB
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x3FA40E8A, "DES-ECB w0");
    CHECK(out[1] == 0x984D4815, "DES-ECB w1");
    CHECK(out[2] == 0xB600F58E, "DES-ECB w2");
    CHECK(out[3] == 0x21CC557D, "DES-ECB w3");

    // DES-ECB decrypt
    CRYP_CR = 0x4000;
    CRYP_CR = 0x88004; // + ALGODIR=1 (decrypt)
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt_des[0], "DES-ECB decrypt w0");
    CHECK(out[1] == pt_des[1], "DES-ECB decrypt w1");
    CHECK(out[2] == pt_des[2], "DES-ECB decrypt w2");
    CHECK(out[3] == pt_des[3], "DES-ECB decrypt w3");

    // ===== DES-CBC =====
    uart_puts("--- DES CBC ---\n");
    CRYP_CR = 0x4000;
    for (int i = 0; i < 2; i++) CRYP_KEY(i) = kdes[i];
    for (int i = 0; i < 2; i++) CRYP_IV(i) = iv_des[i];
    CRYP_CR = 0x88008; // enable + bit19 + ALGOMODE=001 → DES-CBC
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0xE5C7CDDE, "DES-CBC w0");
    CHECK(out[1] == 0x872BF27C, "DES-CBC w1");
    CHECK(out[2] == 0x39C3201D, "DES-CBC w2");
    CHECK(out[3] == 0x2D8313B7, "DES-CBC w3");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 2; i++) CRYP_IV(i) = iv_des[i];
    CRYP_CR = 0x8800C; // + ALGODIR=1 (decrypt)
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt_des[0], "DES-CBC decrypt w0");
    CHECK(out[1] == pt_des[1], "DES-CBC decrypt w1");
    CHECK(out[2] == pt_des[2], "DES-CBC decrypt w2");
    CHECK(out[3] == pt_des[3], "DES-CBC decrypt w3");

    // ===== TDES-ECB =====
    uart_puts("--- TDES ECB ---\n");
    CRYP_CR = 0x4000;
    for (int i = 0; i < 6; i++) CRYP_KEY(i) = ktdes[i];
    CRYP_CR = 0x88010; // enable + bit19 + ALGOMODE=010 → TDES-ECB
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0xFBE62B68, "TDES-ECB w0");
    CHECK(out[1] == 0x3922941E, "TDES-ECB w1");
    CHECK(out[2] == 0x8CBD5711, "TDES-ECB w2");
    CHECK(out[3] == 0xBB3D1709, "TDES-ECB w3");

    CRYP_CR = 0x4000;
    CRYP_CR = 0x88014; // + ALGODIR=1
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt_des[0], "TDES-ECB decrypt w0");
    CHECK(out[1] == pt_des[1], "TDES-ECB decrypt w1");
    CHECK(out[2] == pt_des[2], "TDES-ECB decrypt w2");
    CHECK(out[3] == pt_des[3], "TDES-ECB decrypt w3");

    // ===== TDES-CBC =====
    uart_puts("--- TDES CBC ---\n");
    CRYP_CR = 0x4000;
    for (int i = 0; i < 6; i++) CRYP_KEY(i) = ktdes[i];
    for (int i = 0; i < 2; i++) CRYP_IV(i) = iv_des[i];
    CRYP_CR = 0x88018; // enable + bit19 + ALGOMODE=011 → TDES-CBC
    for (int i = 0; i < 4; i++) CRYP_DIN = pt_des[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x204011F9, "TDES-CBC w0");
    CHECK(out[1] == 0x86E35647, "TDES-CBC w1");
    CHECK(out[2] == 0x37BFABD2, "TDES-CBC w2");
    CHECK(out[3] == 0x0FAB61EB, "TDES-CBC w3");

    CRYP_CR = 0x4000;
    for (int i = 0; i < 6; i++) CRYP_KEY(i) = ktdes[i];
    for (int i = 0; i < 2; i++) CRYP_IV(i) = iv_des[i];
    CRYP_CR = 0x8801C; // + ALGODIR=1
    for (int i = 0; i < 4; i++) CRYP_DIN = out[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == pt_des[0], "TDES-CBC decrypt w0");
    CHECK(out[1] == pt_des[1], "TDES-CBC decrypt w1");
    CHECK(out[2] == pt_des[2], "TDES-CBC decrypt w2");
    CHECK(out[3] == pt_des[3], "TDES-CBC decrypt w3");

    // ===== DATATYPE AES-ECB =====
    uart_puts("--- DATATYPE ---\n");
    // 32-bit (no swap) — reference
    CRYP_CR = 0x4000;
    for (int i = 0; i < 4; i++) CRYP_KEY(i) = k128[i];
    CRYP_CR = 0x8000; // DATATYPE=00
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x3AD77BB4, "DT32 w0");
    CHECK(out[1] == 0x0D7A3660, "DT32 w1");
    CHECK(out[2] == 0xA89ECAF3, "DT32 w2");
    CHECK(out[3] == 0x2466EF97, "DT32 w3");

    // 16-bit DATATYPE (bit 6)
    CRYP_CR = 0x4000;
    CRYP_CR = 0x8040; // DATATYPE=01 (16-bit)
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0x45A1B939, "DT16 w0");
    CHECK(out[1] == 0x32E3B584, "DT16 w1");
    CHECK(out[2] == 0xFD5FDF04, "DT16 w2");
    CHECK(out[3] == 0x2B85E425, "DT16 w3");

    // 8-bit DATATYPE (bit 7)
    CRYP_CR = 0x4000;
    CRYP_CR = 0x8080; // DATATYPE=10 (8-bit)
    for (int i = 0; i < 4; i++) CRYP_DIN = pt[i];
    for (int i = 0; i < 4; i++) out[i] = CRYP_DOUT;
    CHECK(out[0] == 0xB8E1FFEE, "DT8 w0");
    CHECK(out[1] == 0x3109A871, "DT8 w1");
    CHECK(out[2] == 0x8FB98191, "DT8 w2");
    CHECK(out[3] == 0x83083D45, "DT8 w3");

    // ===== SUMMARY =====
    uart_puts("---- SUMMARY ----\n");
    uart_puts("PASS: ");
    for (int i = 7; i >= 0; i--) {
        int nib = (pass >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7))); USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
    uart_puts("\n");
    uart_puts("FAIL: ");
    for (int i = 7; i >= 0; i--) {
        int nib = (fail >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7))); USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
    uart_puts("\n");
    uart_puts("=== DONE ===\n");
    while (1);
}
