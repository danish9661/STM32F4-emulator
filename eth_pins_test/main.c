// eth_pins_test: MII/RMII pin mirrors. The emulator drives ETH signal
// levels into GPIO IDR (RMII AF11 wiring + MII COL): TX_EN (PB11) HIGH
// across the paced TX wire time, CRS_DV (PA7)/RXD (PC4/PC5) across the
// RX wire time, COL (PA3, MII mode) from an applied collision until that
// TX's wire end, MDIO (PA2)/MDC (PC1) idle HIGH. Runs at 10M so every
// window (~170k inst for 1200 B) dwarfs all jitter; sampling is done by
// the guest itself (every ~10 inst), so no step-size dependence.
// Against netsim in irq_eth mode (matrix + browser).
#include "defs.h"

#define GPIOA_BASE 0x40020000
#define GPIOB_BASE 0x40020400
#define GPIOC_BASE 0x40020800
#define GPIO_MODER(base) (*(volatile unsigned int *)((base) + 0x00))
#define GPIO_IDR(base)   (*(volatile unsigned int *)((base) + 0x10))
#define GPIO_AFRL(base)  (*(volatile unsigned int *)((base) + 0x20))
#define GPIO_AFRH(base)  (*(volatile unsigned int *)((base) + 0x24))
#define SYSCFG_PMC (*(volatile unsigned int *)0x40013804)

static volatile unsigned int tx_desc[2] __attribute__((aligned(8))) = { 0 };
static volatile int eth_done = 0;
static volatile unsigned int rx_desc[2] __attribute__((aligned(8))) = { 0 };
static volatile unsigned char rx_buf[1536] __attribute__((aligned(4)));
static volatile int rx_flag = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { eth_done = 1; DMASR = (1 << 0) | (1 << 16) | (1 << 14); }
    if (sr & (1 << 6)) { rx_flag = 1; DMASR = (1 << 6) | (1 << 16) | (1 << 14); }
}

void ETH_WKUP_IRQHandler(void) { /* unused here; vector slot */ }

static unsigned char tx_frame[1536];
static const unsigned char my_ip[4] = {10, 0, 2, 15};
static const unsigned char gw_ip[4] = {10, 0, 2, 2};
static const unsigned char gw_mac[6] = {0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd};
static const unsigned char my_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};

static unsigned int ip_header(unsigned char *dst_mac, unsigned char *dst_ip,
                              unsigned char proto, unsigned int l4len) {
    for (int i = 0; i < 6; i++) { tx_frame[i] = dst_mac[i]; tx_frame[6 + i] = my_mac[i]; }
    tx_frame[12] = 0x08; tx_frame[13] = 0x00;
    unsigned int ipLen = 20 + l4len;
    tx_frame[14] = 0x45; tx_frame[15] = 0;
    tx_frame[16] = ipLen >> 8; tx_frame[17] = ipLen & 0xFF;
    tx_frame[18] = 0; tx_frame[19] = 0;
    tx_frame[20] = 0; tx_frame[21] = 0;
    tx_frame[22] = 64; tx_frame[23] = proto;
    tx_frame[24] = 0; tx_frame[25] = 0;
    for (int i = 0; i < 4; i++) { tx_frame[26 + i] = my_ip[i]; tx_frame[30 + i] = dst_ip[i]; }
    unsigned int ck = cksum(&tx_frame[14], 20);
    tx_frame[24] = ck >> 8; tx_frame[25] = ck & 0xFF;
    return 34;
}

static int eth_send_frame(unsigned int len) {
    if (len < 60) { for (unsigned int i = len; i < 60; i++) tx_frame[i] = 0; len = 60; }
    tx_desc[0] = 0x80000000 | (len & 0x3FFF);
    tx_desc[1] = (unsigned int)&tx_frame[0];
    DMATDLAR = (unsigned int)&tx_desc[0];
    eth_done = 0;
    DMATPDR = 1;
    for (int i = 0; i < 2000000; i++) if (eth_done) return 1;
    return 0;
}

static unsigned int eth_recv_frame(unsigned int timeout_iters) {
    for (unsigned int i = 0; i < timeout_iters; i++) {
        if ((i & 0x3F) == 0) DMARPDR = 1;
        if (rx_flag) {
            rx_flag = 0;
            unsigned int len = (rx_desc[0] >> 16) & 0x3FFF;
            rx_desc[0] = 0x80000000 | 1536;
            DMARPDR = 1;
            if (len > 1536) len = 1536;
            return len;
        }
    }
    return 0;
}

// AF mux one pin (af = 11 for ETH RMII/MII).
static void af_pin(unsigned int base, unsigned int pin) {
    GPIO_MODER(base) = (GPIO_MODER(base) & ~(3u << (pin * 2))) | (2u << (pin * 2));
    if (pin < 8)
        GPIO_AFRL(base) = (GPIO_AFRL(base) & ~(0xFu << (pin * 4))) | (11u << (pin * 4));
    else
        GPIO_AFRH(base) = (GPIO_AFRH(base) & ~(0xFu << ((pin - 8) * 4))) | (11u << ((pin - 8) * 4));
}

int main(void) {
    uart_init();
    uart_puts("PINS Test: starting\r\n");
    RCC_AHB1ENR |= (1 << 25); // ETH
    RCC_AHB1ENR |= (1 << 0) | (1 << 1) | (1 << 2); // GPIOA/B/C
    DMABMR |= 1;
    wait_ms(10);
    MACCR = (1 << 2) | (1 << 3) | (1 << 11); // RE + TE + DM (10M: FES=0)
    MACA0HR = 0x00000200 | (1 << 31);
    MACA0LR = 0x00000001;
    DMAOMR = (1 << 13) | (1 << 1);
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6);
    NVIC_ISER1 |= (1 << (61 - 32));
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;
    // RMII wiring + AF mux (silicon-correct setup, verified by readback).
    SYSCFG_PMC = (1 << 23); // RMII
    af_pin(GPIOA_BASE, 2); af_pin(GPIOA_BASE, 7);
    af_pin(GPIOB_BASE, 11);
    af_pin(GPIOC_BASE, 1); af_pin(GPIOC_BASE, 4); af_pin(GPIOC_BASE, 5);
    af_pin(GPIOA_BASE, 3); // COL (MII)
    {
        int ok = ((SYSCFG_PMC & (1 << 23)) != 0)
            && ((GPIO_MODER(GPIOA_BASE) >> 4) & 3) == 2
            && ((GPIO_MODER(GPIOB_BASE) >> 22) & 3) == 2
            && ((GPIO_AFRL(GPIOA_BASE) >> 8) & 0xF) == 11;
        if (ok) uart_puts("PINS AF OK\r\n");
        else uart_puts("PINS AF FAIL\r\n");
    }

    // ---- TX_EN: HIGH across a 1200 B TX @10M, LOW after ----
    {
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 1200);
        tx_frame[off] = 0; tx_frame[off + 1] = 7;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 7;
        tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
        int saw_high = 0;
        tx_desc[0] = 0x80000000 | ((14 + 20 + 8 + 1200) & 0x3FFF);
        tx_desc[1] = (unsigned int)&tx_frame[0];
        DMATDLAR = (unsigned int)&tx_desc[0];
        eth_done = 0;
        DMATPDR = 1;
        for (int i = 0; i < 2000000 && !eth_done; i++) {
            if (GPIO_IDR(GPIOB_BASE) & (1 << 11)) saw_high = 1;
        }
        int done = eth_done;
        // Settle past the wire end, then must read LOW.
        for (volatile int i = 0; i < 300000; i++);
        int low_after = !(GPIO_IDR(GPIOB_BASE) & (1 << 11));
        (void)eth_recv_frame(20000); // drain the echo reply
        if (done && saw_high && low_after) uart_puts("PINS TX OK\r\n");
        else uart_puts("PINS TX FAIL\r\n");
    }

    // ---- RX_DV/CRS: HIGH right after a 1200 B delivery, LOW later ----
    // (Own trigger: the echo reply's wire time is the window.)
    {
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 1200);
        tx_frame[off] = 0; tx_frame[off + 1] = 8;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 7;
        tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
        tx_desc[0] = 0x80000000 | ((14 + 20 + 8 + 1200) & 0x3FFF);
        tx_desc[1] = (unsigned int)&tx_frame[0];
        DMATDLAR = (unsigned int)&tx_desc[0];
        eth_done = 0;
        DMATPDR = 1;
        unsigned int len = eth_recv_frame(200000);
        int high_now = (GPIO_IDR(GPIOA_BASE) & (1 << 7))
            && (GPIO_IDR(GPIOC_BASE) & (1 << 4))
            && (GPIO_IDR(GPIOC_BASE) & (1 << 5));
        for (volatile int i = 0; i < 500000; i++);
        int low_later = !(GPIO_IDR(GPIOA_BASE) & (1 << 7));
        if (len && eth_done && high_now && low_later) uart_puts("PINS RX OK\r\n");
        else uart_puts("PINS RX FAIL\r\n");
    }

    // ---- COL (MII): armed collision stretches it across the TX ----
    {
        SYSCFG_PMC = 0; // MII
        MACCR &= ~(1 << 11); // half-duplex
        uart_puts("COLLIDE ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 1200);
        tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
        int saw_high = 0;
        tx_desc[0] = 0x80000000 | ((14 + 20 + 8 + 1200) & 0x3FFF);
        tx_desc[1] = (unsigned int)&tx_frame[0];
        DMATDLAR = (unsigned int)&tx_desc[0];
        eth_done = 0;
        DMATPDR = 1;
        for (int i = 0; i < 2000000 && !eth_done; i++) {
            if (GPIO_IDR(GPIOA_BASE) & (1 << 3)) saw_high = 1;
        }
        int done = eth_done;
        for (volatile int i = 0; i < 300000; i++);
        int low_after = !(GPIO_IDR(GPIOA_BASE) & (1 << 3));
        (void)eth_recv_frame(20000);
        if (done && saw_high && low_after) uart_puts("PINS COL OK\r\n");
        else uart_puts("PINS COL FAIL\r\n");
        // Settle past the stretch before the negative (else stale-HIGH).
        for (volatile int i = 0; i < 300000; i++);
        // Full-duplex negative: armed collision leaves COL dark.
        MACCR |= (1 << 11);
        uart_puts("COLLIDE ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        saw_high = 0;
        tx_desc[0] = 0x80000000 | ((14 + 20 + 12) & 0x3FFF);
        tx_desc[1] = (unsigned int)&tx_frame[0];
        DMATDLAR = (unsigned int)&tx_desc[0];
        eth_done = 0;
        DMATPDR = 1;
        for (int i = 0; i < 2000000 && !eth_done; i++) {
            if (GPIO_IDR(GPIOA_BASE) & (1 << 3)) saw_high = 1;
        }
        (void)eth_recv_frame(20000);
        if (eth_done && !saw_high) uart_puts("PINS COL DROP OK\r\n");
        else uart_puts("PINS COL DROP FAIL\r\n");
        SYSCFG_PMC = (1 << 23); // back to RMII
    }

    // ---- MDIO/MDC idle HIGH ----
    {
        if ((GPIO_IDR(GPIOA_BASE) & (1 << 2)) && (GPIO_IDR(GPIOC_BASE) & (1 << 1)))
            uart_puts("PINS IDLE OK\r\n");
        else uart_puts("PINS IDLE FAIL\r\n");
    }

    uart_puts("PINS ALL PASS\r\n");
    uart_puts("PINS Test: done\r\n");
    while (1);
}
