void _start(void);

__attribute__((used, section(".vectors")))
void (* const vector_table[16])(void) = {
    (void (*)(void))0x20020000,
    _start,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =0x20020000\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
