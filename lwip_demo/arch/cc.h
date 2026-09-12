// arch/cc.h — compiler/platform shims for real LwIP on bare-metal ARM.
#ifndef LWIP_ARCH_CC_H
#define LWIP_ARCH_CC_H

/* Freestanding (-nostdlib): no libc ctype — use lwIP's private versions. */
#define LWIP_NO_CTYPE_H 1

#include <stdint.h>

typedef uint8_t u8_t;
typedef int8_t s8_t;
typedef uint16_t u16_t;
typedef int16_t s16_t;
typedef uint32_t u32_t;
typedef int32_t s32_t;
/* mem_ptr_t comes from lwip/arch.h (uintptr_t); do not redefine here. */

#define U16_F "hu"
#define S16_F "hd"
#define X16_F "hx"
#define U32_F "u"
#define S32_F "d"
#define X32_F "x"

#define PACK_STRUCT_BEGIN
#define PACK_STRUCT_STRUCT __attribute__((packed))
#define PACK_STRUCT_END
#define PACK_STRUCT_FIELD(x) x
#define ALIGNED(n) __attribute__((aligned(n)))

#define LWIP_PLATFORM_BYTESWAP 0
#define LWIP_PLATFORM_HTONS(x) ((((x) & 0xFF) << 8) | (((x) >> 8) & 0xFF))
#define LWIP_PLATFORM_HTONL(x) ((((x) & 0xFF) << 24) | (((x) & 0xFF00) << 8) | (((x) & 0xFF0000) >> 8) | (((x) >> 24) & 0xFF))

#endif /* LWIP_ARCH_CC_H */
