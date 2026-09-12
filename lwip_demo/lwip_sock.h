// lwip_sock.h — BSD-style socket API over real LwIP (NO_SYS=1 raw
// layer). Single-threaded: blocking calls pump the netif + timers
// internally. Returns lwIP err_t (ERR_OK=0, negative codes) except
// where noted; byte counts on success. select() is level-triggered
// polling (no threads needed); threads/pthreads genuinely need an OS
// and are out of scope (documented in NETWORKING.md).
#ifndef LWIP_SOCK_H
#define LWIP_SOCK_H

#include "lwip/opt.h"
#include "lwip/ip_addr.h"
#include "lwip/err.h"

#define LWIP_SOCK_MAX 4
#define LWIP_SOCK_DGRAM_Q 4
#define LWIP_SOCK_DGRAM_MAX 256

int lwip_sock_init(struct netif *netif);
int lwip_socket(int domain, int type);
int lwip_bind(int s, unsigned int port);
int lwip_listen(int s);
int lwip_accept(int s, unsigned int *rip, unsigned int *rport);
int lwip_connect(int s, unsigned int rip, unsigned int port);
int lwip_send(int s, const unsigned char *data, unsigned int len);
int lwip_recv(int s, unsigned char *buf, unsigned int maxlen);
int lwip_sendto(int s, const unsigned char *data, unsigned int len,
                unsigned int rip, unsigned int port);
int lwip_recvfrom(int s, unsigned char *buf, unsigned int maxlen,
                  unsigned int *rip, unsigned int *rport);
int lwip_closesocket(int s);
// readfds: bitmask over fds 0..3. Returns ready count (0 on timeout).
int lwip_select(unsigned int readfds, unsigned int timeout_ms);
int lwip_sock_readable(int s);
// Resolve via lwIP's real DNS resolver. Returns 0 + packed v4 on success.
int lwip_gethostbyname(const char *name, unsigned int *out_ip);

#endif
