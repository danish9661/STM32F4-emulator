// Ethernet + DHCP client for STM32F407 emulator
#include <stdint.h>

// Ethernet MAC registers (base 0x40028000)
#define MACCR     (*(volatile uint32_t *)0x40028000)
#define MACFFR    (*(volatile uint32_t *)0x40028004)
#define MACMIIAR  (*(volatile uint32_t *)0x40028010)
#define MACMIIDR  (*(volatile uint32_t *)0x40028014)
#define MACPMTCSR (*(volatile uint32_t *)0x4002802C)
#define MACSR     (*(volatile uint32_t *)0x40028038)
#define MACA0HR   (*(volatile uint32_t *)0x40028040)
#define MACA0LR   (*(volatile uint32_t *)0x40028044)

// Ethernet DMA registers (base 0x40029000)
#define DMABMR   (*(volatile uint32_t *)0x40029000)
#define DMATPDR  (*(volatile uint32_t *)0x40029004)
#define DMARPDR  (*(volatile uint32_t *)0x40029008)
#define DMARDLAR (*(volatile uint32_t *)0x4002900C)
#define DMATDLAR (*(volatile uint32_t *)0x40029010)
#define DMASR    (*(volatile uint32_t *)0x40029014)
#define DMAOMR   (*(volatile uint32_t *)0x40029018)
#define DMAIER   (*(volatile uint32_t *)0x4002901C)

// RCC
#define RCC_AHB1ENR (*(volatile uint32_t *)0x40023830)
#define RCC_APB2ENR (*(volatile uint32_t *)0x40023844)

// NVIC
#define NVIC_ISER1 (*(volatile uint32_t *)0xE000E104)
#define NVIC_ISER2 (*(volatile uint32_t *)0xE000E108)

// USART1
#define USART_SR  (*(volatile uint32_t *)0x40011000)
#define USART_DR  (*(volatile uint32_t *)0x40011004)
#define USART_BRR (*(volatile uint32_t *)0x40011008)
#define USART_CR1 (*(volatile uint32_t *)0x4001100C)

// SYSTICK
#define STK_CTRL  (*(volatile uint32_t *)0xE000E010)
#define STK_LOAD  (*(volatile uint32_t *)0xE000E014)
#define STK_VAL   (*(volatile uint32_t *)0xE000E018)

// Our MAC address
#define MAC_B0 0x02
#define MAC_B1 0x00
#define MAC_B2 0x00
#define MAC_B3 0x00
#define MAC_B4 0x00
#define MAC_B5 0x01

// RX/TX descriptor ring size
#define ETH_RX_DESC_CNT 4
#define ETH_TX_DESC_CNT 2

// Packet buffers
#define ETH_MAX_PKT 1536
static uint8_t rx_buf[ETH_RX_DESC_CNT][ETH_MAX_PKT] __attribute__((aligned(4)));
static uint8_t tx_pkt[ETH_MAX_PKT] __attribute__((aligned(4)));

// Descriptors (normal format: 2 words each)
static volatile uint32_t rx_desc[ETH_RX_DESC_CNT][2] __attribute__((aligned(8)));
static volatile uint32_t tx_desc[ETH_TX_DESC_CNT][2] __attribute__((aligned(8)));

static volatile uint32_t eth_irq_flag = 0;
static volatile uint32_t rx_frame_len = 0;
static volatile uint32_t rx_frame_idx = 0;
static uint8_t my_ip[4] = {0,0,0,0};
static uint8_t server_ip[4] = {0,0,0,0};
static uint8_t gw_ip[4] = {0,0,0,0};
static uint8_t subnet[4] = {0,0,0,0};
static uint32_t dhcp_xid = 0x12345678;

// ── USART ──
static void uart_init(void) {
    RCC_AHB1ENR |= (1 << 0);
    RCC_APB2ENR |= (1 << 4);
    *(volatile uint32_t *)0x40020000 = (*(volatile uint32_t *)0x40020000 & ~0xF) | 0xA;
    *(volatile uint32_t *)0x40020024 = (*(volatile uint32_t *)0x40020024 & ~0xF0) | 0x70;
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}

static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = (uint8_t)c;
}

static void uart_puts(const char *s) {
    while (*s) uart_putchar(*s++);
}

static void uart_hex8(uint8_t v) {
    const char *hex = "0123456789ABCDEF";
    uart_putchar(hex[v >> 4]); uart_putchar(hex[v & 0xF]);
}

static void uart_ip(uint8_t *ip) {
    uart_putchar('0' + ip[0]/100); uart_putchar('0' + (ip[0]%100)/10); uart_putchar('0' + ip[0]%10);
    uart_putchar('.');
    uart_putchar('0' + ip[1]/100); uart_putchar('0' + (ip[1]%100)/10); uart_putchar('0' + ip[1]%10);
    uart_putchar('.');
    uart_putchar('0' + ip[2]/100); uart_putchar('0' + (ip[2]%100)/10); uart_putchar('0' + ip[2]%10);
    uart_putchar('.');
    uart_putchar('0' + ip[3]/100); uart_putchar('0' + (ip[3]%100)/10); uart_putchar('0' + ip[3]%10);
}

static void uart_hex32(uint32_t v) {
    for (int i = 7; i >= 0; i--) uart_putchar("0123456789ABCDEF"[(v >> (i*4)) & 0xF]);
}

// ── Busy-wait delay (no SysTick needed) ──
static void delay_ms(uint32_t ms) {
    for (uint32_t i = 0; i < ms * 4000; i++) __asm__("nop");
}

// ── Ethernet helpers ──
static void eth_write_reg(volatile uint32_t *reg, uint32_t mask, uint32_t val) {
    *reg = (*reg & ~mask) | (val & mask);
}

static uint16_t eth_phy_read(uint8_t phy, uint8_t reg) {
    MACMIIAR = (1 << 0) | ((phy & 0x1F) << 11) | ((reg & 0x1F) << 6);
    for (int i = 0; i < 10000; i++) {
        if (!(MACMIIAR & 1)) break;
    }
    return (uint16_t)MACMIIDR;
}

static void eth_init(void) {
    RCC_AHB1ENR |= (1 << 25);
    uart_puts("ETH clock ON\r\n");

    DMABMR |= 1;
    delay_ms(2);
    uart_puts("DMA reset\r\n");

    MACCR = (1 << 2) | (1 << 3) | (1 << 11);
    uart_puts("MAC RE+TE\r\n");

    MACA0HR = (MAC_B0 << 8) | MAC_B1 | (1 << 31);
    MACA0LR = (MAC_B2 << 24) | (MAC_B3 << 16) | (MAC_B4 << 8) | MAC_B5;
    uart_puts("MAC addr set\r\n");

    // Wait for link
    for (int i = 0; i < 100; i++) {
        uint16_t bmsr = eth_phy_read(0, 1);
        if (bmsr & 0x4) { uart_puts("link up\r\n"); break; }
        delay_ms(10);
    }

    // Enable DMA TX+RX
    DMAOMR = (1 << 13) | (1 << 1);
    uart_puts("DMA ST+SR\r\n");
}

static void eth_setup_rx(void) {
    for (int i = 0; i < ETH_RX_DESC_CNT; i++) {
        rx_desc[i][0] = 0x80000000 | ETH_MAX_PKT; // OWN=1, RCH=0, RBS1=size
        rx_desc[i][1] = (uint32_t)rx_buf[i];
    }
    DMARDLAR = (uint32_t)rx_desc;
    DMARPDR = 1;
    uart_puts("RX descriptors ready\r\n");
}

static int eth_send_packet(const uint8_t *data, uint32_t len) {
    tx_desc[0][0] = 0x80000000 | (1 << 28) | (1 << 27) | (len & 0x3FFF); // OWN+FS+LS+TBS1
    tx_desc[0][1] = (uint32_t)data;
    DMATDLAR = (uint32_t)tx_desc;
    DMATPDR = 1;
    for (int i = 0; i < 5000000; i++) {
        if (eth_irq_flag & 1) { eth_irq_flag &= ~1; return 1; }
    }
    return 0;
}

static int eth_recv_packet(uint8_t **buf, uint32_t *len) {
    for (int i = 0; i < 5000000; i++) {
        if (eth_irq_flag & 2) {
            eth_irq_flag &= ~2;
            *buf = rx_buf[rx_frame_idx];
            *len = rx_frame_len;
            // Re-arm this descriptor
            rx_desc[rx_frame_idx][0] = 0x80000000 | ETH_MAX_PKT;
            // Re-arm RX poll for the next packet
            DMARPDR = 1;
            return 1;
        }
        // Periodically re-assert RX poll (JS consumes it once)
        if ((i & 0xFFFF) == 0) DMARPDR = 1;
    }
    return 0;
}

// ── ETH IRQ Handler ──
void ETH_IRQHandler(void) {
    uint32_t sr = DMASR;
    if (sr & 1)      { eth_irq_flag |= 1; DMASR = 0x10001; }
    if (sr & (1<<6)) { eth_irq_flag |= 2; DMASR = 1 << 6; }
    // Determine which RX descriptor was used
    for (int i = 0; i < ETH_RX_DESC_CNT; i++) {
        if (!(rx_desc[i][0] & 0x80000000) && (rx_desc[i][0] & 0x1FFFFFFF)) {
            rx_frame_idx = i;
            rx_frame_len = (rx_desc[i][0] >> 16) & 0x3FFF;
            break;
        }
    }
}

// ── Packet builders ──
static void build_arp(uint8_t *buf, uint32_t *len) {
    uint8_t *p = buf;
    // Ethernet header
    for (int i = 0; i < 6; i++) *p++ = 0xFF;       // dst broadcast
    *p++ = MAC_B0; *p++ = MAC_B1; *p++ = MAC_B2;
    *p++ = MAC_B3; *p++ = MAC_B4; *p++ = MAC_B5;   // src
    *p++ = 0x08; *p++ = 0x06;                       // ARP
    // ARP
    *p++ = 0x00; *p++ = 0x01; // HTYPE
    *p++ = 0x08; *p++ = 0x00; // PTYPE
    *p++ = 6;  *p++ = 4;      // HLEN, PLEN
    *p++ = 0x00; *p++ = 0x01; // request
    *p++ = MAC_B0; *p++ = MAC_B1; *p++ = MAC_B2;
    *p++ = MAC_B3; *p++ = MAC_B4; *p++ = MAC_B5;
    *p++ = 10; *p++ = 0; *p++ = 2; *p++ = 15;      // SHA + SPA (10.0.2.15)
    for (int i = 0; i < 6; i++) *p++ = 0;           // THA = 0
    *p++ = 10; *p++ = 0; *p++ = 2; *p++ = 2;        // TPA (10.0.2.2)
    while ((p - buf) < 60) *p++ = 0;                // pad
    *len = 60;
}

static void build_dhcp(uint8_t *buf, uint32_t *len, uint8_t msg_type, uint32_t xid) {
    uint8_t *p = buf;
    // Ethernet header
    *p++ = 0xFF; *p++ = 0xFF; *p++ = 0xFF; *p++ = 0xFF; *p++ = 0xFF; *p++ = 0xFF;
    *p++ = MAC_B0; *p++ = MAC_B1; *p++ = MAC_B2;
    *p++ = MAC_B3; *p++ = MAC_B4; *p++ = MAC_B5;
    *p++ = 0x08; *p++ = 0x00;                       // IP
    // IP header
    uint8_t *iph = p;
    *p++ = 0x45; *p++ = 0x00;                       // v4, IHL=5, DSCP=0
    uint8_t *lenp = p;  p += 2;                     // total len (fill later)
    *p++ = 0x00; *p++ = 0x00;                       // id
    *p++ = 0x00; *p++ = 0x00;                       // flags/frag
    *p++ = 0x80; *p++ = 0x11;                       // TTL=128, UDP
    uint8_t *chkp = p;  p += 2;                     // IP checksum (fill later)
    // IP src = 0.0.0.0, dst = 255.255.255.255
    *p++ = 0; *p++ = 0; *p++ = 0; *p++ = 0;
    *p++ = 255; *p++ = 255; *p++ = 255; *p++ = 255;
    // UDP header
    uint8_t *udph = p;
    *p++ = 0x00; *p++ = 0x44;                       // src port 68 (bootpc)
    *p++ = 0x00; *p++ = 0x43;                       // dst port 67 (bootps)
    uint8_t *udplenp = p; p += 2;                   // UDP len (fill later)
    *p++ = 0x00; *p++ = 0x00;                       // UDP checksum (0 = no checksum)

    uint8_t *dhcpp = p;
    // DHCP header
    *p++ = 0x01; *p++ = 0x01;                       // BOOTREQUEST, HTYPE=eth
    *p++ = 0x06; *p++ = 0x00;                       // HLEN=6, HOPS=0
    *p++ = (xid >> 24) & 0xFF; *p++ = (xid >> 16) & 0xFF;
    *p++ = (xid >> 8) & 0xFF; *p++ = xid & 0xFF;   // XID
    *p++ = 0x00; *p++ = 0x00;                       // SECS
    *p++ = 0x00; *p++ = 0x00;                       // FLAGS
    // CIADDR = 0
    for (int i = 0; i < 4; i++) *p++ = 0;
    // YIADDR = 0
    for (int i = 0; i < 4; i++) *p++ = 0;
    // SIADDR = 0
    for (int i = 0; i < 4; i++) *p++ = 0;
    // GIADDR = 0
    for (int i = 0; i < 4; i++) *p++ = 0;
    // CHADDR = our MAC
    *p++ = MAC_B0; *p++ = MAC_B1; *p++ = MAC_B2;
    *p++ = MAC_B3; *p++ = MAC_B4; *p++ = MAC_B5;
    for (int i = 0; i < 10; i++) *p++ = 0;           // padding
    // Server host name (64 bytes of zero)
    for (int i = 0; i < 64; i++) *p++ = 0;
    // Boot file name (128 bytes of zero)
    for (int i = 0; i < 128; i++) *p++ = 0;
    // DHCP magic cookie
    *p++ = 0x63; *p++ = 0x82; *p++ = 0x53; *p++ = 0x63;
    // DHCP options
    *p++ = 53; *p++ = 1; *p++ = msg_type;            // DHCP message type
    *p++ = 55; *p++ = 3; *p++ = 1; *p++ = 3; *p++ = 6; // Param req list: subnet, router, DNS
    *p++ = 12; *p++ = 4; *p++ = 's'; *p++ = 't'; *p++ = 'm'; *p++ = '3'; // Hostname
    *p++ = 255;                                        // End option
    for (int i = (p - dhcpp) % 4; i > 0 && i < 4; i++) *p++ = 0;

    uint32_t udp_len = (p - udph);
    udplenp[0] = (udp_len >> 8) & 0xFF;
    udplenp[1] = udp_len & 0xFF;

    uint32_t ip_total = (p - iph);
    lenp[0] = (ip_total >> 8) & 0xFF;
    lenp[1] = ip_total & 0xFF;
    // IP checksum
    uint32_t sum = 0;
    for (int i = 0; i < ip_total; i += 2) {
        uint16_t w = (iph[i] << 8) | (i+1 < ip_total ? iph[i+1] : 0);
        sum += w;
    }
    while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
    uint16_t cksum = ~(uint16_t)sum;
    chkp[0] = (cksum >> 8) & 0xFF;
    chkp[1] = cksum & 0xFF;

    *len = p - buf;
    while (*len < 60) { *p++ = 0; (*len)++; }
}

// ── DHCP response parser ──
static int parse_dhcp_ack(uint8_t *buf, uint32_t len) {
    if (len < 42) return 0;
    // Check Ethernet type = IP
    if (buf[12] != 0x08 || buf[13] != 0x00) return 0;
    // IP header
    uint8_t *ip = buf + 14;
    uint8_t ip_hdr_len = (ip[0] & 0x0F) * 4;
    if (ip[9] != 0x11) return 0; // UDP
    // UDP header
    uint8_t *udp = ip + ip_hdr_len;
    uint16_t sport = (udp[0] << 8) | udp[1];
    uint16_t dport = (udp[2] << 8) | udp[3];
    if (sport != 67 || dport != 68) return 0;
    // DHCP
    uint8_t *dhcp = udp + 8;
    uint32_t rxid = (dhcp[4] << 24) | (dhcp[5] << 16) | (dhcp[6] << 8) | dhcp[7];
    if (rxid != dhcp_xid) return 0;
    uint8_t msg_type_val = 0;
    uint8_t *yiaddr = dhcp + 16;
    for (int i = 0; i < 4; i++) my_ip[i] = yiaddr[i];
    // Parse DHCP options
    uint8_t *opt = dhcp + 240;
    uint32_t off = 0;
    while (opt[off] != 0xFF && off < (len - (uint32_t)(opt - buf))) {
        if (opt[off] == 53) { msg_type_val = opt[off+2]; off += 3; continue; }
        if (opt[off] == 1)  { for (int i = 0; i < 4; i++) subnet[i] = opt[off+2+i]; off += opt[off+1]+2; continue; }
        if (opt[off] == 3)  { for (int i = 0; i < 4; i++) gw_ip[i] = opt[off+2+i]; off += opt[off+1]+2; continue; }
        if (opt[off] == 54) { for (int i = 0; i < 4; i++) server_ip[i] = opt[off+2+i]; off += opt[off+1]+2; continue; }
        off += opt[off+1] + 2;
    }
    return msg_type_val;
}

// ── Main ──
void setup(void) {
    uart_init();
    uart_puts("\r\n=== ETH DHCP Test ===\r\n");
    eth_init();
    eth_setup_rx();
    NVIC_ISER1 |= (1 << (61 - 32));
    DMAIER = (1 << 16) | (1 << 0) | (1 << 6); // NIE + TSE + RSE
    uart_puts("ETH ready\r\n");
}

void loop(void) {
    uint8_t *pkt;
    uint32_t pkt_len;

    // Send ARP
    uart_puts("Sending ARP...\r\n");
    build_arp(tx_pkt, &pkt_len);
    if (eth_send_packet(tx_pkt, pkt_len)) {
        uart_puts("ARP sent OK\r\n");
    } else {
        uart_puts("ARP TX timeout\r\n");
        return;
    }

    // DHCP Discover
    uart_puts("Sending DHCP Discover...\r\n");
    build_dhcp(tx_pkt, &pkt_len, 1, dhcp_xid);
    if (eth_send_packet(tx_pkt, pkt_len)) {
        uart_puts("DHCP Discover sent\r\n");
    } else {
        uart_puts("DHCP Discover TX timeout\r\n");
        return;
    }

    // Wait for DHCP Offer
    uart_puts("Waiting for DHCP Offer...\r\n");
    for (int attempt = 0; attempt < 10; attempt++) {
        if (eth_recv_packet(&pkt, &pkt_len)) {
            int mt = parse_dhcp_ack(pkt, pkt_len);
            if (mt == 2) { // OFFER
                uart_puts("DHCP Offer: IP="); uart_ip(my_ip);
                uart_puts(" GW="); uart_ip(gw_ip);
                uart_puts(" SN="); uart_ip(subnet);
                uart_puts(" Server="); uart_ip(server_ip);
                uart_puts("\r\n");
                // Send Request
                uart_puts("Sending DHCP Request...\r\n");
                build_dhcp(tx_pkt, &pkt_len, 3, dhcp_xid);
                if (eth_send_packet(tx_pkt, pkt_len)) {
                    uart_puts("DHCP Request sent\r\n");
                } else {
                    uart_puts("DHCP Request TX timeout\r\n");
                    return;
                }
                // Wait for Ack
                uart_puts("Waiting for DHCP Ack...\r\n");
                for (int ack_attempt = 0; ack_attempt < 10; ack_attempt++) {
                    if (eth_recv_packet(&pkt, &pkt_len)) {
                        int mt2 = parse_dhcp_ack(pkt, pkt_len);
                        if (mt2 == 5) {
                            uart_puts("DHCP Ack: IP="); uart_ip(my_ip);
                            uart_puts("\r\n=== DHCP SUCCESS ===\r\n");
                            return;
                        }
                    }
                }
                uart_puts("DHCP Ack timeout\r\n");
                return;
            }
        }
    }
    uart_puts("DHCP Offer timeout\r\n");
}

// Arduino entry point wrappers
int main(void) { setup(); while(1) loop(); }
