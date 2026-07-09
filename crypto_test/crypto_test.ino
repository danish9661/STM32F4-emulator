#define USART1_BASE  0x40011000
#define USART_SR     (*(volatile uint32_t *)(USART1_BASE + 0x00))
#define USART_DR     (*(volatile uint32_t *)(USART1_BASE + 0x04))
#define USART_BRR    (*(volatile uint32_t *)(USART1_BASE + 0x08))
#define USART_CR1    (*(volatile uint32_t *)(USART1_BASE + 0x0C))

#define CRYP_BASE    0x50060000
#define CRYP_CR      (*(volatile uint32_t *)(CRYP_BASE + 0x00))
#define CRYP_SR      (*(volatile uint32_t *)(CRYP_BASE + 0x04))
#define CRYP_DIN     (*(volatile uint32_t *)(CRYP_BASE + 0x08))
#define CRYP_DOUT    (*(volatile uint32_t *)(CRYP_BASE + 0x0C))

#define HASH_BASE    0x50060400
#define HASH_CR      (*(volatile uint32_t *)(HASH_BASE + 0x00))
#define HASH_DIN     (*(volatile uint32_t *)(HASH_BASE + 0x04))
#define HASH_STR     (*(volatile uint32_t *)(HASH_BASE + 0x08))
#define HASH_HR(n)   (*(volatile uint32_t *)(HASH_BASE + 0x0C + (n)*4))

static int pass, fail;

static void tx_c(char c) { while (!(USART_SR & (1 << 7))); USART_DR = c; }
static void tx_s(const char *s) { while (*s) tx_c(*s++); }
static void tx_nl(void) { tx_c('\n'); }
static void tx_hex(uint32_t v) {
    for (int i = 7; i >= 0; i--) {
        int nib = (v >> (i * 4)) & 0xF;
        while (!(USART_SR & (1 << 7)));
        USART_DR = (nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

#define CHECK(cond, msg) do { \
    if (cond) { tx_s("PASS "); tx_s(msg); tx_nl(); pass++; } \
    else { tx_s("FAIL "); tx_s(msg); tx_nl(); fail++; } \
} while(0)

static void set_key(uint32_t *key, int nwords) {
    volatile uint32_t *kreg = (volatile uint32_t *)(CRYP_BASE + 0x20);
    for (int i = 0; i < nwords; i++) kreg[i] = key[i];
}

static void set_iv(uint32_t *iv, int nwords) {
    volatile uint32_t *ivreg = (volatile uint32_t *)(CRYP_BASE + 0x40);
    for (int i = 0; i < nwords; i++) ivreg[i] = iv[i];
}

static void cryp_write_block(uint32_t *data, int nwords) {
    for (int i = 0; i < nwords; i++) CRYP_DIN = data[i];
}

static void cryp_read_block(uint32_t *out, int nwords) {
    for (int i = 0; i < nwords; i++) out[i] = CRYP_DOUT;
}

void setup() {
    USART_BRR = 364;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
    tx_s("=== CRYPTO TEST ===\n");

    // ===== AES-128 ECB encrypt (NIST) =====
    tx_s("--- AES-128 ECB ---\n");
    uint32_t k128[4] = {0x2B7E1516, 0x28AED2A6, 0xABF71588, 0x09CF4F3C};
    uint32_t pt[4]   = {0x6BC1BEE2, 0x2E409F96, 0xE93D7E11, 0x7393172A};
    uint32_t ct128[4];

    CRYP_CR = 0x4000; // flush + disable
    set_key(k128, 4);
    CRYP_CR = 0x8000; // enable ECB encrypt AES-128
    cryp_write_block(pt, 4);
    cryp_read_block(ct128, 4);
    CHECK(ct128[0] == 0x3AD77BB4, "AES-128 ECB encrypt w0");
    CHECK(ct128[1] == 0x0D7A3660, "AES-128 ECB encrypt w1");
    CHECK(ct128[2] == 0xA89ECAF3, "AES-128 ECB encrypt w2");
    CHECK(ct128[3] == 0x2466EF97, "AES-128 ECB encrypt w3");

    // AES-128 ECB decrypt back
    CRYP_CR = 0x4000; // flush
    CRYP_CR = 0x8004; // enable + decrypt
    cryp_write_block(ct128, 4);
    uint32_t dec128[4];
    cryp_read_block(dec128, 4);
    CHECK(dec128[0] == pt[0], "AES-128 ECB decrypt w0");
    CHECK(dec128[1] == pt[1], "AES-128 ECB decrypt w1");
    CHECK(dec128[2] == pt[2], "AES-128 ECB decrypt w2");
    CHECK(dec128[3] == pt[3], "AES-128 ECB decrypt w3");

    // ===== AES-256 ECB encrypt (NIST) =====
    tx_s("--- AES-256 ECB ---\n");
    uint32_t k256[8] = {0x603DEB10, 0x15CA71BE, 0x2B73AEF0, 0x857D7781,
                        0x1F352C07, 0x3B6108D7, 0x2D9810A3, 0x0914DFF4};
    uint32_t ct256[4];

    CRYP_CR = 0x4000;
    set_key(k256, 8);
    CRYP_CR = 0x8A00; // enable, KEYSIZE=10 (AES-256)
    cryp_write_block(pt, 4);
    cryp_read_block(ct256, 4);
    CHECK(ct256[0] == 0xF3EED1BD, "AES-256 ECB encrypt w0");
    CHECK(ct256[1] == 0xB5D2A03C, "AES-256 ECB encrypt w1");
    CHECK(ct256[2] == 0x064B5A7E, "AES-256 ECB encrypt w2");
    CHECK(ct256[3] == 0x3DB181F8, "AES-256 ECB encrypt w3");

    // AES-256 ECB decrypt back
    CRYP_CR = 0x4000;
    CRYP_CR = 0x8A04; // enable + decrypt + AES-256
    cryp_write_block(ct256, 4);
    uint32_t dec256[4];
    cryp_read_block(dec256, 4);
    CHECK(dec256[0] == pt[0], "AES-256 ECB decrypt w0");
    CHECK(dec256[1] == pt[1], "AES-256 ECB decrypt w1");
    CHECK(dec256[2] == pt[2], "AES-256 ECB decrypt w2");
    CHECK(dec256[3] == pt[3], "AES-256 ECB decrypt w3");

    // ===== AES-128 CBC encrypt/decrypt =====
    tx_s("--- AES-128 CBC ---\n");
    uint32_t iv_cbc[4] = {0x00010203, 0x04050607, 0x08090A0B, 0x0C0D0E0F};
    uint32_t pt2[4]    = {0x6BC1BEE2, 0x2E409F96, 0xE93D7E11, 0x7393172A};
    uint32_t ct_cbc[4];

    CRYP_CR = 0x4000;
    set_key(k128, 4);
    set_iv(iv_cbc, 4);
    CRYP_CR = 0x8008; // enable, ALGOMODE=001 (CBC)
    cryp_write_block(pt2, 4);
    cryp_read_block(ct_cbc, 4);
    CHECK(ct_cbc[0] == 0x7649ABAC, "AES-128 CBC encrypt w0");
    CHECK(ct_cbc[1] == 0x8119B246, "AES-128 CBC encrypt w1");
    CHECK(ct_cbc[2] == 0xCEE98E9B, "AES-128 CBC encrypt w2");
    CHECK(ct_cbc[3] == 0x12E9197D, "AES-128 CBC encrypt w3");

    // CBC decrypt back
    CRYP_CR = 0x4000;
    set_iv(iv_cbc, 4);
    CRYP_CR = 0x800C; // enable + decrypt + CBC
    cryp_write_block(ct_cbc, 4);
    uint32_t dec_cbc[4];
    cryp_read_block(dec_cbc, 4);
    CHECK(dec_cbc[0] == pt2[0], "AES-128 CBC decrypt w0");
    CHECK(dec_cbc[1] == pt2[1], "AES-128 CBC decrypt w1");
    CHECK(dec_cbc[2] == pt2[2], "AES-128 CBC decrypt w2");
    CHECK(dec_cbc[3] == pt2[3], "AES-128 CBC decrypt w3");

    // ===== SHA-1 =====
    tx_s("--- SHA-1 ---\n");
    // SHA-1("abcd") — verified against sha1 crate
    HASH_CR = 0x4000; // flush
    HASH_DIN = 0x61626364;
    HASH_STR = 0x100;
    CHECK(HASH_HR(0) == 0x81FE8BFE, "SHA-1 HR0");
    CHECK(HASH_HR(1) == 0x87576C3E, "SHA-1 HR1");
    CHECK(HASH_HR(2) == 0xCB22426F, "SHA-1 HR2");
    CHECK(HASH_HR(3) == 0x8E578473, "SHA-1 HR3");
    CHECK(HASH_HR(4) == 0x82917ACF, "SHA-1 HR4");

    // SHA-1 of longer message: "STM32F407" = 9 bytes, use NBLW=8 for partial last word
    HASH_CR = 0x4001; // flush via INIT bit (self-clears)
    HASH_DIN = 0x53544D33; // "STM3"
    HASH_DIN = 0x32463430; // "2F40"
    HASH_DIN = 0x37000000; // "7\0\0\0" — only high byte is data
    HASH_STR = 0x108; // DCAL=1, NBLW=8 (9 bytes = 8 bits in last word)
    CHECK(HASH_HR(0) == 0xA743E1DE, "SHA-1 long HR0");
    CHECK(HASH_HR(1) == 0x41258D51, "SHA-1 long HR1");
    CHECK(HASH_HR(2) == 0x0F1B7BA5, "SHA-1 long HR2");
    CHECK(HASH_HR(3) == 0x52BECC79, "SHA-1 long HR3");
    CHECK(HASH_HR(4) == 0x9B8E709D, "SHA-1 long HR4");

    // ===== SHA-256 =====
    tx_s("--- SHA-256 ---\n");
    // SHA-256("abcd") — ALGO1=1 (bit 18), ALGO0=0
    HASH_CR = 0x4001; // flush
    HASH_CR = 0x44000; // SHA-256 (DMAE=1, ALGO1=1)
    HASH_DIN = 0x61626364;
    HASH_STR = 0x100;
    uint32_t sha256_hr0 = *(volatile uint32_t *)(HASH_BASE + 0x310);
    uint32_t sha256_hr1 = *(volatile uint32_t *)(HASH_BASE + 0x314);
    uint32_t sha256_hr2 = *(volatile uint32_t *)(HASH_BASE + 0x318);
    uint32_t sha256_hr3 = *(volatile uint32_t *)(HASH_BASE + 0x31C);
    uint32_t sha256_hr4 = *(volatile uint32_t *)(HASH_BASE + 0x320);
    uint32_t sha256_hr5 = *(volatile uint32_t *)(HASH_BASE + 0x324);
    uint32_t sha256_hr6 = *(volatile uint32_t *)(HASH_BASE + 0x328);
    uint32_t sha256_hr7 = *(volatile uint32_t *)(HASH_BASE + 0x32C);
    CHECK(sha256_hr0 == 0x88D4266F, "SHA-256 HR0");
    CHECK(sha256_hr1 == 0xD4E6338D, "SHA-256 HR1");
    CHECK(sha256_hr2 == 0x13B845FC, "SHA-256 HR2");
    CHECK(sha256_hr3 == 0xF289579D, "SHA-256 HR3");
    CHECK(sha256_hr4 == 0x209C8978, "SHA-256 HR4");
    CHECK(sha256_hr5 == 0x23B9217D, "SHA-256 HR5");
    CHECK(sha256_hr6 == 0xA3E16193, "SHA-256 HR6");
    CHECK(sha256_hr7 == 0x6F031589, "SHA-256 HR7");

    // ===== MD5 =====
    tx_s("--- MD5 ---\n");
    // MD5("abcd") — ALGO0=1, ALGO1=0 → CR = DMAE | ALGO0 = 0x4080
    HASH_CR = 0x4001; // flush via INIT
    HASH_CR = 0x4080; // MD5 (ALGO0=1, DMAE=1)
    HASH_DIN = 0x61626364;
    HASH_STR = 0x100;
    uint32_t md5_hr0 = *(volatile uint32_t *)(HASH_BASE + 0x0C);
    uint32_t md5_hr1 = *(volatile uint32_t *)(HASH_BASE + 0x10);
    uint32_t md5_hr2 = *(volatile uint32_t *)(HASH_BASE + 0x14);
    uint32_t md5_hr3 = *(volatile uint32_t *)(HASH_BASE + 0x18);
    // MD5("abcd") = e2fc714c4727ee9395f324cd2e7f331f
    CHECK(md5_hr0 == 0xE2FC714C, "MD5 HR0");
    CHECK(md5_hr1 == 0x4727EE93, "MD5 HR1");
    CHECK(md5_hr2 == 0x95F324CD, "MD5 HR2");
    CHECK(md5_hr3 == 0x2E7F331F, "MD5 HR3");

    tx_s("---- SUMMARY ----\n");
    tx_s("PASS: "); tx_hex(pass); tx_nl();
    tx_s("FAIL: "); tx_hex(fail); tx_nl();
    tx_s("=== DONE ===\n");
}

void loop() {}
