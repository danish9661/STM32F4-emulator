#ifndef FREERTOS_CONFIG_H
#define FREERTOS_CONFIG_H

#include <stdint.h>

/* Demo configuration (minimal, cooperative + preemptive tick). */
#define configUSE_PREEMPTION                    1
#define configUSE_IDLE_HOOK                     0
#define configUSE_TICK_HOOK                     0
#define configCPU_CLOCK_HZ                      ( 168000000UL )
#define configSYSTICK_CLOCK_HZ                  ( 168000000UL )
#define configTICK_RATE_HZ                      ( ( TickType_t ) 1000 )
#define configMAX_PRIORITIES                    ( 5 )
#define configMINIMAL_STACK_SIZE                ( ( unsigned short ) 256 )
#define configTOTAL_HEAP_SIZE                   ( ( size_t ) ( 40 * 1024 ) )
#define configMAX_TASK_NAME_LEN                 ( 16 )
#define configUSE_TRACE_FACILITY                0
#define configUSE_16_BIT_TICKS                  0
#define configIDLE_SHOULD_YIELD                 1
#define configUSE_MUTEXES                       0
#define configUSE_RECURSIVE_MUTEXES             0
#define configUSE_COUNTING_SEMAPHORES           0
#define configUSE_ALTERNATIVE_API               0
#define configCHECK_FOR_STACK_OVERFLOW          0
#define configUSE_PORT_OPTIMISED_TASK_SELECTION 0
#define configASSERT_DEFINED                    0
#define configCHECK_HANDLER_INSTALLATION        0
#define configQUEUE_REGISTRY_SIZE               0
#define configUSE_CO_ROUTINES                   0
#define configNUM_THREAD_LOCAL_STORAGE_POINTERS 0
#define configUSE_TIMERS                        0
#define configUSE_NEWLIB_REENTRANT              0

/* API inclusion (this FreeRTOS version defaults these to 0). */
#define INCLUDE_vTaskDelay                     1
#define INCLUDE_xTaskGetTickCount              1
#define INCLUDE_vTaskDelayUntil                1
#define INCLUDE_uxTaskPriorityGet              1
#define configSUPPORT_STATIC_ALLOCATION        0
#define configSUPPORT_DYNAMIC_ALLOCATION       1

/* Cortex-M interrupt priority plumbing. */
#define configPRIO_BITS                        4
#define configLIBRARY_LOWEST_INTERRUPT_PRIORITY   0xf
#define configLIBRARY_MAX_SYSCALL_INTERRUPT_PRIORITY 5
#define configKERNEL_INTERRUPT_PRIORITY         ( configLIBRARY_LOWEST_INTERRUPT_PRIORITY << ( 8 - configPRIO_BITS ) )
#define configMAX_SYSCALL_INTERRUPT_PRIORITY    ( configLIBRARY_MAX_SYSCALL_INTERRUPT_PRIORITY << ( 8 - configPRIO_BITS ) )

/* No FPU on this port build. */
#define configENABLE_FPU                       0

#endif /* FREERTOS_CONFIG_H */
