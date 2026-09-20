TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

# Real LwIP 2.2.1 (vendored subset, NO_SYS=1 raw API).
LWIP_CORE = init.c mem.c memp.c netif.c pbuf.c raw.c stats.c sys.c \
    tcp.c tcp_in.c tcp_out.c udp.c timeouts.c def.c inet_chksum.c \
    dns.c ip.c
LWIP_IPV4 = ipv4/dhcp.c ipv4/etharp.c ipv4/icmp.c ipv4/ip4.c \
    ipv4/ip4_addr.c ipv4/ip4_frag.c
LWIP_SRCS = $(addprefix lwip/core/,$(LWIP_CORE)) $(addprefix lwip/core/,$(LWIP_IPV4)) \
    lwip/netif/ethernet.c

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I. -Ilwip/include -fno-builtin
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: lwip_demo.bin

lwip_demo.elf: startup.c main.c netif_f4.c lwip_sock.c arch/sys_arch.c link.ld $(LWIP_SRCS)
	$(CC) $(CFLAGS) startup.c main.c netif_f4.c lwip_sock.c arch/sys_arch.c $(LWIP_SRCS) $(LDFLAGS) -o lwip_demo.elf

lwip_demo.bin: lwip_demo.elf
	$(OBJCOPY) -O binary lwip_demo.elf lwip_demo.bin

clean:
	rm -f lwip_demo.elf lwip_demo.bin

.PHONY: all clean
