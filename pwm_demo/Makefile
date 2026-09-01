TOOLCHAIN ?= /home/danish1075/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy
CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: pwm_demo.bin

pwm_demo.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o pwm_demo.elf

pwm_demo.bin: pwm_demo.elf
	$(OBJCOPY) -O binary pwm_demo.elf pwm_demo.bin

clean:
	rm -f pwm_demo.elf pwm_demo.bin

.PHONY: all clean
