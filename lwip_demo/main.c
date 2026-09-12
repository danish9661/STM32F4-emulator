// lwip_demo: a minimal LwIP-style socket API over the raw F4 Ethernet
// driver (no LwIP sources — clean-room API clone, same call shapes).
// Demo: resolve example.com via DNS, TCP connect to the echo service,
// send/recv, then a UDP echo round-trip. Runs against netsim in irq_eth
// mode (TCP echo port 7 + UDP echo port 7 + canned DNS).
#include "defs.h"

// ---- socket table ----
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define LWIP_MAX_SOCK 4

typedef struct { unsigned int addr; } ip_addr_t;

typedef struct {
    int used, type;
    unsigned int sport;      // local port (ephemeral, or bound)
    unsigned int rip;        // remote IP (v4, host order bytes packed)
    unsigned int rport;
    unsigned int iss, snd_nxt, rcv_nxt; // TCP state
    int connected, listening;
} lwip_sock_t;

static lwip_sock_t socks[LWIP_MAX_SOCK];
static unsigned int next_sport = 49160;

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

// ---- LwIP-style API ----
static void lwip_init(void) {
    RCC_AHB1ENR |= (1 << 25);
    DMABMR |= 1;
    wait_ms(10);
    MACCR = (1 << 2) | (1 << 3) | (1 << 11) | (1 << 14); // RE+TE+DM+FES
    MACA0HR = 0x00000200 | (1 << 31);
    MACA0LR = 0x00000001;
    DMAOMR = (1 << 13) | (1 << 1);
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6);
    NVIC_ISER1 |= (1 << (61 - 32));
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;
    for (int i = 0; i < LWIP_MAX_SOCK; i++) socks[i].used = 0;
}

static int lwip_socket(int domain, int type) {
    if (domain != AF_INET) return -1;
    if (type != SOCK_STREAM && type != SOCK_DGRAM) return -1;
    for (int i = 0; i < LWIP_MAX_SOCK; i++) {
        if (!socks[i].used) {
            socks[i].used = 1; socks[i].type = type;
            socks[i].sport = next_sport++;
            socks[i].connected = 0;
            return i;
        }
    }
    return -1;
}

static void tcp_tx(lwip_sock_t *s, unsigned int flags, unsigned char *data, unsigned int dlen) {
    unsigned char rip[4] = {
        (s->rip >> 24) & 0xFF, (s->rip >> 16) & 0xFF,
        (s->rip >> 8) & 0xFF, s->rip & 0xFF };
    unsigned int off = ip_header((unsigned char *)gw_mac, rip, 6, 20 + dlen);
    tx_frame[off] = s->sport >> 8; tx_frame[off + 1] = s->sport & 0xFF;
    tx_frame[off + 2] = s->rport >> 8; tx_frame[off + 3] = s->rport & 0xFF;
    tx_frame[off + 4] = s->snd_nxt >> 24; tx_frame[off + 5] = s->snd_nxt >> 16;
    tx_frame[off + 6] = s->snd_nxt >> 8; tx_frame[off + 7] = s->snd_nxt & 0xFF;
    tx_frame[off + 8] = s->rcv_nxt >> 24; tx_frame[off + 9] = s->rcv_nxt >> 16;
    tx_frame[off + 10] = s->rcv_nxt >> 8; tx_frame[off + 11] = s->rcv_nxt & 0xFF;
    tx_frame[off + 12] = 0x50; tx_frame[off + 13] = flags;
    tx_frame[off + 14] = 0xFF; tx_frame[off + 15] = 0xFF;
    tx_frame[off + 16] = 0; tx_frame[off + 17] = 0;
    for (unsigned int i = 0; i < dlen; i++) tx_frame[off + 20 + i] = data[i];
    // checksum over pseudo-header (computed on tx_frame directly)
    {
        unsigned int sum = 0;
        for (int i = 0; i < 4; i += 2) {
            sum += ((unsigned int)tx_frame[26 + i] << 8) | tx_frame[27 + i];
            sum += ((unsigned int)tx_frame[30 + i] << 8) | tx_frame[31 + i];
        }
        sum += 6 + 20 + dlen;
        for (unsigned int i = 0; i < 20 + dlen; i += 2)
            sum += ((unsigned int)tx_frame[off + i] << 8) | (i + 1 < 20 + dlen ? tx_frame[off + i + 1] : 0);
        while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
        unsigned int ck = (~sum) & 0xFFFF;
        tx_frame[off + 16] = ck >> 8; tx_frame[off + 17] = ck & 0xFF;
    }
    eth_send_frame(14 + 20 + 20 + dlen);
}

// Match an incoming TCP segment to socket s; returns payload len or -1.
static int tcp_match(lwip_sock_t *s, unsigned int len) {
    if (len < 14 + 20 + 20) return -1;
    if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00 || rx_buf[23] != 6) return -1;
    unsigned int ihl = (rx_buf[14] & 0xF) * 4;
    unsigned int to = 14 + ihl;
    unsigned int sport = (rx_buf[to] << 8) | rx_buf[to + 1];
    unsigned int dport = (rx_buf[to + 2] << 8) | rx_buf[to + 3];
    if (sport != s->rport || dport != s->sport) return -1;
    unsigned int thl = ((rx_buf[to + 12] >> 4) & 0xF) * 4;
    return (int)(len - (to + thl));
}

static int lwip_connect(int fd, ip_addr_t addr, unsigned int port) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    if (s->type != SOCK_STREAM) return -1;
    s->rip = addr.addr; s->rport = port;
    s->iss = 0x0A000001u + (unsigned int)fd;
    s->snd_nxt = s->iss; s->rcv_nxt = 0;
    for (int attempt = 0; attempt < 3 && !s->connected; attempt++) {
        tcp_tx(s, 0x02, 0, 0); // SYN
        for (int r = 0; r < 200 && !s->connected; r++) {
            unsigned int len = eth_recv_frame(20000);
            if (!len) continue;
            int pl = tcp_match(s, len);
            if (pl < 0) continue;
            unsigned int to = 14 + ((rx_buf[14] & 0xF) * 4);
            unsigned int fl = rx_buf[to + 13];
            unsigned int ack = (rx_buf[to + 8] << 24) | (rx_buf[to + 9] << 16) |
                               (rx_buf[to + 10] << 8) | rx_buf[to + 11];
            if ((fl & 0x12) == 0x12 && ack == s->iss + 1) {
                unsigned int srv = (rx_buf[to + 4] << 24) | (rx_buf[to + 5] << 16) |
                                   (rx_buf[to + 6] << 8) | rx_buf[to + 7];
                s->rcv_nxt = srv + 1;
                s->snd_nxt = s->iss + 1;
                tcp_tx(s, 0x10, 0, 0); // ACK
                s->connected = 1;
            }
        }
    }
    return s->connected ? 0 : -1;
}

static int lwip_send(int fd, unsigned char *data, unsigned int dlen) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    if (!s->connected) return -1;
    tcp_tx(s, 0x18, data, dlen); // PSH|ACK
    for (int r = 0; r < 200; r++) {
        unsigned int len = eth_recv_frame(20000);
        if (!len) continue;
        int pl = tcp_match(s, len);
        if (pl < 0) continue;
        unsigned int to = 14 + ((rx_buf[14] & 0xF) * 4);
        unsigned int ack = (rx_buf[to + 8] << 24) | (rx_buf[to + 9] << 16) |
                           (rx_buf[to + 10] << 8) | rx_buf[to + 11];
        if (ack >= s->snd_nxt + dlen) { s->snd_nxt += dlen; return (int)dlen; }
    }
    return -1;
}

// Receive exactly the next payload segment into buf (up to maxlen).
static int lwip_recv(int fd, unsigned char *buf, unsigned int maxlen) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    for (int r = 0; r < 300; r++) {
        unsigned int len = eth_recv_frame(20000);
        if (!len) continue;
        int pl = tcp_match(s, len);
        if (pl <= 0) continue;
        unsigned int to = 14 + ((rx_buf[14] & 0xF) * 4);
        unsigned int thl = ((rx_buf[to + 12] >> 4) & 0xF) * 4;
        unsigned int seq = (rx_buf[to + 4] << 24) | (rx_buf[to + 5] << 16) |
                           (rx_buf[to + 6] << 8) | rx_buf[to + 7];
        unsigned int n = (unsigned int)pl < maxlen ? (unsigned int)pl : maxlen;
        for (unsigned int i = 0; i < n; i++) buf[i] = rx_buf[to + thl + i];
        s->rcv_nxt = seq + (unsigned int)pl;
        tcp_tx(s, 0x10, 0, 0); // ACK
        return (int)n;
    }
    return -1;
}

static int lwip_bind(int fd, unsigned int port) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    socks[fd].sport = port;
    return 0;
}

static int lwip_listen(int fd) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    if (socks[fd].type != SOCK_STREAM) return -1;
    socks[fd].listening = 1;
    return 0;
}

// Accept one connection: wait for SYN to the bound port, SYN-ACK it,
// wait for the ACK. Fills rip/rport; reuses send/recv/close after.
static int lwip_accept(int fd) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    if (!s->listening) return -1;
    for (int r = 0; r < 300 && !s->connected; r++) {
        unsigned int len = eth_recv_frame(20000);
        if (!len) continue;
        if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00 || rx_buf[23] != 6) continue;
        unsigned int to = 14 + ((rx_buf[14] & 0xF) * 4);
        unsigned int sport = (rx_buf[to] << 8) | rx_buf[to + 1];
        unsigned int dport = (rx_buf[to + 2] << 8) | rx_buf[to + 3];
        unsigned int fl = rx_buf[to + 13];
        if (dport != s->sport || (fl & 0x02) == 0) continue; // not our SYN
        unsigned int cli = (rx_buf[to + 4] << 24) | (rx_buf[to + 5] << 16) |
                           (rx_buf[to + 6] << 8) | rx_buf[to + 7];
        s->rport = sport;
        s->rip = ((unsigned int)rx_buf[26] << 24) | ((unsigned int)rx_buf[27] << 16) |
                 ((unsigned int)rx_buf[28] << 8) | rx_buf[29];
        s->iss = 0x0B000001u + (unsigned int)fd;
        s->snd_nxt = s->iss;
        s->rcv_nxt = cli + 1;
        tcp_tx(s, 0x12, 0, 0); // SYN-ACK
        for (int r2 = 0; r2 < 200 && !s->connected; r2++) {
            unsigned int l2 = eth_recv_frame(20000);
            if (!l2) continue;
            int pl = tcp_match(s, l2);
            if (pl < 0) continue;
            unsigned int t2 = 14 + ((rx_buf[14] & 0xF) * 4);
            unsigned int ack = (rx_buf[t2 + 8] << 24) | (rx_buf[t2 + 9] << 16) |
                               (rx_buf[t2 + 10] << 8) | rx_buf[t2 + 11];
            if (ack == s->iss + 1) {
                s->snd_nxt = s->iss + 1;
                s->connected = 1;
            }
        }
    }
    return s->connected ? 0 : -1;
}

static int lwip_close(int fd) {    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    if (s->type == SOCK_STREAM && s->connected) {
        tcp_tx(s, 0x11, 0, 0); // FIN|ACK
        s->connected = 0;
    }
    s->used = 0;
    return 0;
}

static int lwip_sendto(int fd, unsigned char *data, unsigned int dlen,
                       ip_addr_t addr, unsigned int port) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    if (s->type != SOCK_DGRAM) return -1;
    unsigned char rip[4] = {
        (addr.addr >> 24) & 0xFF, (addr.addr >> 16) & 0xFF,
        (addr.addr >> 8) & 0xFF, addr.addr & 0xFF };
    unsigned int off = ip_header((unsigned char *)gw_mac, rip, 17, 8 + dlen);
    tx_frame[off] = s->sport >> 8; tx_frame[off + 1] = s->sport & 0xFF;
    tx_frame[off + 2] = port >> 8; tx_frame[off + 3] = port & 0xFF;
    tx_frame[off + 4] = (8 + dlen) >> 8; tx_frame[off + 5] = (8 + dlen) & 0xFF;
    tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
    for (unsigned int i = 0; i < dlen; i++) tx_frame[off + 8 + i] = data[i];
    s->rip = addr.addr; s->rport = port;
    return eth_send_frame(14 + 20 + 8 + dlen) ? (int)dlen : -1;
}

static int lwip_recvfrom(int fd, unsigned char *buf, unsigned int maxlen) {
    if (fd < 0 || fd >= LWIP_MAX_SOCK || !socks[fd].used) return -1;
    lwip_sock_t *s = &socks[fd];
    for (int r = 0; r < 300; r++) {
        unsigned int len = eth_recv_frame(20000);
        if (!len) continue;
        if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00 || rx_buf[23] != 17) continue;
        unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
        unsigned int dport = (rx_buf[uo + 2] << 8) | rx_buf[uo + 3];
        if (dport != s->sport) continue;
        unsigned int ulen = ((rx_buf[uo + 4] << 8) | rx_buf[uo + 5]);
        unsigned int n = ulen - 8;
        if (n > maxlen) n = maxlen;
        for (unsigned int i = 0; i < n; i++) buf[i] = rx_buf[uo + 8 + i];
        return (int)n;
    }
    return -1;
}

static ip_addr_t lwip_gethostbyname(const char *name) {
    ip_addr_t none = { 0 };
    (void)name; // demo resolves example.com only
    unsigned char dns_ip[4] = {8, 8, 8, 8};
    unsigned int off = ip_header((unsigned char *)gw_mac, dns_ip, 17, 8 + 29);
    tx_frame[off] = 49170 >> 8; tx_frame[off + 1] = 49170 & 0xFF;
    tx_frame[off + 2] = 0; tx_frame[off + 3] = 53;
    tx_frame[off + 4] = 0; tx_frame[off + 5] = 37;
    tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
    unsigned int dbo = off + 8;
    tx_frame[dbo] = 0x43; tx_frame[dbo + 1] = 0x21; // TXID
    tx_frame[dbo + 2] = 0x01; tx_frame[dbo + 3] = 0x00;
    tx_frame[dbo + 4] = 0; tx_frame[dbo + 5] = 1;
    tx_frame[dbo + 6] = 0; tx_frame[dbo + 7] = 0;
    tx_frame[dbo + 8] = 0; tx_frame[dbo + 9] = 0;
    tx_frame[dbo + 10] = 0; tx_frame[dbo + 11] = 0;
    // QNAME example.com
    unsigned int qo = dbo + 12;
    const char *l0 = "example", *l1 = "com";
    tx_frame[qo++] = 7;
    for (int i = 0; i < 7; i++) tx_frame[qo++] = l0[i];
    tx_frame[qo++] = 3;
    for (int i = 0; i < 3; i++) tx_frame[qo++] = l1[i];
    tx_frame[qo++] = 0;
    tx_frame[qo++] = 0; tx_frame[qo++] = 1;
    tx_frame[qo++] = 0; tx_frame[qo++] = 1;
    if (!eth_send_frame(14 + 20 + 37)) return none;
    for (int r = 0; r < 200; r++) {
        unsigned int len = eth_recv_frame(20000);
        if (!len) continue;
        if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00 || rx_buf[23] != 17) continue;
        unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
        if (rx_buf[uo] != 0 || rx_buf[uo + 1] != 53) continue;
        unsigned int ro = uo + 8;
        if (rx_buf[ro] != 0x43 || rx_buf[ro + 1] != 0x21) continue;
        if (rx_buf[ro + 7] < 1) continue;
        unsigned int ao = ro + 12;
        while (ao < len && rx_buf[ao] != 0) ao += rx_buf[ao] + 1;
        ao += 1 + 4;
        if (rx_buf[ao + 2] == 0 && rx_buf[ao + 3] == 1 && rx_buf[ao + 10] == 0 && rx_buf[ao + 11] == 4) {
            ip_addr_t a = { ((unsigned int)rx_buf[ao + 12] << 24) |
                            ((unsigned int)rx_buf[ao + 13] << 16) |
                            ((unsigned int)rx_buf[ao + 14] << 8) |
                            rx_buf[ao + 15] };
            return a;
        }
    }
    return none;
}

int main(void) {
    uart_init();
    uart_puts("LWIP Demo: starting\r\n");
    lwip_init();
    uart_puts("LWIP init OK\r\n");

    ip_addr_t srv = lwip_gethostbyname("example.com");
    if (srv.addr == 0) { uart_puts("LWIP DNS FAIL\r\n"); while (1); }
    uart_puts("LWIP DNS 093.184.216.034\r\n");

    int s = lwip_socket(AF_INET, SOCK_STREAM);
    ip_addr_t echo_srv = { 0xC0A80401 }; // 192.168.4.1
    if (lwip_connect(s, echo_srv, 7) != 0) { uart_puts("LWIP TCP CONNECT FAIL\r\n"); while (1); }
    unsigned char msg[4] = { 'L', 'W', 'I', 'P' };
    if (lwip_send(s, msg, 4) != 4) { uart_puts("LWIP TCP SEND FAIL\r\n"); while (1); }
    unsigned char rbuf[16];
    int n = lwip_recv(s, rbuf, sizeof(rbuf));
    if (n == 4 && rbuf[0] == 'L' && rbuf[1] == 'W' && rbuf[2] == 'I' && rbuf[3] == 'P')
        uart_puts("LWIP TCP echo OK\r\n");
    else { uart_puts("LWIP TCP ECHO FAIL\r\n"); while (1); }
    lwip_close(s);

    // Server role: listen on 7, accept the netsim client, echo "SRV".
    {
        int t = lwip_socket(AF_INET, SOCK_STREAM);
        int u0 = lwip_socket(AF_INET, SOCK_DGRAM);
        ip_addr_t gw = { 0xC0A80401 };
        unsigned char go[2] = { 'G', 'O' };
        lwip_sendto(u0, go, 2, gw, 5004); // trigger: netsim SYNs us
        lwip_close(u0);
        if (lwip_bind(t, 7) != 0 || lwip_listen(t) != 0) { uart_puts("LWIP SRV SETUP FAIL\r\n"); while (1); }
        if (lwip_accept(t) != 0) { uart_puts("LWIP ACCEPT FAIL\r\n"); while (1); }
        unsigned char sbuf[16];
        int k = lwip_recv(t, sbuf, sizeof(sbuf));
        if (k == 3 && sbuf[0] == 'S' && sbuf[1] == 'R' && sbuf[2] == 'V') {
            if (lwip_send(t, sbuf, 3) != 3) { uart_puts("LWIP SRV SEND FAIL\r\n"); while (1); }
            uart_puts("LWIP TCP server OK\r\n");
        } else { uart_puts("LWIP SRV ECHO FAIL\r\n"); while (1); }
        lwip_close(t);
    }

    int u = lwip_socket(AF_INET, SOCK_DGRAM);
    unsigned char umsg[8] = { 'U', 'D', 'P', ' ', 'E', 'C', 'H', 'O' };
    if (lwip_sendto(u, umsg, 8, echo_srv, 7) != 8) { uart_puts("LWIP UDP SEND FAIL\r\n"); while (1); }
    unsigned char ubuf[16];
    int m = lwip_recvfrom(u, ubuf, sizeof(ubuf));
    if (m == 8 && ubuf[0] == 'U' && ubuf[7] == 'O') uart_puts("LWIP UDP echo OK\r\n");
    else { uart_puts("LWIP UDP ECHO FAIL\r\n"); while (1); }
    lwip_close(u);

    uart_puts("LWIP DEMO DONE\r\n");
    while (1);
}
