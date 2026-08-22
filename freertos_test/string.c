/* Minimal libc substitutes so the firmware links with -nostdlib. */
void *memset(void *s, int c, unsigned int n) {
    unsigned char *b = (unsigned char *)s;
    while (n--) *b++ = (unsigned char)c;
    return s;
}

void *memcpy(void *d, const void *s, unsigned int n) {
    unsigned char *bd = (unsigned char *)d;
    const unsigned char *bs = (const unsigned char *)s;
    while (n--) *bd++ = *bs++;
    return d;
}

int memcmp(const void *a, const void *b, unsigned int n) {
    const unsigned char *pa = (const unsigned char *)a;
    const unsigned char *pb = (const unsigned char *)b;
    while (n--) {
        if (*pa != *pb) return (int)*pa - (int)*pb;
        pa++; pb++;
    }
    return 0;
}
