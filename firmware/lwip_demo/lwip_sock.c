// lwip_sock.c — BSD sockets over lwIP raw PCBs (NO_SYS, pumploop).
// Each blocking call pumps netif input + sys timeouts while waiting.
// States are driven by raw callbacks setting per-socket flags.
#include "lwip_sock.h"
#include "lwip/tcp.h"
#include "lwip/udp.h"
#include "lwip/dns.h"
#include "lwip/timeouts.h"
#include "lwip/sys.h"

#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define AF_INET 2

typedef struct {
    int used, type;
    unsigned int sport, rip, rport;
    struct tcp_pcb *tpcb;
    struct tcp_pcb *listen;
    struct tcp_pcb *pending; // accepted, awaiting accept()
    struct udp_pcb *upcb;
    // TCP RX state
    unsigned char tcpbuf[512];
    unsigned int tcp_n;
    int tcp_eof, tcp_err;
    // UDP RX ring
    unsigned char dgram[LWIP_SOCK_DGRAM_Q][LWIP_SOCK_DGRAM_MAX];
    unsigned int dgram_len[LWIP_SOCK_DGRAM_Q];
    unsigned int dgram_rip[LWIP_SOCK_DGRAM_Q];
    unsigned int dgram_rport[LWIP_SOCK_DGRAM_Q];
    unsigned int dgram_head, dgram_count;
    int connected;
} sock_t;

static sock_t socks[LWIP_SOCK_MAX];
static unsigned int next_sport = 49160;
static struct netif *sock_netif = 0;

int netif_f4_poll(struct netif *netif);

static void pump(void) {
    if (sock_netif) netif_f4_poll(sock_netif);
    sys_check_timeouts();
}

static int valid(int s) {
    return s >= 0 && s < LWIP_SOCK_MAX && socks[s].used;
}

// API boundary uses human-readable order (0xC0A80401 = 192.168.4.1);
// lwIP stores network byte order. Swap both ways.
static unsigned int h2n(unsigned int h) {
    return ((h & 0xFF) << 24) | (((h >> 8) & 0xFF) << 16) |
           (((h >> 16) & 0xFF) << 8) | ((h >> 24) & 0xFF);
}
#define n2h h2n

int lwip_sock_init(struct netif *netif) {
    sock_netif = netif;
    for (int i = 0; i < LWIP_SOCK_MAX; i++) {
        socks[i].used = 0;
    }
    return ERR_OK;
}

int lwip_socket(int domain, int type) {
    if (domain != AF_INET) return ERR_VAL;
    if (type != SOCK_STREAM && type != SOCK_DGRAM) return ERR_VAL;
    for (int i = 0; i < LWIP_SOCK_MAX; i++) {
        if (!socks[i].used) {
            sock_t *s = &socks[i];
            s->used = 1; s->type = type;
            s->sport = next_sport++;
            s->tpcb = 0; s->listen = 0; s->pending = 0; s->upcb = 0;
            s->tcp_n = 0; s->tcp_eof = 0; s->tcp_err = 0;
            s->dgram_head = 0; s->dgram_count = 0;
            s->connected = 0;
            return i;
        }
    }
    return ERR_MEM;
}

// ---- TCP callbacks (raw API -> socket flags) ----
static err_t on_tcp_connected(void *arg, struct tcp_pcb *tpcb, err_t err) {
    int fd = (int)(mem_ptr_t)arg;
    (void)tpcb;
    if (!valid(fd)) return ERR_OK;
    if (err != ERR_OK) socks[fd].tcp_err = 1;
    else socks[fd].connected = 1;
    return ERR_OK;
}

static err_t on_tcp_recv(void *arg, struct tcp_pcb *tpcb, struct pbuf *p, err_t err) {
    int fd = (int)(mem_ptr_t)arg;
    if (!valid(fd)) { if (p) pbuf_free(p); return ERR_OK; }
    sock_t *s = &socks[fd];
    if (!p) { s->tcp_eof = 1; return ERR_OK; } // FIN
    if (err != ERR_OK) { s->tcp_err = 1; pbuf_free(p); return ERR_OK; }
    {
        unsigned int n = p->tot_len;
        if (n > sizeof(s->tcpbuf) - s->tcp_n) n = sizeof(s->tcpbuf) - s->tcp_n;
        pbuf_copy_partial(p, s->tcpbuf + s->tcp_n, n, 0);
        s->tcp_n += n;
    }
    tcp_recved(tpcb, p->tot_len);
    pbuf_free(p);
    return ERR_OK;
}

static void on_tcp_err(void *arg, err_t err) {
    int fd = (int)(mem_ptr_t)arg;
    (void)err;
    if (!valid(fd)) return;
    socks[fd].tcp_err = 1;
    socks[fd].tpcb = 0; // pcb already freed by lwIP
}

static err_t on_tcp_accept(void *arg, struct tcp_pcb *newpcb, err_t err) {
    int fd = (int)(mem_ptr_t)arg;
    if (!valid(fd) || err != ERR_OK) return ERR_ABRT;
    sock_t *s = &socks[fd];
    if (s->pending) return ERR_ABRT; // one pending max (demo scale)
    s->pending = newpcb;
    tcp_arg(newpcb, arg);
    tcp_recv(newpcb, on_tcp_recv);
    tcp_err(newpcb, on_tcp_err);
    return ERR_OK;
}

static void udp_in(void *arg, struct udp_pcb *pcb, struct pbuf *p,
                   const ip_addr_t *addr, u16_t port) {
    int fd = (int)(mem_ptr_t)arg;
    (void)pcb;
    if (!valid(fd)) { if (p) pbuf_free(p); return; }
    sock_t *s = &socks[fd];
    if (p && s->dgram_count < LWIP_SOCK_DGRAM_Q && p->tot_len <= LWIP_SOCK_DGRAM_MAX) {
        unsigned int i = (s->dgram_head + s->dgram_count) % LWIP_SOCK_DGRAM_Q;
        pbuf_copy_partial(p, s->dgram[i], p->tot_len, 0);
        s->dgram_len[i] = p->tot_len;
        s->dgram_rip[i] = n2h(addr->addr);
        s->dgram_rport[i] = port;
        s->dgram_count++;
    }
    if (p) pbuf_free(p);
}

// Wait until cond(fd) or timeout_ms (sys_now-based). Returns 0 on ready.
static int wait_until(int fd, int (*cond)(int), unsigned int timeout_ms) {
    unsigned int t0 = sys_now();
    for (;;) {
        pump();
        if (cond(fd)) return 0;
        if ((sys_now() - t0) > timeout_ms) return ERR_TIMEOUT;
    }
}
static int c_conn(int fd) { return socks[fd].connected || socks[fd].tcp_err; }
static int c_tdata(int fd) { return socks[fd].tcp_n > 0 || socks[fd].tcp_eof || socks[fd].tcp_err; }
static int c_udata(int fd) { return socks[fd].dgram_count > 0; }
static int c_acc(int fd) { return socks[fd].pending != 0; }

int lwip_bind(int s, unsigned int port) {
    if (!valid(s)) return ERR_VAL;
    socks[s].sport = port;
    return ERR_OK;
}

int lwip_listen(int s) {
    if (!valid(s) || socks[s].type != SOCK_STREAM) return ERR_VAL;
    {
        struct tcp_pcb *l = tcp_new();
        if (!l) return ERR_MEM;
        if (tcp_bind(l, IP_ADDR_ANY, (u16_t)socks[s].sport) != ERR_OK) {
            tcp_close(l);
            return ERR_USE;
        }
        l = tcp_listen_with_backlog(l, 1);
        if (!l) return ERR_MEM;
        socks[s].listen = l;
        tcp_arg(l, (void *)(mem_ptr_t)s);
        tcp_accept(l, on_tcp_accept);
    }
    return ERR_OK;
}

int lwip_accept(int s, unsigned int *rip, unsigned int *rport) {
    if (!valid(s) || !socks[s].listen) return ERR_VAL;
    if (wait_until(s, c_acc, 30000)) return ERR_TIMEOUT;
    {
        struct tcp_pcb *c = socks[s].pending;
        socks[s].pending = 0;
        // Hand the connection to a fresh fd (listening fd stays open).
        int fd = -1;
        for (int i = 0; i < LWIP_SOCK_MAX; i++) {
            if (!socks[i].used) { fd = i; break; }
        }
        if (fd < 0) { tcp_abort(c); return ERR_MEM; }
        sock_t *n = &socks[fd];
        n->used = 1; n->type = SOCK_STREAM;
        n->sport = socks[s].sport;
        {
            ip_addr_t ra;
            u16_t rp;
            // Remote endpoint is known once data/ACK flows; fill best-effort.
            ra.addr = 0; rp = 0;
            (void)ra; (void)rp;
        }
        n->tpcb = c;
        n->tcp_n = 0; n->tcp_eof = 0; n->tcp_err = 0;
        n->connected = 1;
        n->rip = n2h(c->remote_ip.addr);
        n->rport = c->remote_port;
        tcp_arg(c, (void *)(mem_ptr_t)fd);
        if (rip) *rip = n->rip;
        if (rport) *rport = n->rport;
        return fd;
    }
}

int lwip_connect(int s, unsigned int rip, unsigned int port) {
    ip_addr_t addr;
    if (!valid(s) || socks[s].type != SOCK_STREAM) return ERR_VAL;
    addr.addr = h2n(rip);
    {
        struct tcp_pcb *t = tcp_new();
        if (!t) return ERR_MEM;
        socks[s].tpcb = t;
        socks[s].rip = rip;
        socks[s].rport = port;
        tcp_arg(t, (void *)(mem_ptr_t)s);
        tcp_recv(t, on_tcp_recv);
        tcp_err(t, on_tcp_err);
        if (tcp_connect(t, &addr, (u16_t)port, on_tcp_connected) != ERR_OK) {
            tcp_close(t);
            socks[s].tpcb = 0;
            return ERR_CONN;
        }
    }
    if (wait_until(s, c_conn, 30000)) return ERR_TIMEOUT;
    return socks[s].connected ? ERR_OK : ERR_CONN;
}

int lwip_send(int s, const unsigned char *data, unsigned int len) {
    if (!valid(s) || socks[s].type != SOCK_STREAM || !socks[s].tpcb) return ERR_CONN;
    {
        err_t e = tcp_write(socks[s].tpcb, data, (u16_t)len, TCP_WRITE_FLAG_COPY);
        if (e != ERR_OK) return e;
        tcp_output(socks[s].tpcb);
    }
    return (int)len;
}

int lwip_recv(int s, unsigned char *buf, unsigned int maxlen) {
    if (!valid(s) || socks[s].type != SOCK_STREAM || !socks[s].tpcb) return ERR_CONN;
    if (wait_until(s, c_tdata, 30000)) return ERR_TIMEOUT;
    {
        sock_t *sc = &socks[s];
        if (sc->tcp_err) return ERR_CONN;
        if (sc->tcp_n == 0) return 0; // EOF
        unsigned int n = sc->tcp_n < maxlen ? sc->tcp_n : maxlen;
        for (unsigned int i = 0; i < n; i++) buf[i] = sc->tcpbuf[i];
        for (unsigned int i = n; i < sc->tcp_n; i++) sc->tcpbuf[i - n] = sc->tcpbuf[i];
        sc->tcp_n -= n;
        return (int)n;
    }
}

int lwip_sendto(int s, const unsigned char *data, unsigned int len,
                unsigned int rip, unsigned int port) {
    ip_addr_t addr;
    if (!valid(s) || socks[s].type != SOCK_DGRAM) return ERR_VAL;
    addr.addr = h2n(rip);
    if (!socks[s].upcb) {
        struct udp_pcb *u = udp_new();
        if (!u) return ERR_MEM;
        if (udp_bind(u, IP_ADDR_ANY, (u16_t)socks[s].sport) != ERR_OK) {
            udp_remove(u);
            return ERR_USE;
        }
        udp_recv(u, udp_in, (void *)(mem_ptr_t)s);
        socks[s].upcb = u;
    }
    {
        struct pbuf *p = pbuf_alloc(PBUF_TRANSPORT, (u16_t)len, PBUF_RAM);
        err_t e;
        if (!p) return ERR_MEM;
        pbuf_take(p, data, len);
        e = udp_sendto(socks[s].upcb, p, &addr, (u16_t)port);
        pbuf_free(p);
        if (e != ERR_OK) return e;
    }
    socks[s].rip = rip;
    socks[s].rport = port;
    return (int)len;
}

int lwip_recvfrom(int s, unsigned char *buf, unsigned int maxlen,
                  unsigned int *rip, unsigned int *rport) {
    if (!valid(s) || socks[s].type != SOCK_DGRAM) return ERR_VAL;
    // Ensure the pcb exists even before the first send (for select/recv).
    if (!socks[s].upcb) {
        struct udp_pcb *u = udp_new();
        if (!u) return ERR_MEM;
        if (udp_bind(u, IP_ADDR_ANY, (u16_t)socks[s].sport) != ERR_OK) {
            udp_remove(u);
            return ERR_USE;
        }
        udp_recv(u, udp_in, (void *)(mem_ptr_t)s);
        socks[s].upcb = u;
    }
    if (wait_until(s, c_udata, 30000)) return ERR_TIMEOUT;
    {
        sock_t *sc = &socks[s];
        unsigned int i = sc->dgram_head;
        unsigned int n = sc->dgram_len[i] < maxlen ? sc->dgram_len[i] : maxlen;
        for (unsigned int k = 0; k < n; k++) buf[k] = sc->dgram[i][k];
        if (rip) *rip = sc->dgram_rip[i];
        if (rport) *rport = sc->dgram_rport[i];
        sc->dgram_head = (sc->dgram_head + 1) % LWIP_SOCK_DGRAM_Q;
        sc->dgram_count--;
        return (int)n;
    }
}

int lwip_closesocket(int s) {
    if (!valid(s)) return ERR_VAL;
    {
        sock_t *sc = &socks[s];
        if (sc->tpcb) { tcp_close(sc->tpcb); sc->tpcb = 0; }
        if (sc->listen) { tcp_close(sc->listen); sc->listen = 0; }
        if (sc->pending) { tcp_abort(sc->pending); sc->pending = 0; }
        if (sc->upcb) { udp_remove(sc->upcb); sc->upcb = 0; }
        sc->used = 0;
    }
    return ERR_OK;
}

int lwip_sock_readable(int s) {
    sock_t *sc;
    if (!valid(s)) return 0;
    sc = &socks[s];
    if (sc->type == SOCK_STREAM) {
        if (sc->listen) return sc->pending != 0;
        return sc->tcp_n > 0 || sc->tcp_eof || sc->tcp_err;
    }
    return sc->dgram_count > 0;
}

int lwip_select(unsigned int readfds, unsigned int timeout_ms) {
    unsigned int t0 = sys_now();
    for (;;) {
        int n = 0;
        pump();
        for (int i = 0; i < LWIP_SOCK_MAX; i++) {
            if ((readfds >> i) & 1) n += lwip_sock_readable(i) ? 1 : 0;
        }
        if (n > 0) return n;
        if ((sys_now() - t0) > timeout_ms) return 0;
    }
}

static volatile int dns_hit = 0;
static ip_addr_t dns_ip;
static void dns_found_cb(const char *name, const ip_addr_t *ipaddr, void *arg) {
    (void)name; (void)arg;
    if (ipaddr) {
        ip_addr_copy(dns_ip, *ipaddr);
        dns_hit = 1;
    } else {
        dns_hit = -1;
    }
}

int lwip_gethostbyname(const char *name, unsigned int *out_ip) {
    ip_addr_t a;
    err_t e;
    dns_hit = 0;
    e = dns_gethostbyname(name, &a, dns_found_cb, NULL);
    if (e == ERR_OK) {
        *out_ip = n2h(a.addr);
        return ERR_OK;
    }
    if (e != ERR_INPROGRESS) return e;
    {
        unsigned int t0 = sys_now();
        while (!dns_hit) {
            pump();
            if ((sys_now() - t0) > 30000) return ERR_TIMEOUT;
        }
    }
    if (dns_hit < 0) return ERR_VAL;
    *out_ip = n2h(dns_ip.addr);
    return ERR_OK;
}
