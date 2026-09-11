void _start(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

__attribute__((used, section(".vectors")))
void (* const vector_table[16])(void) = {
    (void (*)(void))STACK_TOP,
    _start,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
