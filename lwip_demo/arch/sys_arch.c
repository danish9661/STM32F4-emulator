// arch/sys_arch.c — NO_SYS shims. sys_now() is the DWT cycle counter
// at 168 MHz (virtual time: deterministic under the emulator).
// f4_rand() is a xorshift (seeded fixed: reproducible runs).
#include "lwip/opt.h"
#include "lwip/sys.h"
#include "lwip/stats.h"
#include <stddef.h>

#define DEMCR   (*(volatile unsigned int *)0xE000EDFC)
#define DWT_CR  (*(volatile unsigned int *)0xE0001000)
#define DWT_CY  (*(volatile unsigned int *)0xE0001004)

void sys_init(void) {
    DEMCR |= (1u << 24);
    DWT_CR |= 1u;
}

// UART diagnostic sink for LWIP_PLATFORM_DIAG (TXE-polled, like uart_puts).
void diag_puts(const char *s) {
    volatile unsigned int *sr = (volatile unsigned int *)0x40011000;
    volatile unsigned int *dr = (volatile unsigned int *)0x40011004;
    while (*s) {
        while (!(*sr & (1u << 7)));
        *dr = (unsigned int)(*s++);
    }
}

static void diag_ch(char c) {
    char b[2] = { c, 0 };
    diag_puts(b);
}

static void diag_unum(unsigned int v, int base, int upper) {
    char buf[12];
    int i = 0;
    if (!v) { diag_ch('0'); return; }
    while (v) {
        unsigned int d = v % (unsigned int)base;
        buf[i++] = (char)(d < 10 ? '0' + d : (upper ? 'A' : 'a') + d - 10);
        v /= (unsigned int)base;
    }
    while (i) diag_ch(buf[--i]);
}

// Minimal printf for LWIP_PLATFORM_DIAG (%d %u %x %X %s %c %%, l/h length
// modifiers skipped). stdarg.h is provided by freestanding GCC.
#include <stdarg.h>
void diag_printf(const char *fmt, ...) {
    va_list ap;
    va_start(ap, fmt);
    while (*fmt) {
        if (*fmt != '%') { diag_ch(*fmt++); continue; }
        fmt++;
        while (*fmt == 'l' || *fmt == 'h') fmt++;
        if (*fmt == 'd' || *fmt == 'i') {
            int v = va_arg(ap, int);
            if (v < 0) { diag_ch('-'); v = -v; }
            diag_unum((unsigned int)v, 10, 0);
        } else if (*fmt == 'u') diag_unum(va_arg(ap, unsigned int), 10, 0);
        else if (*fmt == 'x') diag_unum(va_arg(ap, unsigned int), 16, 0);
        else if (*fmt == 'X') diag_unum(va_arg(ap, unsigned int), 16, 1);
        else if (*fmt == 's') diag_puts(va_arg(ap, const char *));
        else if (*fmt == 'c') diag_ch((char)va_arg(ap, int));
        else if (*fmt == '%') diag_ch('%');
        if (*fmt) fmt++;
    }
    va_end(ap);
}

u32_t sys_now(void) {
    return DWT_CY / 168000u; // ms at 168 MHz
}

static unsigned int rng_state = 0x12345678u;
unsigned int f4_rand(void) {
    // Zero-guard: xorshift(0) sticks at 0 (and .data init is a
    // startup-code courtesy, not a law — never spin on rand).
    if (!rng_state) rng_state = 0x12345678u;
    rng_state ^= rng_state << 13;
    rng_state ^= rng_state >> 17;
    rng_state ^= rng_state << 5;
    return rng_state;
}

// Tiny libc for -nostdlib (lwIP needs these; nothing else).
void *memset(void *d, int c, size_t n) {
    unsigned char *p = (unsigned char *)d;
    while (n--) *p++ = (unsigned char)c;
    return d;
}
void *memcpy(void *d, const void *s, size_t n) {
    unsigned char *p = (unsigned char *)d;
    const unsigned char *q = (const unsigned char *)s;
    while (n--) *p++ = *q++;
    return d;
}
int memcmp(const void *a, const void *b, size_t n) {
    const unsigned char *p = (const unsigned char *)a;
    const unsigned char *q = (const unsigned char *)b;
    while (n--) { if (*p != *q) return *p - *q; p++; q++; }
    return 0;
}
size_t strlen(const char *s) {
    size_t n = 0;
    while (*s++) n++;
    return n;
}
int strncmp(const char *a, const char *b, size_t n) {
    while (n--) {
        if (*a != *b) return (unsigned char)*a - (unsigned char)*b;
        if (!*a) return 0;
        a++; b++;
    }
    return 0;
}
void *memmove(void *d, const void *s, size_t n) {
    unsigned char *p = (unsigned char *)d;
    const unsigned char *q = (const unsigned char *)s;
    if (p < q) {
        while (n--) *p++ = *q++;
    } else if (p > q) {
        p += n; q += n;
        while (n--) *--p = *--q;
    }
    return d;
}
int atoi(const char *s) {
    int v = 0, neg = 0;
    while (*s == ' ' || *s == '\t') s++;
    if (*s == '-') { neg = 1; s++; }
    else if (*s == '+') s++;
    while (*s >= '0' && *s <= '9') v = v * 10 + (*s++ - '0');
    return neg ? -v : v;
}
