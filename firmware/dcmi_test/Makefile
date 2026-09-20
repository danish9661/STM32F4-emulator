TOOLCHAIN ?= /home/danish1075/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: dcmi_test.bin

dcmi_test.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o dcmi_test.elf

dcmi_test.bin: dcmi_test.elf
	$(OBJCOPY) -O binary dcmi_test.elf dcmi_test.bin

clean:
	rm -f dcmi_test.elf dcmi_test.bin

.PHONY: all clean
