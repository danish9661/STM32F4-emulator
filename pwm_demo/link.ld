ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 1M
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 128K
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { *(.text*) *(.rodata*) } > FLASH
    .data : { *(.data*) } > RAM AT > FLASH
    .bss : { *(.bss*) } > RAM
}
