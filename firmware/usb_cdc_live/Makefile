TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: usb_cdc_live.bin

usb_cdc_live.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o usb_cdc_live.elf

usb_cdc_live.bin: usb_cdc_live.elf
	$(OBJCOPY) -O binary usb_cdc_live.elf usb_cdc_live.bin

clean:
	rm -f usb_cdc_live.elf usb_cdc_live.bin

.PHONY: all clean
