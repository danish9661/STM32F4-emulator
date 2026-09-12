ENTRY(_start)

MEMORY
{
    FLASH (rx) : ORIGIN = 0x08000000, LENGTH = 2M
    RAM (rwx)  : ORIGIN = 0x20000000, LENGTH = 256K
}

SECTIONS
{
    .vectors : { KEEP(*(.vectors)) } > FLASH
    .text : { *(.text*) *(.rodata*) } > FLASH
    /* Unlike the other demos (explicit static init), real LwIP needs
       proper C runtime init: .data copied from flash, .bss zeroed. */
    .data : { _sdata = .; *(.data*); . = ALIGN(4); _edata = .; } > RAM AT > FLASH
    _sidata = LOADADDR(.data);
    .bss : { _sbss = .; *(.bss*) *(COMMON); . = ALIGN(4); _ebss = .; } > RAM
}
