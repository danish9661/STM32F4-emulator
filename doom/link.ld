ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1M
    /* Internal SRAM: stack (top of RAM) + fixed-address .abi region only. */
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 128K
    /* Engine .data/.bss (≈305K), Doom zone and newlib heap all live in the
       emulated "external SDRAM" at 0xC0000000 (site/emulator.js extra_ram).
       The zone is placed after the engine's data/bss; the WAD image is
       preloaded by the JS driver at 0xB8000000 (same option). */
    EXTRAM (rwx) : ORIGIN = 0xC0000000, LENGTH = 16M
    WADROM (rx)  : ORIGIN = 0xB8000000, LENGTH = 8M
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { *(.text*) *(.rodata*) } > FLASH

    .data :
    {
        __data_load__ = LOADADDR(.data);
        __data_start__ = .;
        *(.data*)
        __data_end__ = .;
    } > EXTRAM AT > FLASH

    .bss :
    {
        __bss_start__ = .;
        *(.bss*)
        *(COMMON)
        __bss_end__ = .;
    } > EXTRAM

    /* Fixed-address guest/JS ABI region in internal SRAM (key ring +
       palette), placed low so the stack (top of RAM, 0x20020000) has
       ~112K of headroom. Link errors if it overlaps anything. */
    .abi 0x20002000 (NOLOAD) : { *(.abi*) } > RAM

    /* Doom zone + newlib heap in external SDRAM, after the engine's
       data/bss (zone base = DOOM_ZONE_ADDR in doomplatform.h). */
    . = 0xC0100000;
    __heap_start = .;
}
