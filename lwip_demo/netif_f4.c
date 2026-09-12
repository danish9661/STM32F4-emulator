// netif_f4.c — real LwIP netif glue over the raw F4 Ethernet driver.
// low_level_output copies a pbuf chain into tx_frame and transmits;
// netif_f4_poll drains pending RX frames into the stack. Single-threaded
// (NO_SYS=1): called from the demo's main loop, no locking.
#include "lwip/opt.h"
#include "lwip/netif.h"
#include "lwip/etharp.h"
#include "netif/ethernet.h"
#include "lwip/pbuf.h"
#include "lwip/snmp.h"

extern int raw_send(const unsigned char *data, unsigned int len);
extern unsigned int raw_try_recv(unsigned char *dst, unsigned int maxlen);
extern const unsigned char f4_mac[6];

#define IF_MTU 1500
#define IF_NAME0 'e'
#define IF_NAME1 'n'

static err_t low_level_output(struct netif *netif, struct pbuf *p) {
    static unsigned char txscratch[1600];

    (void)netif;
    if (p->tot_len > sizeof(txscratch)) return ERR_MEM;
    pbuf_copy_partial(p, txscratch, p->tot_len, 0);
    return raw_send(txscratch, p->tot_len) ? ERR_OK : ERR_IF;
}

// Drain all pending RX frames into ethernet_input. Returns frames taken.
int netif_f4_poll(struct netif *netif) {
    static unsigned char rxscratch[1600];
    int n = 0;
    for (;;) {
        unsigned int len = raw_try_recv(rxscratch, sizeof(rxscratch));
        if (!len) break;
        struct pbuf *p = pbuf_alloc(PBUF_RAW, (u16_t)len, PBUF_POOL);
        if (!p) break;
        pbuf_take(p, rxscratch, len);
        if (netif->input(p, netif) != ERR_OK) {
            pbuf_free(p);
            break;
        }
        n++;
        if (n >= 4) break; // bound work per pump
    }
    return n;
}

err_t netif_f4_init(struct netif *netif) {
    int i;
    netif->name[0] = IF_NAME0;
    netif->name[1] = IF_NAME1;
    netif->output = etharp_output;
    netif->linkoutput = low_level_output;
    netif->hwaddr_len = 6;
    for (i = 0; i < 6; i++) netif->hwaddr[i] = f4_mac[i];
    netif->mtu = IF_MTU;
    netif->flags = NETIF_FLAG_BROADCAST | NETIF_FLAG_ETHARP | NETIF_FLAG_ETHERNET;
#if LWIP_IGMP
    netif->flags |= NETIF_FLAG_IGMP;
#endif
    return ERR_OK;
}
