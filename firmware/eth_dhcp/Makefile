TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: eth_dhcp.bin

eth_dhcp.elf: startup.c eth_dhcp.ino link.ld
	$(CC) $(CFLAGS) -x c startup.c eth_dhcp.ino $(LDFLAGS) -o eth_dhcp.elf

eth_dhcp.bin: eth_dhcp.elf
	$(OBJCOPY) -O binary eth_dhcp.elf eth_dhcp.bin

clean:
	rm -f eth_dhcp.elf eth_dhcp.bin

.PHONY: all clean
