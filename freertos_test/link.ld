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
    /* Test-only globals kept out of the low .bss so they never collide
       with FreeRTOS kernel statics (which the linker places at 0x20000000+).
       MUST sit above the 40 KB ucHeap (0x20000100..0x2000A100) PLUS all task
       TCBs/stacks that FreeRTOS carves out of ucHeap (grows upward), or the
       task stacks will scribble over these globals.  0x2001E000 is well clear
       of the heap region and below the MSP main stack (0x20020000). */
    .testvars 0x2001E000 (NOLOAD) : { KEEP(*(.testvars)) } > RAM
}
