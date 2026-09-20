# doomgeneric on STM32F407 (emulated) — bare-metal, newlib-nano.
# TOOLCHAIN ?= <path-to>arm-none-eabi-   (env override; default from Arduino core)
TOOLCHAIN ?= $(HOME)/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-
CC = $(TOOLCHAIN)gcc
OBJCOPY = $(TOOLCHAIN)objcopy
SIZE = $(TOOLCHAIN)size

ARCH = -mcpu=cortex-m4 -mthumb -mfloat-abi=soft

CFLAGS = $(ARCH) -Os -g -ffreestanding -std=gnu99 \
	-Iengine -If407 \
	-DDOOMGENERIC_RESX=320 -DDOOMGENERIC_RESY=200 -DCMAP256
LDFLAGS = $(ARCH) -T link.ld -Wl,-gc-sections -nostartfiles \
	--specs=nano.specs -lnosys -lm

# Platform-independent engine sources (everything except the SDL/desktop
# ports, the SDL sound files, and the pieces we replace in f407/).
ENGINE_EXCLUDE = engine/doomgeneric_sdl.c engine/doomgeneric_allegro.c \
	engine/doomgeneric_emscripten.c engine/doomgeneric_linuxvt.c \
	engine/doomgeneric_soso.c engine/doomgeneric_sosox.c \
	engine/doomgeneric_win.c engine/doomgeneric_xlib.c \
	engine/i_sdlsound.c engine/i_sdlmusic.c engine/i_cdmus.c \
	engine/i_allegromusic.c engine/i_allegrosound.c \
	engine/i_joystick.c engine/i_system.c engine/i_sound.c \
	engine/w_file_stdc.c
ENGINE_SRC = $(filter-out $(ENGINE_EXCLUDE),$(wildcard engine/*.c))
F407_SRC = f407/startup.c f407/main_f407.c f407/platform.c \
	f407/i_system_f407.c f407/i_sound_f407.c f407/w_file_mem.c \
	f407/i_joystick_f407.c
SRC = $(ENGINE_SRC) $(F407_SRC)

all: doom.bin

doom.elf: $(SRC) link.ld
	$(CC) $(CFLAGS) $(SRC) $(LDFLAGS) -o doom.elf
	$(SIZE) doom.elf

doom.bin: doom.elf
	$(OBJCOPY) -O binary doom.elf doom.bin

clean:
	rm -f doom.elf doom.bin

.PHONY: all clean
