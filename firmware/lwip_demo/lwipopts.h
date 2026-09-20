// lwipopts.h — real LwIP 2.2.1 on bare-metal STM32F4, NO_SYS=1.
// Single-threaded: the demo pumps netif input + sys_check_timeouts()
// in its main loop. Software checksums (the CIC offload path is not
// wired into lwIP's NETIF_CHECKSUM scheme).
#ifndef LWIPOPTS_H
#define LWIPOPTS_H

#define NO_SYS                          1
#define LWIP_SOCKET                     0
#define LWIP_NETCONN                    0
#define LWIP_NETIF_API                  0

#define LWIP_IPV4                       1
#define LWIP_IPV6                       0
#define LWIP_ETHERNET                   1
#define LWIP_ARP                        1
#define LWIP_ICMP                       1
#define LWIP_UDP                        1
#define LWIP_TCP                        1
#define LWIP_DHCP                       1
#define LWIP_DHCP_DOES_ACD_CHECK        0
#define LWIP_DNS                        1
#define LWIP_AUTOIP                     0
#define LWIP_IGMP                       0
#define LWIP_ACD                        0
#define LWIP_SNMP                       0
#define LWIP_STATS                      0
#define LWIP_STATS_DISPLAY              0

#define MEM_SIZE                        (20 * 1024)
#define MEM_ALIGNMENT                   4
#define MEMP_NUM_PBUF                   8
#define MEMP_NUM_TCP_PCB                4
#define MEMP_NUM_TCP_SEG                8
#define MEMP_NUM_UDP_PCB                4
#define MEMP_NUM_NETBUF                 0
#define MEMP_NUM_NETCONN                0
#define MEMP_NUM_TCPIP_MSG_API          0
#define MEMP_NUM_TCPIP_MSG_INPKT        0
#define PBUF_POOL_SIZE                  8
#define PBUF_POOL_BUFSIZE               1600

#define TCP_MSS                         536
#define TCP_WND                         (2 * TCP_MSS)
#define TCP_SND_BUF                     (2 * TCP_MSS)
#define TCP_SND_QUEUELEN                8
#define LWIP_WND_SCALE                  0
#define TCP_QUEUE_OOSEQ                 0

#define LWIP_CHECKSUM_CTRL_PER_NETIF    0
#define CHECKSUM_GEN_IP                 1
#define CHECKSUM_GEN_UDP                1
#define CHECKSUM_GEN_TCP                1
#define CHECKSUM_GEN_ICMP               1
#define CHECKSUM_CHECK_IP               1
#define CHECKSUM_CHECK_UDP              1
#define CHECKSUM_CHECK_TCP              1
#define CHECKSUM_CHECK_ICMP             1

#define LWIP_NETIF_HOSTNAME             1
#define LWIP_NETIF_STATUS_CALLBACK      0
#define LWIP_NETIF_LINK_CALLBACK        0
#define LWIP_DHCP_BOOTP_FILE            0
#define LWIP_DNS_SUPPORT_MDNS_QUERIES   0
#define DNS_TABLE_SIZE                  2
#define DNS_MAX_NAME_LENGTH             32

#define SYS_LIGHTWEIGHT_PROT            0
#define LWIP_TIMERS                     1
#define LWIP_RAND()                     (f4_rand())

#define LWIP_DEBUG                      0
#define LWIP_NOASSERT
#define LWIP_PLATFORM_DIAG(x)           do { diag_printf x; } while (0)
#define LWIP_PLATFORM_ASSERT(x)         do { lwip_assert_fail(); } while (0)

#ifdef __cplusplus
extern "C" {
#endif
unsigned int f4_rand(void);
void lwip_assert_fail(void);
void diag_puts(const char *s);
void diag_printf(const char *fmt, ...);
#ifdef __cplusplus
}
#endif

#endif /* LWIPOPTS_H */
