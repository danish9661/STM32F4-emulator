TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: eth_http.bin

eth_http.elf: startup.c eth_http.ino link.ld
	$(CC) $(CFLAGS) -x c startup.c eth_http.ino $(LDFLAGS) -o eth_http.elf

eth_http.bin: eth_http.elf
	$(OBJCOPY) -O binary eth_http.elf eth_http.bin
	ls -la eth_http.bin

clean:
	rm -f eth_http.elf eth_http.bin

.PHONY: all clean
