// lwip_demo: REAL LwIP 2.2.1 (vendored in lwip/, NO_SYS=1 raw API) on the
// bare-metal F4 Ethernet driver, driven through the BSD-style socket
// layer (lwip_sock.c: socket/bind/listen/accept/connect/send/recv/
// sendto/recvfrom/close/select + real err_t codes). No clean-room stack:
// DHCP, DNS, ARP, TCP and UDP are lwIP's own; netsim (or the real
// gateway) is the peer.
// Demo: DHCP bind -> resolve example.com -> TCP echo client :7 ->
// TCP echo server :7 (select-gated, netsim connects) -> UDP echo ->
// misuse probes -> DONE.
#include "defs.h"

#include "lwip/opt.h"
#include "lwip/init.h"
#include "lwip/sys.h"
#include "lwip/netif.h"
#include "netif/ethernet.h"
#include "lwip/dhcp.h"
#include "lwip/dns.h"
#include "lwip/tcp.h"
#include "lwip/udp.h"
#include "lwip/timeouts.h"
#include "lwip/ip4_addr.h"
#include "lwip/err.h"
#include "lwip_sock.h"

#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2

const unsigned char f4_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};

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

// Raw TX: queue the frame, wait for TS (bounded). Returns 1 on completion.
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

static void uart_ip_bytes(unsigned int ip) {    uart_putchar('0' + ((ip >> 24) & 0xFF) / 100);
    uart_putchar('0' + (((ip >> 24) & 0xFF) % 100) / 10);
    uart_putchar('0' + (((ip >> 24) & 0xFF) % 10));
    uart_putchar('.');
    uart_putchar('0' + ((ip >> 16) & 0xFF) / 100);
    uart_putchar('0' + (((ip >> 16) & 0xFF) % 100) / 10);
    uart_putchar('0' + (((ip >> 16) & 0xFF) % 10));
    uart_putchar('.');
    uart_putchar('0' + ((ip >> 8) & 0xFF) / 100);
    uart_putchar('0' + (((ip >> 8) & 0xFF) % 100) / 10);
    uart_putchar('0' + (((ip >> 8) & 0xFF) % 10));
    uart_putchar('.');
    uart_putchar('0' + (ip & 0xFF) / 100);
    uart_putchar('0' + ((ip & 0xFF) % 100) / 10);
    uart_putchar('0' + ((ip & 0xFF) % 10));
}

static int pump_net(int budget) {
    for (int i = 0; i < budget; i++) {
        netif_f4_poll(&f4_ni);
        sys_check_timeouts();
    }
    return 0;
}

// Network-order u32 (lwIP internals) -> human order (first octet high).
static unsigned int ip_n2h(unsigned int n) {
    return ((n & 0xFF) << 24) | (((n >> 8) & 0xFF) << 16) |
           (((n >> 16) & 0xFF) << 8) | ((n >> 24) & 0xFF);
}

int main(void) {
    uart_init();
    uart_puts("LWIP Demo: starting\r\n");
    sys_init(); // DWT on (NO_SYS never calls this; sys_now needs it)
    eth_hw_init();
    lwip_init();
    lwip_sock_init(&f4_ni);
    {
        ip4_addr_t ip, mask, gw;
        IP4_ADDR(&ip, 0, 0, 0, 0);
        IP4_ADDR(&mask, 0, 0, 0, 0);
        IP4_ADDR(&gw, 0, 0, 0, 0);
        netif_add(&f4_ni, &ip, &mask, &gw, NULL, netif_f4_init, ethernet_input);
        netif_set_hostname(&f4_ni, "stm32f4");
        netif_set_default(&f4_ni);
        netif_set_link_up(&f4_ni);
        netif_set_up(&f4_ni);
    }
    uart_puts("LWIP init OK\r\n");
    dhcp_start(&f4_ni);

    // DHCP bind (netsim serves Offer/Ack; the gateway serves for real).
    {
        int guard = 0;
        while (ip4_addr_isany_val(*netif_ip4_addr(&f4_ni))) {
            pump_net(1000);
            if (++guard > 400) { uart_puts("LWIP DHCP FAIL\r\n"); while (1); }
        }
        uart_puts("LWIP bound ");
        uart_ip_bytes(ip_n2h(netif_ip4_addr(&f4_ni)->addr));
        uart_puts("\r\n");
    }

    // DNS resolve (canned A 93.184.216.34 by netsim).
    {
        unsigned int ip = 0;
        if (lwip_gethostbyname("example.com", &ip) != ERR_OK) {
            uart_puts("LWIP DNS FAIL\r\n"); while (1);
        }
        if (((ip >> 24) & 0xFF) != 93 || ((ip >> 16) & 0xFF) != 184 ||
            ((ip >> 8) & 0xFF) != 216 || (ip & 0xFF) != 34) {
            uart_puts("LWIP DNS FAIL\r\n"); while (1);
        }
        uart_puts("LWIP DNS 093.184.216.034\r\n");
    }

    // TCP echo client to the test peer :7.
    {
        int s = lwip_socket(AF_INET, SOCK_STREAM);
        unsigned char rbuf[16];
        int n;
        if (s < 0 || lwip_connect(s, 0xC0A80401, 7) != ERR_OK) {
            uart_puts("LWIP TCP CONNECT FAIL\r\n"); while (1);
        }
        {
            unsigned char msg[4] = { 'L', 'W', 'I', 'P' };
            if (lwip_send(s, msg, 4) != 4) { uart_puts("LWIP TCP SEND FAIL\r\n"); while (1); }
        }
        n = lwip_recv(s, rbuf, sizeof(rbuf));
        if (n == 4 && rbuf[0] == 'L' && rbuf[1] == 'W' && rbuf[2] == 'I' && rbuf[3] == 'P')
            uart_puts("LWIP TCP echo OK\r\n");
        else { uart_puts("LWIP TCP ECHO FAIL\r\n"); while (1); }
        lwip_closesocket(s);
    }

    // TCP echo server on :7, gated by select (netsim connects on trigger).
    {
        int t = lwip_socket(AF_INET, SOCK_STREAM);
        int u0 = lwip_socket(AF_INET, SOCK_DGRAM);
        unsigned char go[2] = { 'G', 'O' };
        if (t < 0 || u0 < 0) { uart_puts("LWIP SRV SETUP FAIL\r\n"); while (1); }
        lwip_sendto(u0, go, 2, 0xC0A80401, 5004); // trigger: netsim SYNs us
        lwip_closesocket(u0);
        if (lwip_bind(t, 7) != ERR_OK || lwip_listen(t) != ERR_OK) {
            uart_puts("LWIP SRV SETUP FAIL\r\n"); while (1);
        }
        // select() must report the listen fd readable once the SYN lands.
        {
            int r = lwip_select(1u << t, 30000);
            if (r != 1) { uart_puts("LWIP SELECT FAIL\r\n"); while (1); }
            uart_puts("LWIP SELECT OK\r\n");
        }
        {
            unsigned int rip = 0, rport = 0;
            int c = lwip_accept(t, &rip, &rport);
            unsigned char sbuf[16];
            int k;
            if (c < 0) { uart_puts("LWIP ACCEPT FAIL\r\n"); while (1); }
            k = lwip_recv(c, sbuf, sizeof(sbuf));
            if (k == 3 && sbuf[0] == 'S' && sbuf[1] == 'R' && sbuf[2] == 'V') {
                if (lwip_send(c, sbuf, 3) != 3) { uart_puts("LWIP SRV SEND FAIL\r\n"); while (1); }
                uart_puts("LWIP TCP server OK\r\n");
            } else { uart_puts("LWIP SRV ECHO FAIL\r\n"); while (1); }
            lwip_closesocket(c);
        }
        lwip_closesocket(t);
    }

    // UDP echo to the test peer :7.
    {
        int u = lwip_socket(AF_INET, SOCK_DGRAM);
        unsigned char umsg[8] = { 'U', 'D', 'P', ' ', 'E', 'C', 'H', 'O' };
        unsigned char ubuf[16];
        int m;
        if (u < 0) { uart_puts("LWIP UDP SETUP FAIL\r\n"); while (1); }
        if (lwip_sendto(u, umsg, 8, 0xC0A80401, 7) != 8) {
            uart_puts("LWIP UDP SEND FAIL\r\n"); while (1);
        }
        m = lwip_recvfrom(u, ubuf, sizeof(ubuf), 0, 0);
        if (m == 8 && ubuf[0] == 'U' && ubuf[7] == 'O') uart_puts("LWIP UDP echo OK\r\n");
        else { uart_puts("LWIP UDP ECHO FAIL\r\n"); while (1); }
        lwip_closesocket(u);
    }

    // Misuse probes: real lwIP err_t codes, no crash.
    {
        int ok = 1;
        if (lwip_socket(99, SOCK_STREAM) != ERR_VAL) ok = 0;
        if (lwip_closesocket(9) != ERR_VAL) ok = 0;
        {
            int s = lwip_socket(AF_INET, SOCK_STREAM);
            unsigned char b[4];
            if (s < 0 || lwip_send(s, b, 4) != ERR_CONN) ok = 0;
            if (lwip_recv(s, b, 4) != ERR_CONN) ok = 0;
            lwip_closesocket(s);
        }
        if (ok) uart_puts("LWIP ERR OK\r\n");
        else uart_puts("LWIP ERR FAIL\r\n");
    }

    uart_puts("LWIP DEMO DONE\r\n");
    while (1);
}
