TOOLCHAIN = C:/Users/Danish/AppData/Local/Arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O0 -g -ffreestanding -nostdlib -I.
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections

all: spi_tft_test.bin

spi_tft_test.elf: startup.c main.c link.ld
	$(CC) $(CFLAGS) startup.c main.c $(LDFLAGS) -o spi_tft_test.elf

spi_tft_test.bin: spi_tft_test.elf
	$(OBJCOPY) -O binary spi_tft_test.elf spi_tft_test.bin

clean:
	rm -f spi_tft_test.elf spi_tft_test.bin

.PHONY: all clean
