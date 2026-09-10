TOOLCHAIN ?= C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: blinky_f407g.bin

blinky_f407g.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o blinky_f407g.elf

blinky_f407g.bin: blinky_f407g.elf
	$(OBJCOPY) -O binary blinky_f407g.elf blinky_f407g.bin

clean:
	rm -f blinky_f407g.elf blinky_f407g.bin

.PHONY: all clean
