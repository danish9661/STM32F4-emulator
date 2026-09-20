// Register defines + tiny helpers shared by eth_feat_test/main.c.
#define ETH_MAC_BASE    0x40028000
#define ETH_DMA_BASE    0x40029000
#define ETH_PTP_BASE    0x40028700

#define MACCR   (*(volatile unsigned int *)(ETH_MAC_BASE + 0x00))
#define MACFFR  (*(volatile unsigned int *)(ETH_MAC_BASE + 0x04))
#define MACHTHR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x08))
#define MACHTLR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x0C))
#define MACMIIAR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x10))
#define MACMIIDR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x14))
#define MACVLANTR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x1C))
#define MACPMTCTL (*(volatile unsigned int *)(ETH_MAC_BASE + 0x2C))
#define MACIMR  (*(volatile unsigned int *)(ETH_MAC_BASE + 0x3C))
#define MACA0HR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x40))
#define MACA0LR (*(volatile unsigned int *)(ETH_MAC_BASE + 0x44))

#define PTPTSCR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x00))
#define PTPTSHR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x08))
#define PTPTSLR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x0C))
#define PTPTSHUR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x10))
#define PTPTSLUR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x14))
#define PTPTTHR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x1C))
#define PTPTTLR (*(volatile unsigned int *)(ETH_PTP_BASE + 0x20))

#define DMABMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x00))
#define DMATPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x04))
#define DMARPDR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x08))
#define DMARDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x0C))
#define DMATDLAR (*(volatile unsigned int *)(ETH_DMA_BASE + 0x10))
#define DMASR   (*(volatile unsigned int *)(ETH_DMA_BASE + 0x14))
#define DMAOMR  (*(volatile unsigned int *)(ETH_DMA_BASE + 0x18))
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

static void wait_ms(volatile int n) {
    while (n--) for (volatile int i = 0; i < 4000; i++);
}

static unsigned int cksum(unsigned char *p, unsigned int n) {
    unsigned int sum = 0;
    for (unsigned int i = 0; i < n; i += 2)
        sum += ((unsigned int)p[i] << 8) | (i + 1 < n ? p[i + 1] : 0);
    while (sum >> 16) sum = (sum & 0xFFFF) + (sum >> 16);
    return (~sum) & 0xFFFF;
}
