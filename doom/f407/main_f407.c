// Entry point: boot the engine with the memory-resident WAD, then run the
// classic doomgeneric loop (tick + 15 ms sleep so the tic clock paces).
#include "doomgeneric.h"

static char s_ArgV0[] = "doom";
static char s_ArgIwad[] = "-iwad";
static char s_ArgWad[] = "doom1.wad";
static char *s_Argv[] = { s_ArgV0, s_ArgIwad, s_ArgWad, NULL };

int main(void)
{
    doomgeneric_Create(3, s_Argv);
    for (;;) {
        doomgeneric_Tick();
        DG_SleepMs(15);
    }
}
