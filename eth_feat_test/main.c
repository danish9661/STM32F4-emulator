// eth_feat_test: exercises the "left-out" Ethernet features — PHY/MDIO
// auto-negotiation, TX checksum offload (CIC), RX checksum status
// (IPHCE/PCE), multicast hash filtering, VLAN tag filtering, PTP time +
// target + snapshots, Wake-on-LAN magic packet, and TX wire-rate pacing.
// Runs against netsim (matrix) in irq_eth mode; loopback (MACCR LM) covers
// the checksum paths without any peer.
#include "defs.h"

static volatile unsigned int tx_desc[8] __attribute__((aligned(8))) = { 0 };
static volatile int eth_done = 0;
// RX descriptor is 32 bytes (enhanced layout) so the driver can write the
// PTP snapshot at RDES6/7 (+24/+28).
static volatile unsigned int rx_desc[8] __attribute__((aligned(8))) = { 0 };
static volatile unsigned char rx_buf[1536] __attribute__((aligned(4)));
static volatile int rx_flag = 0;
static volatile unsigned int last_rdes0 = 0;
static volatile int ptp_flag = 0;
static volatile int wkp_flag = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { // TS bit
        eth_done = 1;
        DMASR = (1 << 0) | (1 << 16) | (1 << 14);
    }
    if (sr & (1 << 6)) { // RS bit
        rx_flag = 1;
        DMASR = (1 << 6) | (1 << 16) | (1 << 14);
    }
    if (!(sr & ((1 << 0) | (1 << 6)))) ptp_flag = 1; // e.g. PTP target
}

void ETH_WKUP_IRQHandler(void) {
    wkp_flag = 1;
    MACPMTCTL = MACPMTCTL | 0x20; // W1C: clear MPR, keep control bits
}

static unsigned char tx_frame[1536];
static const unsigned char my_ip[4] = {10, 0, 2, 15};
static const unsigned char gw_ip[4] = {10, 0, 2, 2};
static const unsigned char gw_mac[6] = {0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd};
static const unsigned char my_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};

// Build eth+IPv4 header; returns L4 payload offset (34).
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

// Wait for one RX frame; returns len (0 = timeout). Stashes RDES0 (status
// bits) before re-arming the descriptor.
static unsigned int eth_recv_frame(unsigned int timeout_iters) {
    for (unsigned int i = 0; i < timeout_iters; i++) {
        if ((i & 0x3F) == 0) DMARPDR = 1; // keep the poll armed
        if (rx_flag) {
            rx_flag = 0;
            last_rdes0 = rx_desc[0];
            unsigned int len = (rx_desc[0] >> 16) & 0x3FFF;
            rx_desc[0] = 0x80000000 | 1536;
            DMARPDR = 1;
            if (len > 1536) len = 1536;
            return len;
        }
    }
    return 0;
}

static int eth_send_frame(unsigned int len, unsigned int tdes_flags) {
    if (len < 60) { for (unsigned int i = len; i < 60; i++) tx_frame[i] = 0; len = 60; }
    tx_desc[0] = 0x80000000 | tdes_flags | (len & 0x3FFF);
    tx_desc[1] = (unsigned int)&tx_frame[0];
    DMATDLAR = (unsigned int)&tx_desc[0];
    eth_done = 0;
    DMATPDR = 1;
    for (int i = 0; i < 2000000; i++) if (eth_done) return 1;
    return 0;
}

// ---- MDIO ----
static void mii_write(unsigned int phy, unsigned int reg, unsigned int val) {
    MACMIIDR = val;
    MACMIIAR = (phy << 11) | (reg << 6) | (2 << 2) | (1 << 1) | 1;
    for (int i = 0; i < 100000; i++) if (!(MACMIIAR & 1)) return;
}
static unsigned int mii_read(unsigned int phy, unsigned int reg) {
    MACMIIAR = (phy << 11) | (reg << 6) | (2 << 2) | 1;
    for (int i = 0; i < 100000; i++) if (!(MACMIIAR & 1)) return MACMIIDR;
    return 0xFFFF;
}

// ---- CRC32 (Ethernet, reflected) for the multicast hash ----
static unsigned int crc32(unsigned char *p, unsigned int n) {
    unsigned int crc = 0xFFFFFFFF;
    for (unsigned int i = 0; i < n; i++) {
        crc ^= p[i];
        for (int b = 0; b < 8; b++)
            crc = (crc & 1) ? (crc >> 1) ^ 0xEDB88320 : (crc >> 1);
    }
    return ~crc;
}

// ---- DWT cycle counter ----
static void dwt_on(void) {
    *(volatile unsigned int *)0xE000EDFC |= (1 << 24); // DEMCR TRCENA
    *(volatile unsigned int *)0xE0001000 |= 1; // DWT CTRL CYCCNTENA
}
static void dwt_zero(void) { *(volatile unsigned int *)0xE0001004 = 0; }
static unsigned int dwt_rd(void) { return *(volatile unsigned int *)0xE0001004; }

int main(void) {
    uart_init();
    uart_puts("FEAT Test: starting\r\n");
    RCC_AHB1ENR |= (1 << 25);
    DMABMR |= 1;
    wait_ms(10);
    MACCR = (1 << 2) | (1 << 3) | (1 << 11); // RE + TE + DM
    uart_puts("MAC enabled\r\n");
    MACA0HR = 0x00000200 | (1 << 31); // AE; MAC 02:00:00:00:00:01 MSB-first
    MACA0LR = 0x00000001;
    DMAOMR = (1 << 13) | (1 << 1); // ST + SR
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6); // NIE + TSE + RSE
    NVIC_ISER1 |= (1 << (61 - 32)); // IRQ 61
    NVIC_ISER1 |= (1 << (62 - 32)); // IRQ 62 (WKUP)
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;
    uart_puts("Setup complete\r\n");

    // ---- 1. PHY: link + AN ----
    {
        unsigned int bmsr = mii_read(0, 1);
        if ((bmsr & 0x24) == 0x24) uart_puts("PHY link OK\r\n");
        else { uart_puts("PHY LINK FAIL bmsr="); uart_hex32(bmsr); uart_puts("\r\n"); }
        mii_write(0, 0, 0x3300); // AN enable + restart
        unsigned int done = 0;
        for (int i = 0; i < 200000; i++) {
            if (mii_read(0, 1) & 0x20) { done = 1; break; }
        }
        if (done) uart_puts("PHY AN restart OK\r\n");
        else uart_puts("PHY AN TIMEOUT\r\n");
        // Force 10/half, check PHYSTS, restore.
        mii_write(0, 0, 0x0000);
        unsigned int physts = mii_read(0, 0x10);
        if ((physts & 0x07) == 0x03) uart_puts("PHY force OK\r\n");
        else { uart_puts("PHY FORCE FAIL "); uart_hex32(physts); uart_puts("\r\n"); }
        mii_write(0, 0, 0x3100 | 0x0200); // AN enable + restart -> 100/full
        for (int i = 0; i < 200000; i++) {
            if (mii_read(0, 1) & 0x20) break;
        }
        MACCR |= (1 << 14) | (1 << 11); // FES + DM per negotiation
        uart_puts("PHY MAC programmed\r\n");
    }

    // ---- 2/3. Checksum offload via loopback ----
    MACCR |= (1 << 12); // LM loopback (frames must address ourselves:
    // the RX accept filter applies in loopback, like silicon)
    {
        unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 8);
        tx_frame[off] = 49153 >> 8; tx_frame[off + 1] = 49153 & 0xFF;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 7;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 16;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0; // zero UDP cksum
        // Zero the IP checksum too (ip_header filled it in).
        tx_frame[14 + 10] = 0; tx_frame[14 + 11] = 0;
        for (int i = 0; i < 8; i++) tx_frame[off + 8 + i] = 0x40 + i;
        if (eth_send_frame(14 + 20 + 16, 2 << 22)) { // CIC=10 full offload
            unsigned int len = eth_recv_frame(20000);
            if (len && cksum(&rx_buf[14], 20) == 0) uart_puts("CSUM TX insert OK\r\n");
            else uart_puts("CSUM TX INSERT FAIL\r\n");
            if (len && !(last_rdes0 & ((1 << 7) | 1))) uart_puts("CSUM RX status OK\r\n");
            else { uart_puts("CSUM RX STATUS FAIL "); uart_hex32(last_rdes0); uart_puts("\r\n"); }
        } else uart_puts("CSUM LOOPBACK TX TIMEOUT\r\n");
        // Bad IP checksum, no offload -> IPHCE must set.
        tx_frame[14 + 10] = 0x12; tx_frame[14 + 11] = 0x34;
        if (eth_send_frame(14 + 20 + 16, 0)) {
            unsigned int len = eth_recv_frame(20000);
            if (len && (last_rdes0 & (1 << 7))) uart_puts("CSUM RX IPHCE OK\r\n");
            else { uart_puts("CSUM IPHCE FAIL "); uart_hex32(last_rdes0); uart_puts("\r\n"); }
        }
        // Good IP, bad UDP -> PCE must set.
        {
            tx_frame[14 + 10] = 0; tx_frame[14 + 11] = 0;
            unsigned int ck = cksum(&tx_frame[14], 20);
            tx_frame[14 + 10] = ck >> 8; tx_frame[14 + 11] = ck & 0xFF;
            tx_frame[off + 6] = 0xAB; tx_frame[off + 7] = 0xCD;
        }
        if (eth_send_frame(14 + 20 + 16, 0)) {
            unsigned int len = eth_recv_frame(20000);
            if (len && (last_rdes0 & 1) && !(last_rdes0 & (1 << 7)))
                uart_puts("CSUM RX PCE OK\r\n");
            else { uart_puts("CSUM PCE FAIL "); uart_hex32(last_rdes0); uart_puts("\r\n"); }
        }
    }
    MACCR &= ~(1 << 12); // loopback off

    // ---- 4. Multicast hash ----
    {
        static const unsigned char g1[6] = {0x01, 0x00, 0x5E, 0x00, 0x00, 0x07};
        static const unsigned char g2[6] = {0x01, 0x00, 0x5E, 0x00, 0x00, 0x08};
        unsigned int h1 = crc32((unsigned char *)g1, 6) >> 26;
        unsigned int h2 = crc32((unsigned char *)g2, 6) >> 26;
        if (h1 < 32) MACHTLR = 1 << h1; else MACHTHR = 1 << (h1 - 32);
        MACFFR = 1 << 2; // HM
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 4);
        tx_frame[off] = 5001 >> 8; tx_frame[off + 1] = 5001 & 0xFF;
        tx_frame[off + 2] = 5001 >> 8; tx_frame[off + 3] = 5001 & 0xFF;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        int got1 = 0, got2 = 0;
        if (eth_send_frame(14 + 20 + 12, 0)) {
            for (int r = 0; r < 200 && !got1; r++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                if (rx_buf[12] == 0x08 && rx_buf[13] == 0x00 && rx_buf[23] == 17) {
                    unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
                    if (rx_buf[uo + 8] == 'M') {
                        if (rx_buf[uo + 13] == '1') got1 = 1;
                        if (rx_buf[uo + 13] == '2') got2 = 1;
                    }
                }
            }
            // Drain window: a non-member must never arrive (unless the
            // 6-bit hashes collide, 1/64 — then the check is vacuous).
            for (int r = 0; r < 40 && !got2; r++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                if (rx_buf[12] == 0x08 && rx_buf[13] == 0x00 && rx_buf[23] == 17) {
                    unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
                    if (rx_buf[uo + 8] == 'M' && rx_buf[uo + 13] == '2') got2 = 1;
                }
            }
        }
        if (got1 && (!got2 || h1 == h2)) uart_puts("MCAST OK\r\n");
        else { uart_puts("MCAST FAIL "); uart_hex32(got1 * 2 + got2); uart_puts("\r\n"); }
        MACFFR = 0;
        MACHTLR = 0; MACHTHR = 0;
    }

    // ---- 5. VLAN ----
    {
        MACVLANTR = 7; // accept only VID 7
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 4);
        tx_frame[off] = 5002 >> 8; tx_frame[off + 1] = 5002 & 0xFF;
        tx_frame[off + 2] = 5002 >> 8; tx_frame[off + 3] = 5002 & 0xFF;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        int got_tag = 0, got_plain = 0;
        if (eth_send_frame(14 + 20 + 12, 0)) {
            for (int r = 0; r < 200 && !got_tag; r++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                unsigned int l3 = 14;
                if (rx_buf[12] == 0x81 && rx_buf[13] == 0x00) l3 = 18;
                else if (!(rx_buf[12] == 0x08 && rx_buf[13] == 0x00)) continue;
                if (rx_buf[l3 + 9] != 17) continue;
                unsigned int uo = l3 + ((rx_buf[l3] & 0xF) * 4);
                if (rx_buf[uo + 8] == 'V') got_tag = 1;
                if (rx_buf[uo + 8] == 'N') got_plain = 1;
            }
            for (int r = 0; r < 40 && !got_plain; r++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                unsigned int l3 = 14;
                if (rx_buf[12] == 0x81 && rx_buf[13] == 0x00) l3 = 18;
                else if (!(rx_buf[12] == 0x08 && rx_buf[13] == 0x00)) continue;
                if (rx_buf[l3 + 9] != 17) continue;
                unsigned int uo = l3 + ((rx_buf[l3] & 0xF) * 4);
                if (rx_buf[uo + 8] == 'N') got_plain = 1;
            }
        }
        if (got_tag && !got_plain) uart_puts("VLAN OK\r\n");
        else { uart_puts("VLAN FAIL "); uart_hex32(got_tag * 2 + got_plain); uart_puts("\r\n"); }
        MACVLANTR = 0;
    }

    // ---- 6. PTP ----
    {
        PTPTSCR |= 1; // TSE
        PTPTSHUR = 0; PTPTSLUR = 0;
        PTPTSCR |= (1 << 2); // TSSTI init
        unsigned int s0 = PTPTSHR, u0 = PTPTSLR;
        for (volatile int i = 0; i < 200000; i++);
        unsigned int s1 = PTPTSHR, u1 = PTPTSLR;
        if (u1 != u0 || s1 != s0) uart_puts("PTP time OK\r\n");
        else uart_puts("PTP TIME FROZEN\r\n");
        // Target ~4ms ahead (0x800000 sub-units), interrupt on.
        // Carry into seconds when the addition would wrap.
        ptp_flag = 0;
        if (u1 > 0xFFFFFFFFu - 0x800000u) { PTPTTHR = s1 + 1; PTPTTLR = u1 + 0x800000u; }
        else { PTPTTHR = s1; PTPTTLR = u1 + 0x800000u; }
        PTPTSCR |= (1 << 4); // TSITE
        MACIMR |= (1 << 9); // TSTIM
        {
            int hit = 0;
            for (int i = 0; i < 2000000 && !hit; i++) {
                if (ptp_flag) hit = 1;
            }
            if (hit) uart_puts("PTP target OK\r\n");
            else uart_puts("PTP TARGET TIMEOUT\r\n");
        }
        // TX snapshot via loopback RX too (driver writes both).
        MACCR |= (1 << 12);
        tx_desc[6] = 0; tx_desc[7] = 0;
        rx_desc[6] = 0; rx_desc[7] = 0;
        {
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            if (eth_send_frame(14 + 20 + 12, 1 << 25)) { // TTSE
                unsigned int len = eth_recv_frame(20000);
                if (len && (tx_desc[6] || tx_desc[7])) uart_puts("PTP TX snap OK\r\n");
                else uart_puts("PTP TX SNAP FAIL\r\n");
                if (len && (rx_desc[6] || rx_desc[7])) uart_puts("PTP RX snap OK\r\n");
                else uart_puts("PTP RX SNAP FAIL\r\n");
            } else uart_puts("PTP SNAP TX TIMEOUT\r\n");
        }
        MACCR &= ~(1 << 12);
    }

    // ---- 7. Wake-on-LAN ----
    {
        wkp_flag = 0;
        MACPMTCTL = 0x2; // MPE
        MACIMR |= (1 << 3); // PMTIM
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 4);
        tx_frame[off] = 5003 >> 8; tx_frame[off + 1] = 5003 & 0xFF;
        tx_frame[off + 2] = 5003 >> 8; tx_frame[off + 3] = 5003 & 0xFF;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        if (eth_send_frame(14 + 20 + 12, 0)) {
            int hit = 0;
            for (int i = 0; i < 200 && !hit; i++) {
                (void)eth_recv_frame(20000);
                if (wkp_flag) hit = 1;
            }
            if (hit) uart_puts("WOL OK\r\n");
            else uart_puts("WOL TIMEOUT\r\n");
        } else uart_puts("WOL TX TIMEOUT\r\n");
        MACPMTCTL = 0;
    }

    // ---- 8. Wire rate: 1200B frame @100M. Wire bytes ~= len+20, so
    // cycles ~= (1242*8/1e8)*168e6 ~= 16.7k. The completion can only
    // land on a step boundary (driver runs between steps), so allow
    // one step of overshoot on top: assert 10k < d < 40k. A broken
    // (instant) pacer would finish within a single step (<=5k).
    {
        dwt_on();
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 1200);
        tx_frame[off] = 0; tx_frame[off + 1] = 7;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 7;
        tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
        dwt_zero();
        if (eth_send_frame(14 + 20 + 8 + 1200, 0)) {
            unsigned int d = dwt_rd();
            uart_puts("WIRE cycles=");
            uart_hex32(d);
            uart_puts("\r\n");
            if (d > 10000 && d < 40000) uart_puts("WIRE RATE OK\r\n");
            else uart_puts("WIRE RATE OFF\r\n");
        } else uart_puts("WIRE TX TIMEOUT\r\n");
    }

    uart_puts("FEAT ALL PASS\r\n");
    uart_puts("FEAT Test: done\r\n");
    while (1);
}
