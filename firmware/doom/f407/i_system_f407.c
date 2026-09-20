// i_system replacement for the F407 port (replaces engine i_system.c,
// which is SDL-bound). See i_system.h for the API.
#include "doomtype.h"
#include "i_system.h"
#include "doomplatform.h"

#include <stdarg.h>
#include <stdio.h>

// doom_zone_base lives in platform.c
int doom_zone_base(int *size);

byte *I_ZoneBase(int *size)
{
    return (byte *)doom_zone_base(size);
}

void I_Init(void)
{
}

void I_Shutdown(void)
{
}

boolean I_ConsoleStdout(void)
{
    return false;
}

void I_Quit(void)
{
    for (;;) ;
}

void I_Error(char *error, ...)
{
    va_list ap;
    va_start(ap, error);
    vprintf(error, ap);
    va_end(ap);
    printf("\n");
    for (;;) ;
}

void I_Tactile(int on, int off, int total)
{
}

boolean I_GetMemoryValue(unsigned int offset, void *value, int size)
{
    return false;
}

void I_AtExit(atexit_func_t func, boolean run_if_error)
{
}

void I_BindVariables(void)
{
}

void I_PrintStartupBanner(char *gamedescription)
{
    I_PrintBanner(gamedescription);
}

void I_PrintBanner(char *text)
{
    printf("\n");
    printf("%s\n", text);
    printf("(STM32F407 doomgeneric port)\n");
}

void I_PrintDivider(void)
{
    printf("=============================\n");
}
