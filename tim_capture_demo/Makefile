TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: tim_capture_demo.bin

tim_capture_demo.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o tim_capture_demo.elf

tim_capture_demo.bin: tim_capture_demo.elf
	$(OBJCOPY) -O binary tim_capture_demo.elf tim_capture_demo.bin

clean:
	rm -f tim_capture_demo.elf tim_capture_demo.bin

.PHONY: all clean
