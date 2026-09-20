// Ethernet MAC register base
#define ETH_MAC_BASE    0x40028000
#define ETH_DMA_BASE    0x40029000

#define MACCR   (*(volatile unsigned int *)(ETH_MAC_BASE + 0x00))
#define MACFFR  (*(volatile unsigned int *)(ETH_MAC_BASE + 0x04))
#define MACMIIAR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x10))
#define MACMIIDR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x14))
#define MACA0HR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x40))
#define MACA0LR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x44))

#define DMABMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x00))
#define DMATPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x04))
#define DMARPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x08))
#define DMARDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x0C))
#define DMATDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x10))
#define DMASR   (*(volatile unsigned int *)(ETH_DMA_BASE + 0x14))
#define DMAOMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x18))
#define DMARDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x0C))
#define DMARPDR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x08))
#define DMAIER   (*(volatile unsigned int *)(ETH_DMA_BASE + 0x1C))

#define RCC_BASE    0x40023800
#define RCC_AHB1ENR (*(volatile unsigned int *)(RCC_BASE + 0x30))

#define NVIC_ISER1  (*(volatile unsigned int *)0xE000E104)

#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))

static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0); // GPIOA
    *(volatile unsigned int *)0x40023844 |= (1 << 4); // RCC APB2 USART1
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA; // PA9 AF
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70; // PA10 AF
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_hex32(unsigned int v) {
    for (int i = 7; i >= 0; i--) {
        unsigned int nib = (v >> (i * 4)) & 0xF;
        uart_putchar(nib < 10 ? '0' + nib : 'A' + nib - 10);
    }
}

// ARP request packet (60 bytes minimum Ethernet frame)
static const unsigned char arp_pkt[60] __attribute__((aligned(4))) = {
    // Ethernet header: dst MAC (broadcast), src MAC, type=ARP
    0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,           // dst
    0x02, 0x00, 0x00, 0x00, 0x00, 0x01,           // src
    0x08, 0x06,                                     // ARP
    // ARP header
    0x00, 0x01, // HTYPE = Ethernet
    0x08, 0x00, // PTYPE = IPv4
    0x06,       // HLEN = 6
    0x04,       // PLEN = 4
    0x00, 0x01, // OPER = request
    // SHA: 02:00:00:00:00:01
    0x02, 0x00, 0x00, 0x00, 0x00, 0x01,
    // SPA: 10.0.2.15
    0x0A, 0x00, 0x02, 0x0F,
    // THA: 00:00:00:00:00:00
    0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    // TPA: 10.0.2.2
    0x0A, 0x00, 0x02, 0x02,
    // Pad to 60 bytes
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    0, 0, 0, 0, 0, 0, 0, 0,
};

// TX descriptor (normal format): tdes0, tdes1
static volatile unsigned int tx_desc[2] __attribute__((aligned(8))) = { 0, 0 };
static volatile int eth_done = 0;
// RX path (single descriptor; re-armed after each frame)
static volatile unsigned int rx_desc[2] __attribute__((aligned(8))) = { 0, 0 };
static volatile unsigned char rx_buf[1536] __attribute__((aligned(4)));
static volatile int rx_flag = 0;

void ETH_IRQHandler(void) {
    unsigned int sr = DMASR;
    if (sr & (1 << 0)) { // TS bit
        eth_done = 1;
        DMASR = (1 << 0) | (1 << 16) | (1 << 14); // write-1-to-clear TS+NIS+AIS
    }
    if (sr & (1 << 6)) { // RS bit
        rx_flag = 1;
        DMASR = (1 << 6) | (1 << 16) | (1 << 14); // write-1-to-clear RS+NIS+AIS
    }
}

static void wait_ms(volatile int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

// ---- IP client helpers (big-endian wire order) ----
static unsigned char tx_frame[512];
static const unsigned char my_ip[4] = {10, 0, 2, 15};
static const unsigned char gw_ip[4] = {10, 0, 2, 2};
static const unsigned char dns_ip[4] = {8, 8, 8, 8};
static const unsigned char gw_mac[6] = {0x5a, 0x94, 0xef, 0xe4, 0x0c, 0xdd};
static const unsigned char my_mac[6] = {0x02, 0x00, 0x00, 0x00, 0x00, 0x01};

static unsigned int cksum(unsigned char *p, unsigned int n) {
    unsigned int sum = 0;
    for (unsigned int i = 0; i < n; i += 2)
        sum += ((unsigned int)p[i] << 8) | (i + 1 < n ? p[i + 1] : 0);
    while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
    return (~sum) & 0xFFFF;
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
    for (int i = 0; i < 1000000; i++) if (eth_done) return 1;
    return 0;
}

// Wait for one RX frame; returns len (0 = timeout). Re-arms the descriptor.
static unsigned int eth_recv_frame(unsigned int timeout_iters) {
    for (unsigned int i = 0; i < timeout_iters; i++) {
        // Re-arm periodically while waiting (the driver drops stale polls,
        // so delivery needs a poll armed after the frame queued).
        if ((i & 0x3FF) == 0) DMARPDR = 1;
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

static void uart_ip(unsigned char *ip) {
    uart_putchar('0' + ip[0] / 100); uart_putchar('0' + (ip[0] % 100) / 10); uart_putchar('0' + ip[0] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[1] / 100); uart_putchar('0' + (ip[1] % 100) / 10); uart_putchar('0' + ip[1] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[2] / 100); uart_putchar('0' + (ip[2] % 100) / 10); uart_putchar('0' + ip[2] % 10);
    uart_putchar('.');
    uart_putchar('0' + ip[3] / 100); uart_putchar('0' + (ip[3] % 100) / 10); uart_putchar('0' + ip[3] % 10);
}

int main(void) {
    uart_init();
    uart_puts("ETH Test: starting\r\n");

    // Enable ETH clock (AHB1 bit 25)
    RCC_AHB1ENR |= (1 << 25);
    uart_puts("ETH clock enabled\r\n");

    // Reset DMA
    DMABMR |= 1;
    wait_ms(10);
    uart_puts("DMA reset done\r\n");

    // Enable MAC TX and RX (RE=2, TE=3)
    MACCR = (1 << 2) | (1 << 3) | (1 << 11); // RE + TE + DM
    uart_puts("MAC enabled\r\n");

    // Set MAC address 02:00:00:00:00:01
    // MAC 02:00:00:00:00:01, MSB-first (IEEE): HR = first two octets.
    MACA0HR = 0x00000200 | (1 << 31); // AE bit
    MACA0LR = 0x00000001;

    // Wait for MII link
    uart_puts("Waiting for link...\r\n");

    // Enable DMA TX and RX
    DMAOMR = (1 << 13) | (1 << 1); // ST + SR
    uart_puts("DMA enabled\r\n");

    // Enable normal interrupt summary (NIS) with TX completion
    DMAIER = (1 << 16) | (1 << 0); // NIE + TSE
    NVIC_ISER1 |= (1 << (61 - 32)); // IRQ 61 in ISER1 (bit 29)

    // RX path for the ICMP/DNS/UDP phases below ( polled flag, ISR-fed)
    DMAIER |= (1 << 6); // RSE: RX completion interrupts too
    rx_desc[0] = 0x80000000 | 1536;
    rx_desc[1] = (unsigned int)&rx_buf[0];
    DMARDLAR = (unsigned int)&rx_desc[0];
    DMARPDR = 1;

    uart_puts("Setup complete. Starting TX...\r\n");

    // Build TX descriptor chain with one descriptor
    tx_desc[0] = 0x80000000 | (60 & 0x3FFF); // OWN=1, FS=0 (not set), LS=0, TCH=0, TBS1=60
    tx_desc[1] = (unsigned int)&arp_pkt[0];

    // Set TX descriptor list address
    DMATDLAR = (unsigned int)&tx_desc[0];

    // Poll transmit demand
    DMATPDR = 1;

    // Wait for TX completion
    for (int i = 0; i < 1000000; i++) {
        if (eth_done) break;
    }

    if (eth_done) {
        uart_puts("TX completed. tdes0=");
        uart_hex32(tx_desc[0]);
        uart_puts("\r\n");
    } else {
        uart_puts("TIMEOUT! dmasr=");
        uart_hex32(DMASR);
        uart_puts("\r\n");
    }

    // ---- ICMP echo: request the gateway, answer any request seen ----
    {
        unsigned int off = ip_header(gw_mac, gw_ip, 1, 12);
        tx_frame[off] = 8; tx_frame[off + 1] = 0; // echo request
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 0;
        tx_frame[off + 4] = 0x12; tx_frame[off + 5] = 0x34;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 1;
        tx_frame[off + 8] = 0xAA; tx_frame[off + 9] = 0xBB;
        tx_frame[off + 10] = 0xCC; tx_frame[off + 11] = 0xDD;
        unsigned int ck = cksum(&tx_frame[off], 12);
        tx_frame[off + 2] = ck >> 8; tx_frame[off + 3] = ck & 0xFF;
        int got_reply = 0, sent_rx_reply = 0;
        if (eth_send_frame(14 + 20 + 12)) {
            for (int round = 0; round < 200 && !(got_reply && sent_rx_reply); round++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue; // not IP
                if (rx_buf[23] != 1) continue; // not ICMP
                unsigned int io = 14 + ((rx_buf[14] & 0xF) * 4);
                if (rx_buf[io] == 0 && rx_buf[io + 4] == 0x12 && rx_buf[io + 5] == 0x34) {
                    uart_puts("ICMP reply OK\r\n");
                    got_reply = 1;
                } else if (rx_buf[io] == 8) {
                    // Answer any echo request (MACs/IPs swapped, type 0).
                    for (int i = 0; i < 6; i++) { unsigned char t = rx_buf[i]; rx_buf[i] = rx_buf[6 + i]; rx_buf[6 + i] = t; }
                    for (int i = 0; i < 4; i++) { unsigned char t = rx_buf[26 + i]; rx_buf[26 + i] = rx_buf[30 + i]; rx_buf[30 + i] = t; }
                    rx_buf[io] = 0;
                    rx_buf[io + 2] = 0; rx_buf[io + 3] = 0;
                    unsigned int ilen = len - io;
                    unsigned int ck2 = cksum((unsigned char *)&rx_buf[io], ilen);
                    rx_buf[io + 2] = ck2 >> 8; rx_buf[io + 3] = ck2 & 0xFF;
                    for (unsigned int i = 0; i < len; i++) tx_frame[i] = rx_buf[i];
                    if (eth_send_frame(len)) { uart_puts("ICMP RX reply sent\r\n"); sent_rx_reply = 1; }
                }
            }
        }
        if (!got_reply) uart_puts("ICMP no reply (tolerated)\r\n");
    }

    // ---- DNS A query for example.com ----
    {
        unsigned int off = ip_header(gw_mac, dns_ip, 17, 8 + 12 + 13 + 4);
        // UDP header: sport 49153 -> dport 53 (checksum 0 = legal)
        tx_frame[off] = 49153 >> 8; tx_frame[off + 1] = 49153 & 0xFF;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 53;
        unsigned int ulen = 8 + 12 + 13 + 4;
        tx_frame[off + 4] = ulen >> 8; tx_frame[off + 5] = ulen & 0xFF;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        // DNS header: TXID 0x1234, RD set, QDCOUNT 1
        unsigned int dbo = off + 8;
        tx_frame[dbo] = 0x12; tx_frame[dbo + 1] = 0x34;
        tx_frame[dbo + 2] = 0x01; tx_frame[dbo + 3] = 0x00;
        tx_frame[dbo + 4] = 0; tx_frame[dbo + 5] = 1;
        tx_frame[dbo + 6] = 0; tx_frame[dbo + 7] = 0;
        tx_frame[dbo + 8] = 0; tx_frame[dbo + 9] = 0;
        tx_frame[dbo + 10] = 0; tx_frame[dbo + 11] = 0;
        // QNAME labels for "example.com" + QTYPE A + QCLASS IN
        unsigned int qo = dbo + 12;
        const char *labels[2] = { "example", "com" };
        for (int li = 0; li < 2; li++) {
            int sl = 0; while (labels[li][sl]) sl++;
            tx_frame[qo++] = sl;
            for (int ci = 0; ci < sl; ci++) tx_frame[qo++] = labels[li][ci];
        }
        tx_frame[qo++] = 0;
        tx_frame[qo++] = 0; tx_frame[qo++] = 1;
        tx_frame[qo++] = 0; tx_frame[qo++] = 1;
        int got_dns = 0;
        if (eth_send_frame(14 + 20 + ulen)) {
            for (int round = 0; round < 200 && !got_dns; round++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue;
                if (rx_buf[23] != 17) continue;
                unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
                if (rx_buf[uo] != 0 || rx_buf[uo + 1] != 53) continue; // not from DNS
                // match TXID + ANCOUNT>=1, then skip our own question
                unsigned int ro = uo + 8;
                if (rx_buf[ro] != 0x12 || rx_buf[ro + 1] != 0x34) continue;
                if (rx_buf[ro + 7] < 1) continue;
                unsigned int ao = ro + 12;
                while (ao < len && rx_buf[ao] != 0) ao += rx_buf[ao] + 1;
                ao += 1 + 4; // NUL + QTYPE + QCLASS
                // answer: NAME(2) TYPE(2) CLASS(2) TTL(4) RDLEN(2) RDATA
                if (rx_buf[ao + 2] == 0 && rx_buf[ao + 3] == 1 && rx_buf[ao + 10] == 0 && rx_buf[ao + 11] == 4) {
                    uart_puts("DNS IP=");
                    uart_ip((unsigned char *)&rx_buf[ao + 12]);
                    uart_puts("\r\n");
                    got_dns = 1;
                }
            }
        }
        if (!got_dns) uart_puts("DNS TIMEOUT (tolerated)\r\n");
    }

    // ---- UDP echo to port 7 ----
    {
        unsigned int off = ip_header(gw_mac, gw_ip, 17, 8 + 8);
        tx_frame[off] = 49154 >> 8; tx_frame[off + 1] = 49154 & 0xFF;
        tx_frame[off + 2] = 0; tx_frame[off + 3] = 7;
        tx_frame[off + 4] = 0; tx_frame[off + 5] = 16;
        tx_frame[off + 6] = 0; tx_frame[off + 7] = 0;
        const char *msg = "UDP ECHO";
        for (int i = 0; i < 8; i++) tx_frame[off + 8 + i] = msg[i];
        int got_echo = 0;
        if (eth_send_frame(14 + 20 + 16)) {
            for (int round = 0; round < 200 && !got_echo; round++) {
                unsigned int len = eth_recv_frame(20000);
                if (!len) continue;
                if (rx_buf[12] != 0x08 || rx_buf[13] != 0x00) continue;
                if (rx_buf[23] != 17) continue;
                unsigned int uo = 14 + ((rx_buf[14] & 0xF) * 4);
                if (rx_buf[uo + 2] != (49154 >> 8) || rx_buf[uo + 3] != (49154 & 0xFF)) continue;
                int match = 1;
                for (int i = 0; i < 8; i++) if (rx_buf[uo + 8 + i] != (unsigned char)msg[i]) match = 0;
                if (match) { uart_puts("UDP echo OK\r\n"); got_echo = 1; }
            }
        }
        if (!got_echo) uart_puts("UDP echo TIMEOUT (no peer)\r\n");
    }

    uart_puts("ETH Test: done\r\n");
    while (1);
}
