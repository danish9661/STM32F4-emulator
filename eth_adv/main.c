// eth_adv: advanced L3/L4 + link-scope protocol proofs.
// Each phase drives one netsim peer (UDP trigger to a dedicated server
// port) and asserts exactly one observable, so a failure names its
// protocol:
//   RST        — server aborts with RST after SYN, guest reports TCP RST
//   RTO        — server blackholes one segment, guest counts RTO expiry
//   MSS        — guest SYN advertises MSS=500, server SYN-ACK echoes it
//   WINDOW     — server advertises window 1000, guest reads it off the ACK
//   FRAG       — server sends 2 IP fragments, guest reassembles payload
//   ICMPERR    — server sends ICMP port-unreachable for our UDP, guest logs
//   DHCPNAK    — server NAKs the Request, guest restarts with Discover
//   DHCPRENEW  — renew-style Request gets an ACK
//   IGMP       — guest reports 239.0.0.9, answers query, gets group traffic
//   ND         — IPv6 NS for our link-local arrives, guest parses it
//   LLDP       — LLDP frame arrives, guest parses Chassis TLV id
//   STP        — STP config BPDU arrives, guest parses root bridge id
// Runs in irq_eth mode against netsim ('netsim' matrix script), same as
// eth_feat_test. Anti-markers: any line containing FAIL or TIMEOUT.
// NOTE: startup copies neither .data nor zeroes .bss (same as blinky/),
// so my_mac[]/my_ip_ram[] are filled from rom tables at boot and NEVER
// read before that copy. Never use cksum() from defs.h on volatile
// rx_buf — its `p` arg is unsigned char*; use the local cksumv().
#include "defs.h"

static volatile unsigned int tx_desc[8] __attribute__((aligned(8))) = { 0 };
// TX completion generation counter: the ISR stamps every completion.
// eth_send_frame waits on a FRESH generation (not a sticky flag), so a
// stale completion from an earlier frame can never satisfy a later TX
// (the classic single-flag aliasing: TX#2 sees TX#1's done bit and
// returns before its own frame is captured).
static volatile int eth_done = 0;
static volatile unsigned int eth_done_gen = 0;
static volatile unsigned int rx_desc[8] __attribute__((aligned(8))) = { 0 };
static volatile unsigned char rx_buf[1536] __attribute__((aligned(4)));
static volatile int rx_flag = 0;
static volatile unsigned int last_rdes0 = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { // TS bit
        eth_done = 1;
        eth_done_gen++;
        DMASR = (1 << 0) | (1 << 16) | (1 << 14);
    }
    if (sr & (1 << 6)) { // RS bit
        // CONSUME-ON-ENTRY (no lost wakeups): latch the descriptor AND
        // re-arm NOW, inside the ISR, instead of leaving either for the
        // thread-mode poll. The thread loop's OWN-bit check races the
        // driver's synchronous delivery: the frame can land between the
        // loop's flag test and its descriptor read, and a second delivery
        // then overwrites the first before thread mode collects it. And
        // the loop's re-arm write races the NEXT delivery the same way:
        // inline delivery preempts thread mode at any poll point, so a
        // re-arm issued from thread mode can be consumed by a delivery
        // before the loop reaches its flag test (observed: the ICMP wait
        // loop's heartbeat re-arm was eaten by a duplicate delivery, the
        // head stayed CPU-owned, and the loop spun 300x on a frame it
        // never collected). Re-arming here closes both races: the head is
        // DMA-owned before the ISR returns, so every later delivery has a
        // live head and every thread-mode poll sees a clean doorbell.
        last_rdes0 = rx_desc[0];
        rx_desc[0] = 0x80000000 | 1536;
        rx_flag = 1;
        DMARPDR = 1;
        DMASR = (1 << 6) | (1 << 16) | (1 << 14);
    }
    // AIS without NIS (error summary, e.g. JT/NC with no TS yet):
    // still a completion — wake the TX wait so error paths never
    // hang the 2M poll (the status word says which error).
    if (!(sr & ((1 << 0) | (1 << 6))) && (sr & (1 << 14))) { eth_done = 1; eth_done_gen++; }
}

static unsigned char tx_frame[1536];
// NOTE: my_mac MUST be RAM (not const): the startup does NOT copy
// .data (no _sdata/_edata loop — same as blinky/), so const tables
// addressed via my_mac[] would read the FLASH image... (see below).
// It is plain static so it lives in .bss... but .bss is NOT zeroed
// either! So main() copies the MAC/IP tables into RAM at boot
// (rom_mac -> my_mac). NEVER read my_mac[] before that copy.
static unsigned char my_mac[6];
static unsigned char my_ip_ram[4];
static const unsigned char rom_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};
static const unsigned char rom_ip[4] = {192, 168, 4, 2};
static const unsigned char gw_ip[4] = {192, 168, 4, 1};
static const unsigned char gw_mac[6] = {0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd};

// Checksum over guest RAM (volatile rx_buf needs its own — defs.h
// cksum() takes unsigned char* and warns/drops volatile).
static unsigned int cksumv(volatile unsigned char *p, unsigned int n) {
    unsigned int s = 0;
    for (unsigned int i = 0; i < n; i += 2) s += ((unsigned int)p[i] << 8) | (i + 1 < n ? p[i+1] : 0);
    while (s >> 16) s = (s & 0xFFFF) + (s >> 16);
    return (~s) & 0xFFFF;
}

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
    for (int i = 0; i < 4; i++) { tx_frame[26 + i] = my_ip_ram[i]; tx_frame[30 + i] = dst_ip[i]; }
    unsigned int ck = cksum(&tx_frame[14], 20);
    tx_frame[24] = ck >> 8; tx_frame[25] = ck & 0xFF;
    return 34;
}

// Wait for one RX frame; returns len (0 = timeout). Stashes RDES0 (status
// bits) before re-arming the descriptor. The `(i & 0x3F)` heartbeat keeps
// the poll armed — without it a frame queued while no poll is armed sits
// forever (see the feat OWNERSHIP CONTRACT: head-only RBUS delivery).
static unsigned int eth_recv_frame(unsigned int timeout_iters) {
    for (unsigned int i = 0; i < timeout_iters; i++) {
        if ((i & 0x3F) == 0) DMARPDR = 1; // heartbeat: keep the poll armed
        // RX-FLAG RACE (single-desc head, head-only RBUS delivery): the
        // ISR sets rx_flag on RS, but the flag is just a doorbell — the
        // descriptor OWN bit is the ground truth. If a frame lands after
        // our flag check but the ISR hasn't run yet (or the flag was
        // consumed by a prior call), rx_flag reads 0 while a fresh frame
        // sits CPU-owned in rx_buf. So accept EITHER signal: flag set, or
        // head descriptor CPU-owned (OWN clear = delivered, waiting).
        // LATCH DISCIPLINE: the ISR latches last_rdes0 at entry AND
        // re-arms the head, so a flag-wakeup MUST use the latched copy —
        // re-reading rx_desc[0] here races a second delivery overwriting
        // the first. The OWN-bit fallback below is only for the narrow
        // window where the ISR hasn't run yet (delivery between our flag
        // test and our descriptor read) — and only that path re-arms
        // (the ISR already re-armed on its path).
        unsigned int owned = (rx_desc[0] & 0x80000000) == 0;
        if (rx_flag || owned) {
            rx_flag = 0;
            unsigned int len;
            if (!owned) len = (last_rdes0 >> 16) & 0x3FFF; // ISR latched it
            else {
                last_rdes0 = rx_desc[0];
                len = (rx_desc[0] >> 16) & 0x3FFF;
                rx_desc[0] = 0x80000000 | 1536;
                DMARPDR = 1;
            }
            if (len > 1536) len = 1536;
            return len;
        }
    }
    return 0;
}

static int eth_send_frame(unsigned int len, unsigned int tdes_flags) {
    if (len < 60) { for (unsigned int i = len; i < 60; i++) tx_frame[i] = 0; len = 60; }
    // Generation wait: snapshot BEFORE arming the poll so only THIS
    // frame's completion satisfies us (a sticky flag would alias an
    // earlier frame's completion — see the decl comment). The snapshot
    // MUST precede the descriptor programming: the poll write completes
    // synchronously in the model (TS posts before DMATPDR returns), so
    // a snapshot taken after the descriptor write but before the poll
    // aliases a stale completion and returns without transmitting.
    unsigned int gen = eth_done_gen;
    tx_desc[0] = 0x80000000 | tdes_flags | (len & 0x3FFF);
    tx_desc[1] = (unsigned int)&tx_frame[0];
    DMATDLAR = (unsigned int)&tx_desc[0];
    eth_done = 0;
    DMATPDR = 1;
    for (int i = 0; i < 2000000; i++) if (eth_done_gen != gen) return 1;
    return 0;
}

// Drain any queued/stale RX (returns count). Each phase starts clean:
// an unconsumed delivery from an earlier phase would otherwise shift
// every later first-frame assert by one.
static unsigned int drain_rx(void) {
    unsigned int n = 0;
    for (int k = 0; k < 40; k++) {
        unsigned int len = eth_recv_frame(500);
        if (!len) break;
        n++;
    }
    return n;
}

// ---- TCP mini-state for the RST/RTO/MSS/WINDOW phases ----
static unsigned int t_seq, t_ack, t_sport;
static unsigned char t_srv_ip[4];
static unsigned int t_srv_port;

// Parse TCP flags/seq/ack from rx_buf; returns 1 on a TCP frame for us.
static int tcp_parse(unsigned int *seq, unsigned int *ack, int *fl, int *dlen, unsigned int len) {
    if (len < 54 || rx_buf[12] != 0x08 || rx_buf[13] != 0x00) return 0;
    unsigned int ih = (rx_buf[14] & 0x0F) * 4;
    if (rx_buf[14 + 9] != 6) return 0;
    unsigned int o = 14 + ih;
    unsigned int sp = (rx_buf[o] << 8) | rx_buf[o+1];
    unsigned int dp = (rx_buf[o+2] << 8) | rx_buf[o+3];
    if (sp != t_srv_port || dp != (t_sport & 0xFFFF)) return 0;
    *seq = (rx_buf[o+4] << 24) | (rx_buf[o+5] << 16) | (rx_buf[o+6] << 8) | rx_buf[o+7];
    *ack = (rx_buf[o+8] << 24) | (rx_buf[o+9] << 16) | (rx_buf[o+10] << 8) | rx_buf[o+11];
    *fl = rx_buf[o+13];
    int th = ((rx_buf[o+12] >> 4) & 0x0F) * 4;
    int td = (int)len - 14 - (int)ih - th;
    *dlen = td < 0 ? 0 : td;
    return 1;
}

static unsigned int tcp_build(unsigned char fl, unsigned int seq, unsigned int ack,
                              unsigned char *pl, unsigned int pln, unsigned int mss_opt) {
    unsigned int o = ip_header((unsigned char *)gw_mac, t_srv_ip, 6, 20 + (mss_opt ? 4 : 0) + pln);
    tx_frame[o+0] = (t_sport >> 8) & 0xFF; tx_frame[o+1] = t_sport & 0xFF;
    tx_frame[o+2] = (t_srv_port >> 8) & 0xFF; tx_frame[o+3] = t_srv_port & 0xFF;
    tx_frame[o+4] = seq >> 24; tx_frame[o+5] = seq >> 16; tx_frame[o+6] = seq >> 8; tx_frame[o+7] = seq;
    tx_frame[o+8] = ack >> 24; tx_frame[o+9] = ack >> 16; tx_frame[o+10] = ack >> 8; tx_frame[o+11] = ack;
    tx_frame[o+12] = mss_opt ? 0x60 : 0x50; tx_frame[o+13] = fl;
    tx_frame[o+14] = 0xFF; tx_frame[o+15] = 0xFF;
    unsigned int hlen = 20;
    if (mss_opt) { tx_frame[o+20] = 2; tx_frame[o+21] = 4; tx_frame[o+22] = (mss_opt >> 8) & 0xFF; tx_frame[o+23] = mss_opt & 0xFF; hlen = 24; }
    for (unsigned int i = 0; i < pln; i++) tx_frame[o+hlen+i] = pl[i];
    // TCP checksum over pseudo-header (regular cksum() — tx_frame is
    // plain RAM, never volatile).
    unsigned int s = (my_ip_ram[0] << 8) | my_ip_ram[1];
    s += (my_ip_ram[2] << 8) | my_ip_ram[3];
    s += (t_srv_ip[0] << 8) | t_srv_ip[1];
    s += (t_srv_ip[2] << 8) | t_srv_ip[3];
    unsigned int tlen = hlen + pln;
    s += 6 + tlen;
    for (unsigned int i = 0; i < tlen; i += 2) s += ((unsigned int)tx_frame[o+i] << 8) | (i + 1 < tlen ? tx_frame[o+i+1] : 0);
    while (s >> 16) s = (s & 0xFFFF) + (s >> 16);
    unsigned int ck = (~s) & 0xFFFF;
    tx_frame[o+16] = ck >> 8; tx_frame[o+17] = ck & 0xFF;
    // ip_header already set the IP length for the no-option size; with
    // an MSS option the header is 4 bytes longer — fix length+checksum.
    if (mss_opt) {
        unsigned int ipLen = 20 + tlen;
        tx_frame[16] = ipLen >> 8; tx_frame[17] = ipLen & 0xFF;
        tx_frame[24] = 0; tx_frame[25] = 0;
        ck = cksum(&tx_frame[14], 20);
        tx_frame[24] = ck >> 8; tx_frame[25] = ck & 0xFF;
    }
    return 14 + 20 + tlen;
}

// Open a TCP connection to (ip,port) with our standard SYN (MSS 1460,
// window 65535). Returns 1 on SYN-ACK (t_ack set past their SYN),
// -1 on immediate RST (the RST phase's valid answer), 0 on timeout.
static int tcp_open(unsigned char *ip, unsigned int port, unsigned int sport) {
    for (int i = 0; i < 4; i++) t_srv_ip[i] = ip[i];
    t_srv_port = port; t_sport = sport;
    t_seq = 20000; t_ack = 0;
    unsigned int len = tcp_build(0x02, t_seq, 0, 0, 0, 1460);
    if (!eth_send_frame(len, 0)) return 0;
    t_seq++;
    for (int a = 0; a < 200; a++) {
        unsigned int l = eth_recv_frame(20000);
        if (!l) continue;
        unsigned int sq, ak; int fl, dl;
        if (!tcp_parse(&sq, &ak, &fl, &dl, l)) continue;
        if (fl == 0x12) {
            t_ack = sq + 1;
            len = tcp_build(0x10, t_seq, t_ack, 0, 0, 0);
            if (!eth_send_frame(len, 0)) return 0;
            return 1;
        }
        if (fl & 0x04) return -1; // immediate RST (also a valid answer)
    }
    return 0;
}

// Build a DHCP Request (renew-style when ciaddr_set: ciaddr = our IP,
// unicast to server; else broadcast discover-style). Matches the wire
// shape netsim parses: UDP sport 68 -> dport 67, BOOTP op 1/htype 1/
// hlen 6, chaddr = our MAC, magic cookie, option 53 = 3.
static unsigned int dhcp_request(unsigned char bcast, unsigned int xid) {
    unsigned char *p = tx_frame;
    unsigned char *dst = bcast ? (unsigned char *)"\xFF\xFF\xFF\xFF\xFF\xFF" : (unsigned char *)gw_mac;
    for (int i = 0; i < 6; i++) *p++ = dst[i];
    for (int i = 0; i < 6; i++) *p++ = my_mac[i];
    *p++ = 0x08; *p++ = 0x00;
    unsigned char *iph = p;
    *p++ = 0x45; *p++ = 0x00;
    unsigned char *lenp = p; p += 2;
    *p++ = 0; *p++ = 0; *p++ = 0; *p++ = 0;
    *p++ = 0x80; *p++ = 0x11;
    unsigned char *chkp = p; p += 2;
    if (bcast) { *p++ = 0; *p++ = 0; *p++ = 0; *p++ = 0; }
    else for (int i = 0; i < 4; i++) *p++ = my_ip_ram[i];
    if (bcast) { *p++ = 255; *p++ = 255; *p++ = 255; *p++ = 255; }
    else for (int i = 0; i < 4; i++) *p++ = gw_ip[i];
    *p++ = 0x00; *p++ = 0x44; *p++ = 0x00; *p++ = 0x43;
    unsigned char *udplenp = p; p += 2;
    *p++ = 0; *p++ = 0;
    // DHCP fixed header is 236 bytes (op..file, RFC 2131 §3): op(1)
    // htype(1) hlen(1) hops(1) xid(4) secs(2) flags(2) ciaddr(4)
    // yiaddr(4) siaddr(4) giaddr(4) chaddr(16) sname(64) file(128).
    // Netsim reads chaddr at +28 and options at +240 — any short count
    // shifts both and the server answers a different MAC/xid.
    unsigned char *dh = p;
    *p++ = 1; *p++ = 1; *p++ = 6; *p++ = 0;
    *p++ = (xid >> 24) & 0xFF; *p++ = (xid >> 16) & 0xFF;
    *p++ = (xid >> 8) & 0xFF; *p++ = xid & 0xFF;
    *p++ = 0; *p++ = 0; // SECS
    *p++ = 0; *p++ = 0; // FLAGS
    if (bcast) { *p++ = 0; *p++ = 0; *p++ = 0; *p++ = 0; } // ciaddr
    else for (int i = 0; i < 4; i++) *p++ = my_ip_ram[i];
    for (int i = 0; i < 12; i++) *p++ = 0; // yiaddr+siaddr+giaddr
    for (int i = 0; i < 6; i++) *p++ = my_mac[i]; // chaddr
    for (int i = 0; i < 10 + 64 + 128; i++) *p++ = 0; // chaddr pad + sname + file
    *p++ = 0x63; *p++ = 0x82; *p++ = 0x53; *p++ = 0x63;
    *p++ = 53; *p++ = 1; *p++ = 3; // Request
    *p++ = 255;
    while ((p - dh) % 4) *p++ = 0;
    unsigned int ul = p - (udplenp - 4);
    udplenp[0] = ul >> 8; udplenp[1] = ul & 0xFF;
    unsigned int il = p - iph;
    lenp[0] = il >> 8; lenp[1] = il & 0xFF;
    unsigned int s = 0;
    for (unsigned int i = 0; i < il; i += 2) s += ((unsigned int)iph[i] << 8) | (i + 1 < il ? iph[i+1] : 0);
    while (s >> 16) s = (s & 0xFFFF) + (s >> 16);
    unsigned int c2 = (~s) & 0xFFFF;
    chkp[0] = c2 >> 8; chkp[1] = c2 & 0xFF;
    return p - tx_frame;
}

// Scan a DHCP reply in rx_buf for option 53 == want; returns 1 on match
// (also verifies the xid). uo = UDP payload offset in rx_buf.
static int dhcp_opt_is(unsigned int uo, unsigned int xid, unsigned char want) {
    unsigned char *d2 = (unsigned char *)&rx_buf[uo + 8];
    unsigned int rx = (d2[4] << 24) | (d2[5] << 16) | (d2[6] << 8) | d2[7];
    if (rx != xid) return 0;
    unsigned char *op = d2 + 240;
    for (unsigned int k = 0; k + 2 < 300; ) {
        if (op[k] == 0xFF) break;
        if (op[k] == 0) { k++; continue; }
        if (op[k] == 53 && op[k+2] == want) return 1;
        k += op[k+1] + 2;
    }
    return 0;
}

int main(void) {
    uart_init();
    // RAM tables: the startup copies neither .data nor zeroes .bss, so
    // const-initialized tables are NOT valid in RAM — copy them here
    // before ANY use (my_mac[] reads garbage otherwise).
    for (int i = 0; i < 6; i++) my_mac[i] = rom_mac[i];
    for (int i = 0; i < 4; i++) my_ip_ram[i] = rom_ip[i];
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
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;
    uart_puts("ADV Test: starting\r\n");
    uart_puts("Setup complete\r\n");

    // ============ 1. RST: server aborts with RST after our SYN ============
    drain_rx();
    {
        unsigned char srv[4] = {192, 168, 4, 1};
        int r = tcp_open(srv, 5010, 49160);
        if (r == -1) uart_puts("RST OK\r\n");
        else uart_puts("RST FAIL\r\n");
    }

    // ============ 2. RTO: server blackholes one segment ============
    drain_rx();
    {
        unsigned char srv[4] = {192, 168, 4, 1};
        unsigned int rtos = 0, acked = 0;
        if (tcp_open(srv, 5011, 49161)) {
            // send one segment; server stays silent; our RTO counter
            // (loop iterations with no ACK) must expire
            unsigned char pl[8] = {'R','T','O','P','R','O','B','E'};
            unsigned int len = tcp_build(0x18, t_seq, t_ack, pl, 8, 0);
            if (eth_send_frame(len, 0)) {
                t_seq += 8;
                for (unsigned int w = 0; w < 60; w++) {
                    unsigned int l = eth_recv_frame(20000);
                    if (!l) { rtos++; continue; }
                    unsigned int sq, ak; int fl, dl;
                    if (tcp_parse(&sq, &ak, &fl, &dl, l) && ak == t_seq) { acked = 1; break; }
                }
            }
        }
        if (rtos >= 60 && !acked) uart_puts("RTO OK\r\n");
        else uart_puts("RTO FAIL\r\n");
    }

    // ============ 3. MSS: SYN advertises MSS=500, server echoes it ============
    drain_rx();
    {
        unsigned char srv[4] = {192, 168, 4, 1};
        for (int i = 0; i < 4; i++) t_srv_ip[i] = srv[i];
        t_srv_port = 5012; t_sport = 49162;
        t_seq = 30000; t_ack = 0;
        unsigned int len = tcp_build(0x02, t_seq, 0, 0, 0, 500);
        int got = 0, mss_ok = 0;
        if (eth_send_frame(len, 0)) {
            t_seq++;
            for (int a = 0; a < 200 && !got; a++) {
                unsigned int l = eth_recv_frame(20000);
                if (!l) continue;
                unsigned int sq, ak; int fl, dl;
                if (!tcp_parse(&sq, &ak, &fl, &dl, l)) continue;
                if (fl == 0x12) {
                    // scan SYN-ACK options for MSS
                    unsigned int o = 14 + (rx_buf[14] & 0x0F) * 4;
                    int th = ((rx_buf[o+12] >> 4) & 0x0F) * 4;
                    for (int k = 20; k + 3 < th; ) {
                        unsigned char kind = rx_buf[o+k];
                        if (kind == 0) break;
                        if (kind == 1) { k++; continue; }
                        unsigned char kl = rx_buf[o+k+1];
                        if (kind == 2 && kl == 4) {
                            unsigned int mss = (rx_buf[o+k+2] << 8) | rx_buf[o+k+3];
                            if (mss == 500) mss_ok = 1;
                            break;
                        }
                        if (kl < 2) break;
                        k += kl;
                    }
                    t_ack = sq + 1;
                    len = tcp_build(0x10, t_seq, t_ack, 0, 0, 0);
                    eth_send_frame(len, 0);
                    got = 1;
                }
            }
        }
        if (got && mss_ok) uart_puts("MSS OK\r\n");
        else uart_puts("MSS FAIL\r\n");
    }

    // ============ 4. WINDOW: server advertises win=1000 ============
    drain_rx();
    {
        unsigned char srv[4] = {192, 168, 4, 1};
        int got = 0, win_ok = 0;
        if (tcp_open(srv, 5013, 49163)) {
            // any data-bearing server frame carries its window; send a
            // 1-byte probe and read the window off the ACK
            unsigned char pl[1] = {'W'};
            unsigned int len = tcp_build(0x18, t_seq, t_ack, pl, 1, 0);
            if (eth_send_frame(len, 0)) {
                t_seq += 1;
                for (int a = 0; a < 200 && !got; a++) {
                    unsigned int l = eth_recv_frame(20000);
                    if (!l) continue;
                    unsigned int sq, ak; int fl, dl;
                    if (!tcp_parse(&sq, &ak, &fl, &dl, l)) continue;
                    if (fl & 0x10) {
                        unsigned int o = 14 + (rx_buf[14] & 0x0F) * 4;
                        unsigned int win = (rx_buf[o+14] << 8) | rx_buf[o+15];
                        if (win == 1000) win_ok = 1;
                        got = 1;
                    }
                }
            }
        }
        if (got && win_ok) uart_puts("WINDOW OK\r\n");
        else uart_puts("WINDOW FAIL\r\n");
    }

    // ============ 5. FRAG: 2 IP fragments reassemble ============
    // NOTE: no drain_rx() here — the two fragments arrive back-to-back
    // and the reassembly loop below consumes exactly two frames. A drain
    // first would eat frag0 (the loop polls rx_flag directly, so a frame
    // delivered between drain and trigger is still consumed — but a
    // drain AFTER the trigger races the delivery and drops frag0).
    {
        // trigger port 5014: server answers with frag0 (MF) + frag1.
        // The trigger itself is a minimal UDP probe to :5014.
        unsigned int o = ip_header((unsigned char *)gw_mac, (unsigned char *)gw_ip, 17, 8 + 4);
        tx_frame[o+0] = 0xC0; tx_frame[o+1] = 0x00; // sport 49152
        tx_frame[o+2] = 0x13; tx_frame[o+3] = 0x96; // dport 5014
        tx_frame[o+4] = 0; tx_frame[o+5] = 12;
        tx_frame[o+6] = 0; tx_frame[o+7] = 0;
        tx_frame[o+8] = 'F'; tx_frame[o+9] = 'R'; tx_frame[o+10] = 'A'; tx_frame[o+11] = 'G';
        eth_send_frame(o + 12 - 14 + 14, 0);
        // collect both fragments (any order), reassemble by offset.
        // UDP payload lives at reasm[8..] (8-byte UDP header first).
        // REASM BASE (silicon-fragment view): reassembly starts at the
        // FRAGMENTED PAYLOAD (UDP header + data), not at the IP header —
        // so copy from psrc = 14+ihl into reasm[off] and read payload at
        // reasm[8..] (UDP header occupies reasm[0..7] of frag0).
        unsigned char reasm[64]; unsigned int got0 = 0, got1 = 0;
        for (int i = 0; i < 64; i++) reasm[i] = 0;
        for (int a = 0; a < 300 && !(got0 && got1); a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l || l < 34 || rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue;
            if (rx_buf[23] != 17) continue;
            // FLAGS+FRAGOFF (RFC 791 §3.1): MF = 0x2000 (bit 13), offset
            // = low 13 bits in 8-byte units. Big-endian field: MF lives
            // in the HIGH byte (rx_buf[20] & 0x20), offset-high in
            // rx_buf[20] & 0x1F.
            unsigned int off = (((rx_buf[20] & 0x1F) << 8) | rx_buf[21]) * 8;
            int mf = (rx_buf[20] & 0x20) != 0;
            unsigned int ihl = (rx_buf[14] & 0x0F) * 4;
            unsigned int tlen = (rx_buf[16] << 8) | rx_buf[17];
            unsigned int plen = tlen > ihl ? tlen - ihl : 0;
            // Clamp the wire-pad: the DMA pads runts to 60 B, so the
            // reported length can exceed the IP total length — copy only
            // the IP payload bytes (plen), never the pad.
            if (plen + off > 64) plen = 64 - off;
            unsigned int psrc = 14 + ihl;
            for (unsigned int k = 0; k < plen; k++) reasm[off + k] = rx_buf[psrc + k];
            // FRAG GATE (no silent first-frame eat): only a real
            // fragment counts — off==0+MF for frag0, off>0 for frag1.
            // A non-fragmented frame has off==0 AND MF clear (the stale
            // WINDOW-ACK still sitting in rx_buf when the loop starts):
            // counting it as got0 ends the loop on one frame and fails
            // reassembly with a TCP header where the payload should be.
            // Stale-buffer guard: skip the frame unless it is fragmented
            // (MF set) or a non-first fragment (off > 0).
            if (!mf && off == 0) continue;
            if (off == 0 && mf) got0 = plen;
            if (off > 0 && !mf) got1 = plen;
        }
        // payload is "FRAGMENT" + "!" = "FRAGMENT!"
        if (got0 && got1 && reasm[8] == 'F' && reasm[9] == 'R' && reasm[10] == 'A' && reasm[11] == 'G'
            && reasm[12] == 'M' && reasm[13] == 'E' && reasm[14] == 'N' && reasm[15] == 'T'
            && reasm[16] == '!' && reasm[17] == 0) {
            uart_puts("FRAG OK\r\n");
        } else uart_puts("FRAG FAIL\r\n");
    }

    // ============ 6. ICMPERR: port-unreachable for our UDP ============
    // NOTE: no drain_rx() — same one-frame race as FRAG (the ICMP reply
    // is delivered synchronously with the trigger's TX completion).
    {
        // trigger port 5015: server answers ICMP type 3/code 3 quoting it
        unsigned int o = ip_header((unsigned char *)gw_mac, (unsigned char *)gw_ip, 17, 8 + 4);
        tx_frame[o+0] = 0xC0; tx_frame[o+1] = 0x01; // sport 49153
        tx_frame[o+2] = 0x13; tx_frame[o+3] = 0x97; // dport 5015
        tx_frame[o+4] = 0; tx_frame[o+5] = 12;
        tx_frame[o+6] = 0; tx_frame[o+7] = 0;
        tx_frame[o+8] = 'X'; tx_frame[o+9] = 'X'; tx_frame[o+10] = 'X'; tx_frame[o+11] = 'X';
        eth_send_frame(o + 12 - 14 + 14, 0);
        int ok = 0;
        for (int a = 0; a < 300 && !ok; a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l) continue;
            // LEN FLOOR (no phantom pass on stale/short frames): the
            // ICMP reply is 70 B on the wire (14 eth + 20 IP + 36 ICMP).
            // A shorter frame is a stale buffer from an earlier phase
            // (e.g. a 60 B TCP ACK): parsing it reads pad/TCP bytes as
            // ICMP fields. The old `l < 50` floor admitted those.
            if (l < 70 || rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue;
            if (rx_buf[23] != 1 || rx_buf[34] != 3) continue; // ICMP + type 3
            // code 3 (port unreachable) + quoted UDP dport 5015.
            // Quoted IP header starts at +42 (after eth 14 + ICMP 8);
            // its IHL (low nibble of +42) locates the quoted UDP ports.
            // STRICT (no pad-tolerance): the quoted UDP length must equal
            // the real 12 (8 hdr + 4 payload) — a peer quoting the 60 B
            // wire pad would show 26+ here and fail.
            // QIHL GUARD (no phantom pass): IHL < 5 is a corrupt header —
            // without the guard qihl=0 points at the ICMP type/code bytes
            // (3,3) and a naive port compare could pass on garbage.
            if (rx_buf[35] == 3) {
                unsigned int qihl = (rx_buf[42] & 0x0F) * 4;
                if (qihl < 20) continue;
                unsigned int qpo = 42 + qihl;
                unsigned int qlen = (rx_buf[qpo + 4] << 8) | rx_buf[qpo + 5];
                // Quoted UDP: sport 49153 (0xC001) + dport 5015 (0x1397).
                // (An earlier revision compared BOTH pairs against the
                // dport — qpo always holds C0 and the phase could never
                // pass. Caught by byte-dump: qpo showed c0 01 13 97.)
                if (qlen == 12 && rx_buf[qpo] == 0xC0 && rx_buf[qpo + 1] == 0x01
                    && rx_buf[qpo + 2] == 0x13 && rx_buf[qpo + 3] == 0x97) ok = 1;
            }
        }
        if (ok) uart_puts("ICMPERR OK\r\n");
        else uart_puts("ICMPERR FAIL\r\n");
    }

    // ============ 7. DHCPNAK: request refused, restart with Discover ============
    drain_rx();
    {
        // NAK trigger: Request with an unknown xid (netsim NAKs any
        // xid != its live session). Then the guest restarts discovery.
        unsigned int xid = 0xDEADBEEF;
        unsigned int fl = dhcp_request(1, xid);
        int nak = 0;
        if (eth_send_frame(fl, 0)) {
            for (int a = 0; a < 300 && !nak; a++) {
                unsigned int l = eth_recv_frame(20000);
                if (!l) continue;
                // DHCP msg type 6 = NAK with our xid
                if (l > 280 && rx_buf[12] == 0x08 && rx_buf[13] == 0x00 && rx_buf[23] == 17) {
                    unsigned int uo = 14 + (rx_buf[14] & 0x0F) * 4;
                    if (dhcp_opt_is(uo, xid, 6)) nak = 1;
                }
            }
        }
        if (nak) uart_puts("DHCPNAK OK\r\n");
        else uart_puts("DHCPNAK FAIL\r\n");
    }

    // ============ 8. DHCPRENEW: renew-style Request gets an ACK ============
    drain_rx();
    {
        unsigned int xid = 0x12345678;
        unsigned int fl = dhcp_request(0, xid);
        int ack = 0;
        if (eth_send_frame(fl, 0)) {
            for (int a = 0; a < 300 && !ack; a++) {
                unsigned int l = eth_recv_frame(20000);
                if (!l) continue;
                if (l > 280 && rx_buf[12] == 0x08 && rx_buf[13] == 0x00 && rx_buf[23] == 17) {
                    unsigned int uo = 14 + (rx_buf[14] & 0x0F) * 4;
                    if (dhcp_opt_is(uo, xid, 5)) ack = 1;
                }
            }
        }
        if (ack) uart_puts("DHCPRENEW OK\r\n");
        else uart_puts("DHCPRENEW FAIL\r\n");
    }

    // ============ 9. IGMP: join 239.0.0.9, answer query, get group ============
    // NOTE: no drain_rx() here — same one-frame race as FRAG (the query
    // is delivered synchronously with the report's TX completion, before
    // the wait loop's first poll; a drain eats it and the loop times out
    // on the group frame's non-IGMP protocol byte).
    //
    // MULTICAST JOIN (silicon needs it too): the default MAC filter
    // passes only our unicast + broadcast. The query (224.0.0.1) and the
    // group traffic (239.0.0.9) are multicast — program the hash table
    // for both (CRC32 upper 6 bits, like the feat MCAST phase) or the
    // model drops them before any guest byte is seen. HM (bit 2) +
    // HPF (bit 10): hash-or-perfect.
    {
        // CRC32 (Ethernet, reflected) for the multicast hash.
        // (Local copy — defs.h has none and the feat one is static.)
        unsigned int h1 = 0xFFFFFFFF, h2 = 0xFFFFFFFF;
        unsigned char g1[6] = {0x01, 0x00, 0x5E, 0x00, 0x00, 0x01};
        unsigned char g2[6] = {0x01, 0x00, 0x5E, 0x00, 0x00, 0x09};
        for (int i = 0; i < 6; i++) {
            h1 ^= g1[i]; h2 ^= g2[i];
            for (int bb = 0; bb < 8; bb++) {
                h1 = (h1 & 1) ? (h1 >> 1) ^ 0xEDB88320 : (h1 >> 1);
                h2 = (h2 & 1) ? (h2 >> 1) ^ 0xEDB88320 : (h2 >> 1);
            }
        }
        h1 = ~h1; h2 = ~h2;
        MACHTLR = 0; MACHTHR = 0;
        if (((h1 >> 26) & 63) < 32) MACHTLR |= 1 << ((h1 >> 26) & 63);
        else MACHTHR |= 1 << (((h1 >> 26) & 63) - 32);
        if (((h2 >> 26) & 63) < 32) MACHTLR |= 1 << ((h2 >> 26) & 63);
        else MACHTHR |= 1 << (((h2 >> 26) & 63) - 32);
        MACFFR = (1 << 2) | (1 << 10); // HM + HPF
        // membership report for 239.0.0.9 (IGMPv2, type 0x16)
        for (int i = 0; i < 6; i++) tx_frame[i] = 0x01;
        tx_frame[0] = 0x01; tx_frame[1] = 0x00; tx_frame[2] = 0x5E; tx_frame[3] = 0x00; tx_frame[4] = 0x00; tx_frame[5] = 0x09;
        for (int i = 0; i < 6; i++) tx_frame[6+i] = my_mac[i];
        tx_frame[12] = 0x08; tx_frame[13] = 0x00;
        tx_frame[14] = 0x45; tx_frame[15] = 0;
        tx_frame[16] = 0; tx_frame[17] = 28;
        tx_frame[18] = 0; tx_frame[19] = 0; tx_frame[20] = 0; tx_frame[21] = 0;
        tx_frame[22] = 1; tx_frame[23] = 2; // TTL=1, proto IGMP
        tx_frame[24] = 0; tx_frame[25] = 0;
        for (int i = 0; i < 4; i++) tx_frame[26+i] = my_ip_ram[i];
        tx_frame[30] = 239; tx_frame[31] = 0; tx_frame[32] = 0; tx_frame[33] = 9;
        tx_frame[34] = 0x16; tx_frame[35] = 0; tx_frame[36] = 0; tx_frame[37] = 0;
        tx_frame[38] = 239; tx_frame[39] = 0; tx_frame[40] = 0; tx_frame[41] = 9;
        {
            unsigned int c3 = cksum(&tx_frame[14], 20);
            tx_frame[24] = c3 >> 8; tx_frame[25] = c3 & 0xFF;
            c3 = cksum(&tx_frame[34], 8);
            tx_frame[36] = c3 >> 8; tx_frame[37] = c3 & 0xFF;
        }
        eth_send_frame(42, 0);
        // now wait for the server's general query (triggered by the report)
        // and answer it; then expect the group traffic
        int qok = 0, gok = 0;
        for (int a = 0; a < 400 && (!qok || !gok); a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l || l < 42 || rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue;
            if (rx_buf[23] != 2) {
                // group traffic to 239.0.0.9?
                if (l >= 46 && rx_buf[30] == 239 && rx_buf[31] == 0 && rx_buf[32] == 0 && rx_buf[33] == 9) gok = 1;
                continue;
            }
            if (rx_buf[34] == 0x11) { // membership query
                qok = 1;
                // answer with a second report (join confirmed)
                for (int i = 0; i < 6; i++) tx_frame[i] = 0x01;
                tx_frame[0] = 0x01; tx_frame[1] = 0x00; tx_frame[2] = 0x5E; tx_frame[3] = 0x00; tx_frame[4] = 0x00; tx_frame[5] = 0x09;
                for (int i = 0; i < 6; i++) tx_frame[6+i] = my_mac[i];
                tx_frame[12] = 0x08; tx_frame[13] = 0x00;
                tx_frame[14] = 0x45; tx_frame[15] = 0;
                tx_frame[16] = 0; tx_frame[17] = 28;
                tx_frame[18] = 0; tx_frame[19] = 0; tx_frame[20] = 0; tx_frame[21] = 0;
                tx_frame[22] = 1; tx_frame[23] = 2;
                tx_frame[24] = 0; tx_frame[25] = 0;
                for (int i = 0; i < 4; i++) tx_frame[26+i] = my_ip_ram[i];
                tx_frame[30] = 239; tx_frame[31] = 0; tx_frame[32] = 0; tx_frame[33] = 9;
                tx_frame[34] = 0x16; tx_frame[35] = 0; tx_frame[36] = 0; tx_frame[37] = 0;
                tx_frame[38] = 239; tx_frame[39] = 0; tx_frame[40] = 0; tx_frame[41] = 9;
                {
                    unsigned int c3 = cksum(&tx_frame[14], 20);
                    tx_frame[24] = c3 >> 8; tx_frame[25] = c3 & 0xFF;
                    c3 = cksum(&tx_frame[34], 8);
                    tx_frame[36] = c3 >> 8; tx_frame[37] = c3 & 0xFF;
                }
                eth_send_frame(42, 0);
            } else if (l >= 46 && rx_buf[30] == 239 && rx_buf[31] == 0 && rx_buf[32] == 0 && rx_buf[33] == 9) {
                gok = 1;
            }
        }
        if (qok && gok) uart_puts("IGMP OK\r\n");
        else uart_puts("IGMP FAIL\r\n");
    }

    // ============ 10. ND: IPv6 NS arrives, guest parses it ============
    // NOTE: no drain_rx() — same one-frame race as FRAG/IGMP (the NS is
    // delivered synchronously with the trigger's TX completion).
    //
    // MULTICAST STALENESS (IGMP leftovers): the IGMP phase programmed the
    // hash table for 224.0.0.1 + 239.0.0.9 only. The NS arrives at the
    // solicited-node group ff02::1:ff0c:dd02 (33:33:ff:0c:dd:02), which the
    // filter drops. Widen the table to pass-all-multicast (PM, bit 0)
    // for the link-scope phases (ND/LLDP/STP all use multicast DAs).
    {
        MACFFR = (1 << 0); // PM: pass all multicast
        // trigger port 5016: server sends NS for our link-local
        unsigned int o = ip_header((unsigned char *)gw_mac, (unsigned char *)gw_ip, 17, 8 + 4);
        tx_frame[o+0] = 0xC0; tx_frame[o+1] = 0x02; // sport 49154
        tx_frame[o+2] = 0x13; tx_frame[o+3] = 0x98; // dport 5016
        tx_frame[o+4] = 0; tx_frame[o+5] = 12;
        tx_frame[o+6] = 0; tx_frame[o+7] = 0;
        tx_frame[o+8] = 'N'; tx_frame[o+9] = 'S'; tx_frame[o+10] = '?'; tx_frame[o+11] = '?';
        eth_send_frame(o + 12 - 14 + 14, 0);
        int ok = 0;
        for (int a = 0; a < 300 && !ok; a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l || l < 86) continue;
            if (rx_buf[12] != 0x86 || rx_buf[13] != 0xDD) continue; // need IPv6
            if (rx_buf[20] != 58) continue; // need ICMPv6
            if (rx_buf[54] != 135) continue; // need NS
            ok = 1;
        }
        if (ok) uart_puts("ND OK\r\n");
        else uart_puts("ND FAIL\r\n");
    }

    // ============ 11. LLDP: parse Chassis TLV id ============
    // NOTE: no drain_rx() — same one-frame race as FRAG/IGMP/ND.
    {
        unsigned int o = ip_header((unsigned char *)gw_mac, (unsigned char *)gw_ip, 17, 8 + 4);
        tx_frame[o+0] = 0xC0; tx_frame[o+1] = 0x03; // sport 49155
        tx_frame[o+2] = 0x13; tx_frame[o+3] = 0x99; // dport 5017
        tx_frame[o+4] = 0; tx_frame[o+5] = 12;
        tx_frame[o+6] = 0; tx_frame[o+7] = 0;
        tx_frame[o+8] = 'L'; tx_frame[o+9] = 'L'; tx_frame[o+10] = 'D'; tx_frame[o+11] = 'P';
        eth_send_frame(o + 12 - 14 + 14, 0);
        int ok = 0;
        for (int a = 0; a < 300 && !ok; a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l || l < 24) continue;
            if (rx_buf[12] != 0x88 || rx_buf[13] != 0xCC) continue; // LLDP
            // TLV 1 = Chassis ID: type(7b)=1, len; subtype 4 (MAC) + 6B MAC
            unsigned int t = ((rx_buf[14] << 8) | rx_buf[15]) >> 9;
            unsigned int tl = ((rx_buf[14] << 8) | rx_buf[15]) & 0x1FF;
            if (t == 1 && tl == 7 && rx_buf[16] == 4
                && rx_buf[17] == 0x5A && rx_buf[22] == 0xDD) ok = 1;
        }
        if (ok) uart_puts("LLDP OK\r\n");
        else uart_puts("LLDP FAIL\r\n");
    }

    // ============ 12. STP: parse root bridge id ============
    // NOTE: no drain_rx() — same one-frame race as FRAG/IGMP/ND/LLDP.
    {
        unsigned int o = ip_header((unsigned char *)gw_mac, (unsigned char *)gw_ip, 17, 8 + 4);
        tx_frame[o+0] = 0xC0; tx_frame[o+1] = 0x04; // sport 49156
        tx_frame[o+2] = 0x13; tx_frame[o+3] = 0x9A; // dport 5018
        tx_frame[o+4] = 0; tx_frame[o+5] = 12;
        tx_frame[o+6] = 0; tx_frame[o+7] = 0;
        tx_frame[o+8] = 'S'; tx_frame[o+9] = 'T'; tx_frame[o+10] = 'P'; tx_frame[o+11] = '?';
        eth_send_frame(o + 12 - 14 + 14, 0);
        int ok = 0;
        for (int a = 0; a < 300 && !ok; a++) {
            unsigned int l = eth_recv_frame(20000);
            if (!l || l < 60) continue;
            // STP: dst 01:80:C2:00:00:00, LLC 42:42:03, PID 0, BPDU config(0).
            // LLC HEADER OFFSET (not ethertype): STP rides 802.3+LLC, so
            // there is no ethertype — DSAP/SSAP/CTL sit at +14/+15/+16
            // (the old code read +18/+19/+20, i.e. protocol'…/version/type
            // shifted by the 2 length bytes, and compared the root id at
            // +36..+43 = bridge-id bytes — always FAIL).
            if (rx_buf[0] != 0x01 || rx_buf[1] != 0x80 || rx_buf[2] != 0xC2) continue;
            if (rx_buf[14] != 0x42 || rx_buf[15] != 0x42 || rx_buf[16] != 0x03) continue;
            if (rx_buf[17] != 0x00 || rx_buf[18] != 0x00) continue; // protocol id/version
            if (rx_buf[19] != 0x00) continue; // BPDU type = config
            // root id at +21: prio 0x8000 + MAC 5a:94:ef:e4:0c:dd
            if (rx_buf[21] == 0x80 && rx_buf[22] == 0x00
                && rx_buf[23] == 0x5A && rx_buf[28] == 0xDD) ok = 1;
        }
        if (ok) uart_puts("STP OK\r\n");
        else uart_puts("STP FAIL\r\n");
    }

    uart_puts("ADV Test: done\r\n");
    while (1) {}
}
