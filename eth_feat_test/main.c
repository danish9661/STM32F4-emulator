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
    // Ack NOTHING here: MPR/RWKPR are W1C, and acking on entry destroys
    // the evidence before thread mode can assert it (the handler always
    // wins that race). Thread mode acks explicitly after observing.
}

static unsigned char tx_frame[1536];
static const unsigned char my_ip[4] = {10, 0, 2, 15};
static const unsigned char gw_ip[4] = {10, 0, 2, 2};
static const unsigned char gw_mac[6] = {0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd};
static const unsigned char my_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};
static const unsigned char bcast_mac[6] = {0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF};

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

// CRC-16 (poly 0x1021, init 0xFFFF): matches the wakeup-filter engine.
static unsigned int crc16(unsigned char *p, unsigned int n) {
    unsigned int crc = 0xFFFF;
    for (unsigned int i = 0; i < n; i++) {
        crc ^= (unsigned int)p[i] << 8;
        for (int b = 0; b < 8; b++)
            crc = (crc & 0x8000) ? ((crc << 1) ^ 0x1021) & 0xFFFF : (crc << 1) & 0xFFFF;
    }
    return crc & 0xFFFF;
}

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
        // Media interface select (SYSCFG PMC RMII/MII): the data path is
        // mode-agnostic (no pins to mux), but the select element must
        // stick and traffic must flow in both modes.
        SYSCFG_PMC = (1 << 23); // RMII
        if ((SYSCFG_PMC & (1 << 23)) != 0) uart_puts("PHY media RMII OK\r\n");
        else uart_puts("PHY MEDIA FAIL\r\n");
        SYSCFG_PMC = 0; // MII
        if ((SYSCFG_PMC & (1 << 23)) == 0) uart_puts("PHY media MII OK\r\n");
        else uart_puts("PHY MEDIA FAIL\r\n");
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
        // Real driver init: program addend + SSINC, apply with TSFCU.
        // 0x80000000 wraps every 2 ticks; SSINC 26 => 13 sub-units/tick.
        PTPTSAR = 0x80000000;
        PTPSSIR = 26;
        PTPTSCR |= (1 << 1); // TSFCU: latch rate
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
        // Drift correction: halve the addend, re-latch, and compare the
        // subsecond advance over identical CYCCNT windows (expect ~1:2).
        dwt_on();
        {
            unsigned int s0 = PTPTSLR;
            dwt_zero(); while (dwt_rd() < 200000);
            unsigned int s1 = PTPTSLR;
            unsigned int full = s1 - s0;
            PTPTSAR = 0x40000000;
            PTPTSCR |= (1 << 1); // TSFCU
            unsigned int s2 = PTPTSLR;
            dwt_zero(); while (dwt_rd() < 200000);
            unsigned int s3 = PTPTSLR;
            unsigned int half = s3 - s2;
            PTPTSAR = 0x80000000;
            PTPTSCR |= (1 << 1); // restore full rate
            unsigned int pct = full ? (half * 100) / full : 0;
            if (pct >= 40 && pct <= 60) uart_puts("PTP drift OK\r\n");
            else { uart_puts("PTP DRIFT FAIL "); uart_hex32(pct); uart_puts("\r\n"); }
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
        // Wakeup-frame filter: program filter 0 ("WAKE" at [42..46]) in
        // the sourced DWC_gmac layout — mask, command (unicast-eligible),
        // offset, CRC — plus GLOBU (unicast-to-us eligibility), then match
        // + mismatch through loopback (accept-filtered to ourselves).
        {
            unsigned char pat[4] = { 'W', 'A', 'K', 'E' };
            unsigned int crc = crc16(pat, 4);
            MACPMTCTL = (1 << 31) | 0x206; // WFFRPR + MPE + WFE + GLOBU
            MACRWUFFR = 0x0F; // word 0: filter 0 mask
            MACRWUFFR = 0x00; MACRWUFFR = 0x00; MACRWUFFR = 0x00; // words 1-3
            MACRWUFFR = 0x00; // word 4: commands
            MACRWUFFR = 42; // word 5: filter 0 offset
            MACRWUFFR = crc; // word 6: filter 0 CRC
            MACRWUFFR = 0x00; // word 7
            MACCR |= (1 << 12); // LM loopback
            wkp_flag = 0;
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
            tx_frame[off] = 0; tx_frame[off + 1] = 8;
            tx_frame[off + 2] = 0; tx_frame[off + 3] = 9;
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            tx_frame[off + 8] = 'W'; tx_frame[off + 9] = 'A';
            tx_frame[off + 10] = 'K'; tx_frame[off + 11] = 'E';
            int fok = 0;
            if (eth_send_frame(14 + 20 + 12, 0)) {
                for (int i = 0; i < 200 && !fok; i++) {
                    (void)eth_recv_frame(20000);
                    if ((MACPMTCTL & 0x40) && wkp_flag) fok = 1;
                }
            }
            // Read-to-clear already handled by the observes above; nothing to ack.
            // Mismatch must NOT set RWKPR.
            int bad = 0;
            tx_frame[off + 9] = 'X';
            wkp_flag = 0;
            if (eth_send_frame(14 + 20 + 12, 0)) {
                for (int i = 0; i < 60 && !bad; i++) {
                    (void)eth_recv_frame(20000);
                    if (MACPMTCTL & 0x40) bad = 1;
                }
            }
            // Multicast-only command: the unicast match goes silent...
            int mcast_ok = 0, uni_silent = 1;
            MACPMTCTL = (1 << 31) | 0x206; // re-arm pointer, keep enables
            MACRWUFFR = 0x0F; MACRWUFFR = 0x00; MACRWUFFR = 0x00; MACRWUFFR = 0x00;
            MACRWUFFR = 0x08; // word 4: filter 0 multicast-only
            MACRWUFFR = 42; MACRWUFFR = crc; MACRWUFFR = 0x00;
            wkp_flag = 0;
            tx_frame[off + 9] = 'A'; // back to "WAKE"
            if (eth_send_frame(14 + 20 + 12, 0)) {
                for (int i = 0; i < 30; i++) {
                    (void)eth_recv_frame(20000);
                    if (MACPMTCTL & 0x40) uni_silent = 0;
                }
            }
            // ...but the same payload to broadcast matches.
            {
                unsigned int bo = ip_header((unsigned char *)bcast_mac, my_ip, 17, 8 + 4);
                tx_frame[bo + 4] = 0; tx_frame[bo + 5] = 12;
                tx_frame[bo + 6] = 0; tx_frame[bo + 7] = 0;
                tx_frame[bo + 8] = 'W'; tx_frame[bo + 9] = 'A';
                tx_frame[bo + 10] = 'K'; tx_frame[bo + 11] = 'E';
                wkp_flag = 0;
                if (eth_send_frame(14 + 20 + 12, 0)) {
                    for (int i = 0; i < 200 && !mcast_ok; i++) {
                        (void)eth_recv_frame(20000);
                        if ((MACPMTCTL & 0x40) && wkp_flag) mcast_ok = 1;
                    }
                }
            }
            // Powerdown: receiver drops everything, WOL still sees magic.
            int pd_ok = 0, pd_drop = 1;
            MACPMTCTL = 0x1 | 0x2; // PWRDWN + MPE
            wkp_flag = 0;
            {
                // Normal frame under PD: must never arrive.
                unsigned int po = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
                tx_frame[po + 4] = 0; tx_frame[po + 5] = 12;
                tx_frame[po + 6] = 0; tx_frame[po + 7] = 0;
                if (eth_send_frame(14 + 20 + 12, 0)) {
                    for (int i = 0; i < 30; i++) {
                        unsigned int l = eth_recv_frame(20000);
                        if (l) pd_drop = 0;
                    }
                }
                // Magic pattern under PD: still detected (loopback frame).
                unsigned char *mp = tx_frame;
                for (int i = 0; i < 6; i++) { mp[i] = my_mac[i]; mp[6 + i] = my_mac[i]; }
                mp[12] = 0x08; mp[13] = 0x00;
                for (int i = 0; i < 6; i++) mp[14 + i] = 0xFF;
                for (int r = 0; r < 16; r++)
                    for (int i = 0; i < 6; i++) mp[20 + r * 6 + i] = my_mac[i];
                if (eth_send_frame(14 + 102, 0)) {
                    for (int i = 0; i < 200 && !pd_ok; i++) {
                        (void)eth_recv_frame(20000);
                        if ((MACPMTCTL & 0x20) && wkp_flag) pd_ok = 1;
                    }
                }
            }
            MACPMTCTL = 0;
            MACCR &= ~(1 << 12);
            {
                unsigned int pass = (fok ? 32 : 0) | (!bad ? 16 : 0) | (uni_silent ? 8 : 0) |
                                    (mcast_ok ? 4 : 0) | (pd_drop ? 2 : 0) | (pd_ok ? 1 : 0);
                if (pass == 0x3F) uart_puts("WOL filter OK\r\n");
                else { uart_puts("WOL FILTER FAIL "); uart_hex32(pass); uart_puts("\r\n"); }
            }
        }
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

    // ---- 8b. Collision report (half-duplex error path) ----
    // The matrix hook arms one collision when it sees COLLIDE ARM; the
    // send must follow after a spin so the arm lands first (the driver
    // decides EC at poll-processing time, not at TS).
    {
        MACCR &= ~(1 << 11); // DM=0 half-duplex
        MACCR |= (1 << 12); // LM loopback (self-contained)
        uart_puts("COLLIDE ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        {
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            if (eth_send_frame(14 + 20 + 12, 0)) {
                (void)eth_recv_frame(20000);
                unsigned int t0 = tx_desc[0];
                if ((t0 & 0x100) && ((t0 >> 3) & 0xF) == 0xF)
                    uart_puts("COLLIDE OK\r\n");
                else { uart_puts("COLLIDE FAIL "); uart_hex32(t0); uart_puts("\r\n"); }
            } else uart_puts("COLLIDE TX TIMEOUT\r\n");
        }
        // Full-duplex negative: an armed collision must be dropped.
        MACCR &= ~(1 << 12);
        MACCR |= (1 << 11); // DM=1 full
        uart_puts("COLLIDE ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        {
            unsigned int off = ip_header((unsigned char *)gw_mac, gw_ip, 17, 8 + 4);
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            if (eth_send_frame(14 + 20 + 12, 0)) {
                (void)eth_recv_frame(20000);
                if (!(tx_desc[0] & 0x100)) uart_puts("COLLIDE DROP OK\r\n");
                else uart_puts("COLLIDE DROP FAIL\r\n");
            }
        }
    }

    // ---- 8c. RX wire pacing: loopback 1200B, DWT from send to recv ----
    // Covers TX pacing (~17k) + RX pacing (~17k) + step overshoot.
    {
        MACCR |= (1 << 12); // LM loopback
        unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 1200);
        tx_frame[off] = 0; tx_frame[off + 1] = 8;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 9;
        tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
        dwt_zero();
        if (eth_send_frame(14 + 20 + 8 + 1200, 0)) {
            unsigned int len = eth_recv_frame(200000);
            unsigned int d = dwt_rd();
            uart_puts("RX cycles=");
            uart_hex32(d);
            uart_puts("\r\n");
            if (len && d > 12000 && d < 80000) uart_puts("RX RATE OK\r\n");
            else uart_puts("RX RATE OFF\r\n");
        } else uart_puts("RX RATE TX TIMEOUT\r\n");
        // CSMA/CD deferral: half-duplex TX while the 1200 B receive is
        // still on the wire must report DB (TS still completes); the
        // same race in full-duplex must not. Run at 10M so the ~170k
        // wire window dwarfs the stepped-execution path jitter (~15k);
        // the rate itself is proven by the WIRE/RX bands at 100M.
        // PIPELINED (never wait TX#1's paced TS first — that wait alone
        // consumes the whole RX window, on silicon too): queue TX#1,
        // take delivery#1, fire TX#2 immediately, then join everything.
        MACCR &= ~(1 << 11); // DM=0 half-duplex
        MACCR &= ~(1 << 14); // 10M
        {
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 1200);
            tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
            tx_desc[0] = 0x80000000 | ((14 + 20 + 8 + 1200) & 0x3FFF);
            tx_desc[1] = (unsigned int)&tx_frame[0];
            DMATDLAR = (unsigned int)&tx_desc[0];
            eth_done = 0;
            DMATPDR = 1;
            unsigned int l1 = eth_recv_frame(200000);
            int db = 0, ok2 = 0, consumed = 0;
            if (l1) {
                unsigned int off2 = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
                tx_frame[off2 + 4] = 0; tx_frame[off2 + 5] = 12;
                tx_frame[off2 + 6] = 0; tx_frame[off2 + 7] = 0;
                tx_desc[0] = 0x80000000 | ((14 + 20 + 12) & 0x3FFF);
                tx_desc[1] = (unsigned int)&tx_frame[0];
                DMATDLAR = (unsigned int)&tx_desc[0];
                DMATPDR = 1;
                // Join: both paced completions + TX#2's loopback back.
                // (Bounded: 40 rounds worst-case, fast path exits early.)
                for (int i = 0; i < 40 && (!ok2 || !consumed); i++) {
                    unsigned int l2 = eth_recv_frame(20000);
                    if (l2) ok2 = 1;
                    if (!(tx_desc[0] & 0x80000000)) consumed = 1;
                }
                db = (tx_desc[0] & 0x1) != 0;
            }
            if (db && ok2 && consumed) uart_puts("DEFER OK\r\n");
            else uart_puts("DEFER FAIL\r\n");
        }
        MACCR |= (1 << 11); // DM=1 full-duplex
        {
            // Same race full-duplex: DB must stay clear.
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 1200);
            tx_frame[off + 4] = (1200 + 8) >> 8; tx_frame[off + 5] = (1200 + 8) & 0xFF;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            for (int i = 0; i < 1200; i++) tx_frame[off + 8 + i] = i & 0xFF;
            int db = 0;
            if (eth_send_frame(14 + 20 + 8 + 1200, 0)) {
                if (eth_recv_frame(200000)) {
                    unsigned int off2 = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
                    tx_frame[off2 + 4] = 0; tx_frame[off2 + 5] = 12;
                    tx_frame[off2 + 6] = 0; tx_frame[off2 + 7] = 0;
                    if (eth_send_frame(14 + 20 + 12, 0)) {
                        (void)eth_recv_frame(200000);
                        db = (tx_desc[0] & 0x1) != 0;
                    }
                }
            }
            if (!db) uart_puts("DEFER DROP OK\r\n");
            else uart_puts("DEFER DROP FAIL\r\n");
        }
        MACCR |= (1 << 14); // restore 100M
        MACCR &= ~(1 << 12);
    }

    // ---- 8d. Link/carrier (matrix hook drops/restores the wire) ----
    // Down: MDIO reports link-clear and TX never completes (no carrier:
    // NC status). Up: MDIO link + TX works. Spins give the hook time
    // (link state is a wire property the guest can only observe).
    {
        uart_puts("LINK DOWN ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        {
            unsigned int bmsr = mii_read(0, 1);
            int blink = !(bmsr & 0x04);
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            // Dead wire completes with NC status (TS still raises: error
            // completion, like silicon) and nothing goes out.
            int ok = eth_send_frame(14 + 20 + 12, 0);
            unsigned int nc = tx_desc[0] & 0x400;
            if (blink && ok && nc) uart_puts("LINK DOWN OK\r\n");
            else { uart_puts("LINK DOWN FAIL "); uart_hex32((blink << 16) | nc); uart_puts("\r\n"); }
        }
        uart_puts("LINK UP ARM\r\n");
        for (volatile int i = 0; i < 20000; i++);
        {
            unsigned int bmsr = mii_read(0, 1);
            int blink = (bmsr & 0x04) != 0;
            unsigned int off = ip_header((unsigned char *)my_mac, my_ip, 17, 8 + 4);
            tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
            tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
            int ok = eth_send_frame(14 + 20 + 12, 0);
            if (ok) (void)eth_recv_frame(20000);
            if (blink && ok) uart_puts("LINK UP OK\r\n");
            else uart_puts("LINK UP FAIL\r\n");
        }
    }

    // ---- 9. PPS: 32768 Hz for a CYCCNT-measured 200k-inst window ----
    // (~39 edges). The count is model-side (the pin's observable sink);
    // the harness asserts the band (see the matrix post hook).
    {
        uart_puts("PPS ppscr before=");
        uart_hex32(PTPPPSCR);
        uart_puts("\r\n");
        PTPPPSCR = 15;
        dwt_zero(); while (dwt_rd() < 200000);
        PTPPPSCR = 0; // back to 1 Hz: ~0 edges over the rest of the run
        uart_puts("PPS window done\r\n");
    }

    // ---- 10. STOP + WOL wake ----
    // Arm magic-packet wake, trigger netsim (its reply queues
    // synchronously), then enter STOP. The sleep drain delivers the queued
    // frame while asleep; IRQ62 wakes us. CYCCNT across WFI discriminates
    // real sleep (>50k, one sleep-step is 120k) from a nop fall-through.
    {
        wkp_flag = 0;
        MACPMTCTL = 0x20 | 0x2; // W1C stale MPR away, arm MPE (PMTIM set)
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 4);
        tx_frame[off] = 5003 >> 8; tx_frame[off + 1] = 5003 & 0xFF;
        tx_frame[off + 2] = 5003 >> 8; tx_frame[off + 3] = 5003 & 0xFF;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 12;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        if (eth_send_frame(14 + 20 + 12, 0)) {
            // No DMARPDR re-arm here: the poll must be CLEAR at WFI so the
            // queued magic cannot be delivered pre-sleep (stale polls are
            // dropped by the driver). The sleep drain force-delivers it.
            rx_desc[0] = 0x80000000 | 1536;
            uart_puts("GOING TO STOP\r\n");
            dwt_zero();
            *(volatile unsigned int *)0xE000ED10 |= (1 << 2); // SCR SLEEPDEEP
            __asm__ volatile ("wfi");
            *(volatile unsigned int *)0xE000ED10 &= ~(1 << 2);
            unsigned int slept = dwt_rd();
            uart_puts("BACK FROM STOP\r\n");
            int hit = wkp_flag;
            for (int i = 0; i < 60 && !hit; i++) {
                (void)eth_recv_frame(20000);
                if (wkp_flag) hit = 1;
            }
            if (hit && (MACPMTCTL & 0x20) && slept > 50000)
                uart_puts("WOKE BY WOL\r\n");
            else { uart_puts("WOL WAKE FAIL "); uart_hex32(slept); uart_puts("\r\n"); }
            MACPMTCTL = MACPMTCTL | 0x20; // W1C: clear MPR now observed
        } else uart_puts("WOL WAKE TX TIMEOUT\r\n");
        MACPMTCTL = 0;
    }

    uart_puts("FEAT ALL PASS\r\n");
    uart_puts("FEAT Test: done\r\n");
    while (1);
}
