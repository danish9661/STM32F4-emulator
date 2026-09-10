// USB CDC-ACM echo firmware (OTG FS device, polling, no interrupts).
// Boots on UART, enumerates against the harness host (see
// site/test_usb.mjs, the "netsim" that plays USB host through the
// model's usb_* exports), then echoes bulk packets on EP1 twice.
//
// Flow: init -> USBRST -> ENUMDNE -> EP0 control transfers (descriptors,
// address, config, CDC line coding) -> "USB enum done" -> 2x EP1 echo ->
// "USB echo OK" -> done. Any wait-loop timeout prints USB FAIL + spins.
#include <stdint.h>

#define USB_BASE    0x50000000
#define GOTGCTL     (*(volatile unsigned int *)(USB_BASE + 0x000))
#define GOTGINT     (*(volatile unsigned int *)(USB_BASE + 0x004))
#define GAHBCFG     (*(volatile unsigned int *)(USB_BASE + 0x008))
#define GUSBCFG     (*(volatile unsigned int *)(USB_BASE + 0x00C))
#define GRSTCTL     (*(volatile unsigned int *)(USB_BASE + 0x010))
#define GINTSTS     (*(volatile unsigned int *)(USB_BASE + 0x014))
#define GINTMSK     (*(volatile unsigned int *)(USB_BASE + 0x018))
#define GRXSTSP     (*(volatile unsigned int *)(USB_BASE + 0x020))
#define GRXFSIZ     (*(volatile unsigned int *)(USB_BASE + 0x024))
#define GNPTXFSIZ   (*(volatile unsigned int *)(USB_BASE + 0x028))
#define GNPTXSTS    (*(volatile unsigned int *)(USB_BASE + 0x02C))
#define GCCFG       (*(volatile unsigned int *)(USB_BASE + 0x038))
#define DIEPTXF1    (*(volatile unsigned int *)(USB_BASE + 0x104))
#define DCFG        (*(volatile unsigned int *)(USB_BASE + 0x800))
#define DCTL        (*(volatile unsigned int *)(USB_BASE + 0x804))
#define DSTS        (*(volatile unsigned int *)(USB_BASE + 0x808))
#define DIEPMSK     (*(volatile unsigned int *)(USB_BASE + 0x810))
#define DOEPMSK     (*(volatile unsigned int *)(USB_BASE + 0x814))
#define DAINT       (*(volatile unsigned int *)(USB_BASE + 0x818))
#define DAINTMSK    (*(volatile unsigned int *)(USB_BASE + 0x81C))
#define DIEPCTL0    (*(volatile unsigned int *)(USB_BASE + 0x900))
#define DIEPINT0    (*(volatile unsigned int *)(USB_BASE + 0x908))
#define DIEPTSIZ0   (*(volatile unsigned int *)(USB_BASE + 0x910))
#define DIEPCTL1    (*(volatile unsigned int *)(USB_BASE + 0x920))
#define DIEPINT1    (*(volatile unsigned int *)(USB_BASE + 0x928))
#define DIEPTSIZ1   (*(volatile unsigned int *)(USB_BASE + 0x930))
#define DOEPCTL0    (*(volatile unsigned int *)(USB_BASE + 0xB00))
#define DOEPINT0    (*(volatile unsigned int *)(USB_BASE + 0xB08))
#define DOEPTSIZ0   (*(volatile unsigned int *)(USB_BASE + 0xB10))
#define DOEPCTL1    (*(volatile unsigned int *)(USB_BASE + 0xB20))
#define DOEPINT1    (*(volatile unsigned int *)(USB_BASE + 0xB28))
#define DOEPTSIZ1   (*(volatile unsigned int *)(USB_BASE + 0xB30))
#define FIFO0       (*(volatile unsigned int *)(USB_BASE + 0x1000))
#define FIFO1       (*(volatile unsigned int *)(USB_BASE + 0x2000))

// GINTSTS bits
#define G_RXFLVL  (1u << 4)
#define G_NPTXFE  (1u << 5)
#define G_USBRST  (1u << 12)
#define G_ENUMDNE (1u << 13)
// EPCTL bits
#define EPENA (1u << 31)
#define CNAK  (1u << 26)
#define STALLB (1u << 21)
// EPINT bits
#define XFRC 1u
#define STUP (1u << 3)

// --- UART (blinky pattern, the test console) ---
#define RCC_BASE    0x40023800
#define USART1_BASE 0x40011000
#define USART_SR    (*(volatile unsigned int *)(USART1_BASE + 0x00))
#define USART_DR    (*(volatile unsigned int *)(USART1_BASE + 0x04))
#define USART_BRR   (*(volatile unsigned int *)(USART1_BASE + 0x08))
#define USART_CR1   (*(volatile unsigned int *)(USART1_BASE + 0x0C))
static void uart_init(void) {
    *(volatile unsigned int *)(RCC_BASE + 0x30) |= (1 << 0);
    *(volatile unsigned int *)0x40023844 |= (1 << 4);
    *(volatile unsigned int *)0x40020000 = (*(volatile unsigned int *)0x40020000 & ~0xF) | 0xA;
    *(volatile unsigned int *)0x40020024 = (*(volatile unsigned int *)0x40020024 & ~0xF0) | 0x70;
    USART_BRR = 16000000 / 115200;
    USART_CR1 = (1 << 13) | (1 << 3) | (1 << 2);
}
static void uart_putchar(char c) {
    while (!(USART_SR & (1 << 7)));
    USART_DR = c;
}
static void uart_puts(const char *s) { while (*s) uart_putchar(*s++); }
static void uart_hex(unsigned int v, int n) {
    for (int i = n - 1; i >= 0; i--) {
        unsigned int d = (v >> (i * 4)) & 0xF;
        uart_putchar(d < 10 ? '0' + d : 'A' + d - 10);
    }
}
static void fail(const char *why) {
    uart_puts("USB FAIL ");
    uart_puts(why);
    uart_puts("\r\n");
    for (;;);
}
#define WAIT(cond, tag) do { unsigned long t = 3000000; while (!(cond)) if (!t--) fail(tag); } while (0)

// --- descriptors (CDC-ACM, VID:PID 0483:5740) ---
static const unsigned char dev_desc[] = {
    18, 1, 0x00, 0x02, 0x02, 0x00, 0x00, 64,
    0x83, 0x04, 0x40, 0x57, 0x00, 0x02, 1, 2, 3, 1,
};
static const unsigned char cfg_desc[] = {
    9, 2, 67, 0, 2, 1, 0, 0xC0, 50,          // config (wTotal=67)
    9, 4, 0, 0, 1, 2, 2, 1, 0,               // IF0 comm
    5, 0x24, 0, 0x10, 1,                     // header
    5, 0x24, 1, 0, 1,                        // call mgmt
    4, 0x24, 2, 2,                           // ACM
    5, 0x24, 6, 0, 1,                        // union
    7, 5, 0x82, 3, 8, 0, 10,                 // EP2 IN interrupt
    9, 4, 1, 0, 2, 0x0A, 0, 0, 0,            // IF1 data
    7, 5, 0x01, 2, 64, 0, 0,                 // EP1 OUT bulk
    7, 5, 0x81, 2, 64, 0, 0,                 // EP1 IN bulk
};
static const unsigned char str_lang[] = { 4, 3, 0x09, 0x04 };
static const unsigned char str_mfr[] = {
    20, 3, 'S',0,'T',0,'M',0,'3',0,'2',0,'F',0,'4',0,'0',0,'7',0,
};
static const unsigned char str_prod[] = {
    18, 3, 'C',0,'D',0,'C',0,' ',0,'E',0,'c',0,'h',0,'o',0,
};
static const unsigned char str_ser[] = { 10, 3, '0',0,'0',0,'0',0,'1',0 };
static const unsigned char line_coding[7] = { 0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08 };

static unsigned char ep0_buf[128];
static unsigned char echo_buf[128];
static int configured = 0;
static int echo_round = 0;

// --- EP0 helpers ---
// Word-safe FIFO writer (bounds-checked tail: short descriptors must not
// over-read .rodata; the model pads short tails with zero like the FIFO).
static void fifo_write(volatile unsigned int *fifo, const unsigned char *p, unsigned int len) {
    for (unsigned int i = 0; i < len; i += 4) {
        unsigned int w = 0;
        for (unsigned int b = 0; b < 4 && i+b < len; b++)
            w |= (unsigned int)p[i+b] << (8*b);
        *fifo = w;
    }
}
static void fifo_write0(const unsigned char *p, unsigned int len) {
    fifo_write(&FIFO0, p, len);
}
// Word-oriented FIFOs always move whole words: an 18-byte transfer pads
// to 20 on the wire, and the 2 stale bytes would corrupt the NEXT transfer
// (silicon behaves the same — stock drivers flush here too).
static void tx_flush(unsigned int ep) {
    GRSTCTL = (1 << 5) | ((ep & 0x1F) << 6);
    WAIT(!(GRSTCTL & (1 << 5)), "txflsh");
}
// IN data stage on EP0 (PKTCNT covers multi-packet; host slices by MPSIZ).
static void ep0_in_send(const unsigned char *p, unsigned int len) {
    tx_flush(0);
    unsigned int pkts = (len + 63) / 64;
    if (pkts == 0) pkts = 1;
    DIEPTSIZ0 = (pkts << 19) | len;
    fifo_write0(p, len);
    DIEPCTL0 |= EPENA | CNAK;
    WAIT(DIEPINT0 & XFRC, "ep0in");
    DIEPINT0 = XFRC;
}
static void ep0_in_zlp(void) {
    DIEPTSIZ0 = (1 << 19) | 0;
    DIEPCTL0 |= EPENA | CNAK;
    WAIT(DIEPINT0 & XFRC, "ep0zlp");
    DIEPINT0 = XFRC;
}
// Arm EP0-OUT then drain any stale RX statuses (status-stage leftovers).
static void rx_drain(void) {
    unsigned long t = 100000;
    while ((GINTSTS & G_RXFLVL) && t--) { (void)GRXSTSP; }
}
static void ep0_out_arm(void) {
    DOEPTSIZ0 = (1 << 19) | 64;
    DOEPCTL0 |= EPENA | CNAK;
}
// OUT data stage on EP0: returns received length (via GRXSTSP BCNT).
static unsigned int ep0_out_recv(unsigned char *dst, unsigned int maxlen) {
    ep0_out_arm();
    WAIT(GINTSTS & G_RXFLVL, "ep0rx");
    unsigned int st = GRXSTSP;
    unsigned int bcnt = (st >> 4) & 0x7FFF;
    if (bcnt > maxlen) bcnt = maxlen;
    unsigned int words = (bcnt + 3) / 4;
    for (unsigned int i = 0; i < words; i++) {
        unsigned int w = FIFO0;
        for (unsigned int b = 0; b < 4 && 4*i+b < bcnt; b++)
            dst[4*i+b] = (w >> (8*b)) & 0xFF;
    }
    WAIT(DOEPINT0 & XFRC, "ep0xfr");
    DOEPINT0 = XFRC;
    return bcnt;
}
static void ep0_out_status(void) {
    ep0_out_arm();
    WAIT(DOEPINT0 & XFRC, "ep0stat");
    DOEPINT0 = XFRC;
    rx_drain();
}
static void ep0_stall(void) {
    DIEPCTL0 |= STALLB;
    DOEPCTL0 |= STALLB;
}

// --- SETUP handling ---
static void handle_setup(void) {
    unsigned char s[8];
    for (int i = 0; i < 2; i++) {
        unsigned int w = FIFO0;
        s[4*i] = w & 0xFF; s[4*i+1] = (w >> 8) & 0xFF;
        s[4*i+2] = (w >> 16) & 0xFF; s[4*i+3] = (w >> 24) & 0xFF;
    }
    unsigned int req = s[0] | (s[1] << 8);
    unsigned int val = s[2] | (s[3] << 8);
    unsigned int len = s[6] | (s[7] << 8);
    uart_puts("REQ ");
    uart_hex(req, 4);
    uart_puts("\r\n");
    switch (req) {
    case 0x0680: { // GET_DESCRIPTOR standard-IN
        const unsigned char *p = 0;
        unsigned int n = 0;
        unsigned int type = (val >> 8) & 0xFF, idx = val & 0xFF;
        if (type == 1) { p = dev_desc; n = sizeof(dev_desc); }
        else if (type == 2) { p = cfg_desc; n = sizeof(cfg_desc); }
        else if (type == 3) {
            if (idx == 0) { p = str_lang; n = sizeof(str_lang); }
            else if (idx == 1) { p = str_mfr; n = sizeof(str_mfr); }
            else if (idx == 2) { p = str_prod; n = sizeof(str_prod); }
            else if (idx == 3) { p = str_ser; n = sizeof(str_ser); }
        }
        if (!p) { ep0_stall(); break; }
        if (n > len) n = len;
        ep0_in_send(p, n);
        ep0_out_status();
        break;
    }
    case 0x0500: // SET_ADDRESS
        ep0_in_zlp();
        DCFG = (DCFG & ~0x7F0) | ((val & 0x7F) << 4);
        break;
    case 0x0900: { // SET_CONFIGURATION
        ep0_in_zlp();
        // Enable EP1 bulk both dirs, arm first OUT.
        DIEPCTL1 = (DIEPCTL1 & ~0x7FF) | 64;
        DIEPCTL1 = (DIEPCTL1 & ~(3u << 18)) | (2u << 18);
        DIEPCTL1 = (DIEPCTL1 & ~(0xFu << 22)) | (1u << 22);
        DOEPCTL1 = (DOEPCTL1 & ~0x7FF) | 64;
        DOEPCTL1 = (DOEPCTL1 & ~(3u << 18)) | (2u << 18);
        DOEPTSIZ1 = (1 << 19) | 64;
        DOEPCTL1 |= EPENA | CNAK;
        configured = 1;
        uart_puts("USB enum done\r\n");
        break;
    }
    case 0x0080: { // GET_STATUS device
        static const unsigned char z[2] = { 0, 0 };
        ep0_in_send(z, 2);
        ep0_out_status();
        break;
    }
    case 0x0100: // CLEAR_FEATURE
    case 0x0300: // SET_FEATURE
        ep0_in_zlp();
        ep0_out_status();
        break;
    case 0x2221: // SET_CONTROL_LINE_STATE (no data stage)
        ep0_in_zlp();
        break;
    case 0x2021: { // SET_LINE_CODING: 7-byte OUT then status IN
        unsigned int n = ep0_out_recv(ep0_buf, sizeof(ep0_buf));
        (void)n;
        ep0_in_zlp();
        break;
    }
    case 0x21A1: // GET_LINE_CODING
        ep0_in_send(line_coding, 7);
        ep0_out_status();
        break;
    default:
        ep0_stall();
        break;
    }
    // Re-arm EP0-OUT for the next SETUP.
    ep0_out_arm();
}

static void ep1_in_send(const unsigned char *p, unsigned int len) {
    tx_flush(1);
    DIEPTSIZ1 = (1 << 19) | len;
    fifo_write(&FIFO1, p, len);
    DIEPCTL1 |= EPENA | CNAK;
    WAIT(DIEPINT1 & XFRC, "ep1in");
    DIEPINT1 = XFRC;
}

static void usb_init(void) {
    // Clocks + PA11/PA12 AF10.
    *(volatile unsigned int *)(RCC_BASE + 0x30) |= (1 << 0);
    *(volatile unsigned int *)(RCC_BASE + 0x34) |= (1 << 7); // OTGFSEN
    *(volatile unsigned int *)0x40020000 |= 0x2A00000;   // PA11+PA12 AF
    *(volatile unsigned int *)0x40020008 |= 0x3C00000;   // high speed
    *(volatile unsigned int *)0x40020028 |= 0x000AA000;  // AF10 both
    GCCFG |= (1 << 16) | (1 << 21); // PWRDWN + NOVBUSSENS
    GUSBCFG |= (1 << 30);           // FDMOD: force device
    GRSTCTL |= 1;                   // core soft reset
    WAIT(!(GRSTCTL & 1), "csrst");
    GRSTCTL |= (1 << 4);            // flush RX
    WAIT(!(GRSTCTL & (1 << 4)), "rxflsh");
    GRSTCTL |= (1 << 5) | (0x10 << 6); // flush all TX
    WAIT(!(GRSTCTL & (1 << 5)), "txflsh");
    GAHBCFG |= 1;                   // global interrupt
    GINTMSK |= G_USBRST | G_ENUMDNE | G_RXFLVL | (1 << 18) | (1 << 19);
    GRXFSIZ = 128;
    GNPTXFSIZ = (64 << 16) | 128;
    DIEPTXF1 = (64 << 16) | 192;
    DCFG = 0;
    DCTL |= (1 << 1);               // soft disconnect pulse
    for (volatile int i = 0; i < 50000; i++);
    DCTL &= ~(1 << 1);
    DIEPMSK |= 1;
    DOEPMSK |= 1 | (1 << 3);
    DAINTMSK |= (1 | (1 << 1)) | ((1 | (1 << 1)) << 16);
    ep0_out_arm();
    uart_puts("USB init done\r\n");
}

int main(void) {
    uart_init();
    uart_puts("=== USB CDC Test ===\r\n");
    usb_init();
    WAIT(GINTSTS & G_USBRST, "usbrst");
    GINTSTS = G_USBRST;
    uart_puts("USBRST\r\n");
    WAIT(GINTSTS & G_ENUMDNE, "enumdne");
    GINTSTS = G_ENUMDNE;
    uart_puts("ENUMDNE\r\n");
    for (;;) {
        WAIT(GINTSTS & G_RXFLVL, "rx");
        unsigned int st = GRXSTSP;
        unsigned int ep = st & 0xF, pkt = (st >> 21) & 0xF;
        if (ep == 0 && pkt == 6) {
            handle_setup();
        } else if (ep == 1 && pkt == 2 && configured) {
            // EP1-OUT data arrives via the shared RXFIFO (all OUT traffic
            // does on silicon — never the TX window).
            unsigned int bcnt = (st >> 4) & 0x7FFF;
            if (bcnt > sizeof(echo_buf)) bcnt = sizeof(echo_buf);
            unsigned int words = (bcnt + 3) / 4;
            for (unsigned int i = 0; i < words; i++) {
                unsigned int w = FIFO0;
                for (unsigned int b = 0; b < 4 && 4*i+b < bcnt; b++)
                    echo_buf[4*i+b] = (w >> (8*b)) & 0xFF;
            }
            DOEPINT1 = XFRC;
            ep1_in_send(echo_buf, bcnt);
            DOEPTSIZ1 = (1 << 19) | 64;
            DOEPCTL1 |= EPENA | CNAK;
            echo_round++;
            uart_puts("USB echo ");
            uart_putchar('0' + echo_round);
            uart_puts("\r\n");
            if (echo_round >= 2) {
                uart_puts("USB echo OK\r\nUSB done\r\n");
                for (;;);
            }
        } else {
            // Stale/unknown RX event: pop data if any and continue.
            unsigned int bcnt = (st >> 4) & 0x7FFF;
            for (unsigned int i = 0; i < (bcnt + 3) / 4; i++) (void)FIFO0;
        }
    }
}
