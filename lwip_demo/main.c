// lwip_demo: REAL LwIP 2.2.1 (vendored in lwip/, NO_SYS=1 raw API) on the
// bare-metal F4 Ethernet driver. No clean-room stack: DHCP, DNS, ARP,
// TCP and UDP are lwIP's own; netsim (or the real gateway) is the peer.
// Demo: DHCP bind -> resolve example.com -> TCP echo client :7 ->
// TCP echo server :7 (netsim connects) -> UDP echo -> DONE.
#include "defs.h"

#include "lwip/opt.h"
#include "lwip/init.h"
#include "lwip/sys.h"
#include "lwip/netif.h"
#include "lwip/etharp.h"
#include "netif/ethernet.h"
#include "lwip/dhcp.h"
#include "lwip/dns.h"
#include "lwip/tcp.h"
#include "lwip/udp.h"
#include "lwip/timeouts.h"
#include "lwip/ip4_addr.h"

const unsigned char f4_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};
static const unsigned char echo_srv[4] = {192, 168, 4, 1};

static volatile unsigned int tx_desc[2] __attribute__((aligned(8))) = { 0 };
static volatile int eth_done = 0;
static volatile unsigned int rx_desc[2] __attribute__((aligned(8))) = { 0 };
static volatile unsigned char rx_buf[1536] __attribute__((aligned(4)));
static volatile int rx_flag = 0;
static unsigned int poll_div = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { eth_done = 1; DMASR = (1 << 0) | (1 << 16) | (1 << 14); }
    if (sr & (1 << 6)) { rx_flag = 1; DMASR = (1 << 6) | (1 << 16) | (1 << 14); }
}

void ETH_WKUP_IRQHandler(void) { /* unused here; vector slot */ }

static unsigned char tx_frame[1536];

static void eth_hw_init(void) {
    RCC_AHB1ENR |= (1 << 25); // ETH clock
    DMABMR |= 1;
    wait_ms(10);
    MACCR = (1 << 2) | (1 << 3) | (1 << 11) | (1 << 14); // RE+TE+DM+FES
    MACA0HR = 0x00000200 | (1 << 31); // AE; MAC 02:00:00:00:00:01 MSB-first
    MACA0LR = 0x00000001;
    DMAOMR = (1 << 13) | (1 << 1); // ST + SR
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6); // NIE + TSE + RSE
    NVIC_ISER1 |= (1 << (61 - 32)); // IRQ 61
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;
}

int raw_send(const unsigned char *data, unsigned int len) {
    unsigned int n = len < 60 ? 60 : len;
    for (unsigned int i = 0; i < n; i++)
        tx_frame[i] = i < len ? data[i] : 0;
    tx_desc[0] = 0x80000000 | (n & 0x3FFF);
    tx_desc[1] = (unsigned int)&tx_frame[0];
    DMATDLAR = (unsigned int)&tx_desc[0];
    eth_done = 0;
    DMATPDR = 1;
    for (int i = 0; i < 2000000; i++) if (eth_done) return 1;
    return 0;
}

// Raw RX: consume one frame if flagged, else re-arm periodically.
unsigned int raw_try_recv(unsigned char *dst, unsigned int maxlen) {
    if ((poll_div++ & 0x3F) == 0) DMARPDR = 1;
    if (!rx_flag) return 0;
    rx_flag = 0;
    unsigned int len = (rx_desc[0] >> 16) & 0x3FFF;
    rx_desc[0] = 0x80000000 | 1536;
    DMARPDR = 1;
    if (len > 1536) len = 1536;
    if (len > maxlen) len = maxlen;
    for (unsigned int i = 0; i < len; i++) dst[i] = rx_buf[i];
    return len;
}

int netif_f4_poll(struct netif *netif);
err_t netif_f4_init(struct netif *netif);

static struct netif f4_ni;
static volatile int have_ip = 0, dns_done = 0, dns_ok = 0;
static volatile int tcp_echo_ok = 0, tcp_fail = 0;
static volatile int srv_echo_ok = 0;
static volatile int udp_echo_ok = 0;
static ip_addr_t srv_ip;

static void uart_ip(const unsigned char *ip) {
    uart_putchar('0' + ip[0] / 100); uart_putchar('0' + (ip[0] % 100) / 10); uart_putchar('0' + ip[0] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[1] / 100); uart_putchar('0' + (ip[1] % 100) / 10); uart_putchar('0' + ip[1] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[2] / 100); uart_putchar('0' + (ip[2] % 100) / 10); uart_putchar('0' + ip[2] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[3] / 100); uart_putchar('0' + (ip[3] % 100) / 10); uart_putchar('0' + ip[3] % 10);
}

static void uart_ip4(const ip4_addr_t *ip) {
    uart_ip((unsigned char *)&ip->addr);
}

static err_t tcp_srv_recv_cb(void *arg, struct tcp_pcb *tpcb, struct pbuf *p, err_t err);

static void dns_cb(const char *name, const ip_addr_t *ipaddr, void *arg) {
    (void)name; (void)arg;
    if (ipaddr) {
        ip_addr_copy(srv_ip, *ipaddr);
        dns_ok = 1;
    }
    dns_done = 1;
}

static err_t tcp_connected_cb(void *arg, struct tcp_pcb *tpcb, err_t err) {
    (void)arg;
    if (err != ERR_OK) { tcp_fail = 1; return ERR_OK; }
    tcp_write(tpcb, "LWIP", 4, TCP_WRITE_FLAG_COPY);
    tcp_output(tpcb);
    return ERR_OK;
}

static err_t tcp_recv_cb(void *arg, struct tcp_pcb *tpcb, struct pbuf *p, err_t err) {
    (void)arg;
    if (!p) { tcp_close(tpcb); return ERR_OK; } // FIN
    if (err == ERR_OK && p->tot_len == 4) {
        char b[4];
        pbuf_copy_partial(p, b, 4, 0);
        if (b[0] == 'L' && b[1] == 'W' && b[2] == 'I' && b[3] == 'P') {
            tcp_echo_ok = 1;
            tcp_recved(tpcb, p->tot_len);
            pbuf_free(p);
            tcp_close(tpcb);
            return ERR_OK;
        }
    }
    tcp_fail = 1;
    pbuf_free(p);
    tcp_close(tpcb);
    return ERR_OK;
}

static void tcp_err_cb(void *arg, err_t err) {
    (void)arg; (void)err;
    tcp_fail = 1;
}

static err_t tcp_accept_cb(void *arg, struct tcp_pcb *newpcb, err_t err) {
    (void)arg;
    if (err != ERR_OK) return ERR_ABRT;
    tcp_recv(newpcb, tcp_srv_recv_cb);
    tcp_err(newpcb, tcp_err_cb);
    return ERR_OK;
}

static err_t tcp_srv_recv_cb(void *arg, struct tcp_pcb *tpcb, struct pbuf *p, err_t err) {
    (void)arg;
    if (!p) { tcp_close(tpcb); return ERR_OK; }
    if (err == ERR_OK && p->tot_len == 3) {
        char b[3];
        pbuf_copy_partial(p, b, 3, 0);
        if (b[0] == 'S' && b[1] == 'R' && b[2] == 'V') {
            tcp_recved(tpcb, p->tot_len);
            pbuf_free(p);
            tcp_write(tpcb, "SRV", 3, TCP_WRITE_FLAG_COPY);
            tcp_output(tpcb);
            srv_echo_ok = 1;
            tcp_close(tpcb);
            return ERR_OK;
        }
    }
    pbuf_free(p);
    tcp_close(tpcb);
    return ERR_OK;
}

static void udp_recv_cb(void *arg, struct udp_pcb *pcb, struct pbuf *p,
                         const ip_addr_t *addr, u16_t port) {
    (void)arg; (void)pcb; (void)addr; (void)port;
    if (p && p->tot_len == 8) {
        char b[8];
        pbuf_copy_partial(p, b, 8, 0);
        if (b[0] == 'U' && b[7] == 'O') udp_echo_ok = 1;
    }
    if (p) pbuf_free(p);
}

// Pump stack + timers until cond() or budget (loop iterations) exhausts.
static int pump_until(int (*cond)(void), int budget) {
    for (int i = 0; i < budget && !cond(); i++) {
        netif_f4_poll(&f4_ni);
        sys_check_timeouts();
    }
    return cond() ? 0 : -1;
}
static int c_ip(void) { return have_ip; }
static int c_dns(void) { return dns_done; }
static int c_techo(void) { return tcp_echo_ok || tcp_fail; }
static int c_secho(void) { return srv_echo_ok; }
static int c_uecho(void) { return udp_echo_ok; }

int main(void) {
    uart_init();
    uart_puts("LWIP Demo: starting\r\n");
    sys_init(); // DWT on (NO_SYS never calls this; sys_now needs it)
    eth_hw_init();
    lwip_init();
    ip4_addr_t ip, mask, gw;
    IP4_ADDR(&ip, 0, 0, 0, 0);
    IP4_ADDR(&mask, 0, 0, 0, 0);
    IP4_ADDR(&gw, 0, 0, 0, 0);
    netif_add(&f4_ni, &ip, &mask, &gw, NULL, netif_f4_init, ethernet_input);
    netif_set_hostname(&f4_ni, "stm32f4");
    netif_set_default(&f4_ni);
    netif_set_link_up(&f4_ni);
    netif_set_up(&f4_ni);
    uart_puts("LWIP init OK\r\n");
    dhcp_start(&f4_ni);

    // DHCP bind (netsim serves Offer/Ack; the gateway serves for real).
    {
        int guard = 0;
        while (!have_ip && guard++ < 400000) {
            netif_f4_poll(&f4_ni);
            sys_check_timeouts();
            if (!ip4_addr_isany_val(*netif_ip4_addr(&f4_ni))) have_ip = 1;
        }
        if (!have_ip) { uart_puts("LWIP DHCP FAIL\r\n"); while (1); }
        uart_puts("LWIP bound ");
        uart_ip4(netif_ip4_addr(&f4_ni));
        uart_puts("\r\n");
    }

    // DNS resolve (canned A 93.184.216.34 by netsim).
    {
        err_t e = dns_gethostbyname("example.com", &srv_ip, dns_cb, NULL);
        if (e == ERR_OK) { dns_ok = 1; dns_done = 1; }
        else if (e != ERR_INPROGRESS) { uart_puts("LWIP DNS FAIL\r\n"); while (1); }
        if (pump_until(c_dns, 200000)) { uart_puts("LWIP DNS FAIL\r\n"); while (1); }
        if (!dns_ok) { uart_puts("LWIP DNS FAIL\r\n"); while (1); }
        unsigned char *a = (unsigned char *)&srv_ip.addr;
        if (!(a[0] == 93 && a[1] == 184 && a[2] == 216 && a[3] == 34)) {
            uart_puts("LWIP DNS FAIL\r\n"); while (1);
        }
        uart_puts("LWIP DNS 093.184.216.034\r\n");
    }

    // TCP echo client to the test peer :7 (netsim echoes any dst IP).
    {
        ip4_addr_t peer;
        IP4_ADDR(&peer, 192, 168, 4, 1);
        struct tcp_pcb *t = tcp_new();
        if (!t) { uart_puts("LWIP TCP PCB FAIL\r\n"); while (1); }
        tcp_recv(t, tcp_recv_cb);
        tcp_err(t, tcp_err_cb);
        if (tcp_connect(t, &peer, 7, tcp_connected_cb) != ERR_OK) {
            uart_puts("LWIP TCP CONNECT FAIL\r\n"); while (1);
        }
        if (pump_until(c_techo, 200000) || tcp_fail || !tcp_echo_ok) {
            uart_puts("LWIP TCP ECHO FAIL\r\n"); while (1);
        }
        uart_puts("LWIP TCP echo OK\r\n");
    }

    // TCP echo server on :7 (netsim connects as a client on trigger).
    {
        struct udp_pcb *trig = udp_new();
        ip4_addr_t gwip;
        IP4_ADDR(&gwip, 192, 168, 4, 1);
        struct pbuf *g = pbuf_alloc(PBUF_TRANSPORT, 2, PBUF_RAM);
        if (g) {
            ((char *)g->payload)[0] = 'G'; ((char *)g->payload)[1] = 'O';
            udp_sendto(trig, g, &gwip, 5004);
            pbuf_free(g);
        }
        udp_remove(trig);
        struct tcp_pcb *l = tcp_new();
        if (!l || tcp_bind(l, IP_ADDR_ANY, 7) != ERR_OK) {
            uart_puts("LWIP SRV SETUP FAIL\r\n"); while (1);
        }
        struct tcp_pcb *ll = tcp_listen_with_backlog(l, 1);
        if (!ll) { uart_puts("LWIP SRV SETUP FAIL\r\n"); while (1); }
        tcp_accept(ll, tcp_accept_cb);
        if (pump_until(c_secho, 200000) || !srv_echo_ok) {
            uart_puts("LWIP SRV ECHO FAIL\r\n"); while (1);
        }
        tcp_close(ll);
        uart_puts("LWIP TCP server OK\r\n");
    }

    // UDP echo to the test peer :7.
    {
        struct udp_pcb *u = udp_new();
        ip4_addr_t peer;
        IP4_ADDR(&peer, 192, 168, 4, 1);
        if (!u || udp_bind(u, IP_ADDR_ANY, 49170) != ERR_OK) {
            uart_puts("LWIP UDP SETUP FAIL\r\n"); while (1);
        }
        udp_recv(u, udp_recv_cb, NULL);
        struct pbuf *p = pbuf_alloc(PBUF_TRANSPORT, 8, PBUF_RAM);
        if (!p) { uart_puts("LWIP UDP SETUP FAIL\r\n"); while (1); }
        {
            char *d = (char *)p->payload;
            d[0]='U'; d[1]='D'; d[2]='P'; d[3]=' '; d[4]='E'; d[5]='C'; d[6]='H'; d[7]='O';
        }
        udp_sendto(u, p, &peer, 7);
        pbuf_free(p);
        if (pump_until(c_uecho, 200000) || !udp_echo_ok) {
            uart_puts("LWIP UDP ECHO FAIL\r\n"); while (1);
        }
        udp_remove(u);
        uart_puts("LWIP UDP echo OK\r\n");
    }

    uart_puts("LWIP DEMO DONE\r\n");
    while (1);
}
