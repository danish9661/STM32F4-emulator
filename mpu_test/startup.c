extern int main(void);
extern void _start(void);

#ifndef STACK_TOP
#define STACK_TOP 0x20020000
#endif
#define STR_(x) #x
#define STR(x) STR_(x)

void MemManage_Handler(void);
void SVC_Handler(void);
void Default_Handler(void) { while (1); }

// Minimal table: MemManage in slot 4, SVC in slot 11. MemManage is a naked
// trampoline (captures MSP/PSP pre-push, tail-calls MemManage_Handler_c —
// a C prologue's own push would corrupt frame+offset math); SVC is plain C
// (no `interrupt` attribute — it warns under hard-float toolchains even
// for integer-only handlers; stacking is exact so none is needed).
__attribute__((used, section(".vectors")))
void (* const vector_table[16])(void) = {
    (void (*)(void))STACK_TOP,
    _start,
    [2]  = Default_Handler, [3]  = Default_Handler,
    [4]  = MemManage_Handler,
    [5]  = Default_Handler, [6]  = Default_Handler,
    [7]  = Default_Handler, [8]  = Default_Handler,
    [9]  = Default_Handler, [10] = Default_Handler,
    [11] = SVC_Handler,
    [12] = Default_Handler, [13] = Default_Handler,
    [14] = Default_Handler, [15] = Default_Handler,
};

__attribute__((naked)) void _start(void) {
    __asm__ volatile (
        "ldr sp, =" STR(STACK_TOP) "\n"
        "bl main\n"
        "1: b 1b\n"
    );
}
