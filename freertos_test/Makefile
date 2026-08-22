TOOLCHAIN ?= /home/danish1075/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
LD = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy

FREERTOS = FreeRTOS
INC = -I. -I$(FREERTOS)/include -I$(FREERTOS)/portable/GCC/ARM_CM3

CFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib $(INC)
LDFLAGS = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -T link.ld -Wl,-gc-sections \
          -specs=nosys.specs -specs=nano.specs

SRCS = startup.c main.c string.c \
       $(FREERTOS)/tasks.c $(FREERTOS)/list.c $(FREERTOS)/queue.c \
       $(FREERTOS)/portable/GCC/ARM_CM3/port.c \
       $(FREERTOS)/portable/MemMang/heap_4.c

OBJS = $(SRCS:.c=.o)

all: freertos_test.bin

freertos_test.elf: $(OBJS) link.ld
	$(CC) $(CFLAGS) $(OBJS) $(LDFLAGS) -o freertos_test.elf

freertos_test.bin: freertos_test.elf
	$(OBJCOPY) -O binary freertos_test.elf freertos_test.bin

%.o: %.c
	$(CC) $(CFLAGS) -c $< -o $@

clean:
	rm -f $(OBJS) freertos_test.elf freertos_test.bin

.PHONY: all clean
