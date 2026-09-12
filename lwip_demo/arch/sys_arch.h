// arch/sys_arch.h — NO_SYS=1: no threads/mboxes/semaphores. Only the
// clock, protection stubs (single-threaded: no-ops) and rand wiring.
#ifndef LWIP_ARCH_SYS_ARCH_H
#define LWIP_ARCH_SYS_ARCH_H

#define SYS_MBOX_NULL NULL
#define SYS_SEM_NULL  NULL

#ifdef __cplusplus
extern "C" {
#endif
void sys_init(void);
unsigned int sys_now(void);
unsigned int f4_rand(void);
#ifdef __cplusplus
}
#endif

#endif /* LWIP_ARCH_SYS_ARCH_H */
