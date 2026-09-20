TOOLCHAIN ?= /home/danish1075/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -O2 -g -ffreestanding -nostdlib -fsingle-precision-constant -Wdouble-promotion -Werror -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -nostdlib -T link.ld -Wl,-gc-sections

all: fpu_irq_test.bin

fpu_irq_test.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o fpu_irq_test.elf

fpu_irq_test.bin: fpu_irq_test.elf
	$(OBJCOPY) -O binary fpu_irq_test.elf fpu_irq_test.bin

clean:
	rm -f fpu_irq_test.elf fpu_irq_test.bin

.PHONY: all clean
