// i_sound replacement for the F407 port (replaces engine i_sound.c, which is
// SDL-bound). Silent stubs for now; the I2S audio path can be wired later.
#include "doomtype.h"
#include "i_sound.h"

int snd_sfxdevice = 0;
int snd_musicdevice = 0;
int snd_samplerate = 11025;
int snd_cachesize = 1;
int snd_maxslicetime_ms = 0;
char *snd_musiccmd = NULL;

void I_BindSoundVariables(void)
{
}

// ── sound effects ───────────────────────────────────────────────────────────

void I_InitSound(boolean use_sfx_prefix)
{
}

void I_ShutdownSound(void)
{
}

int I_GetSfxLumpNum(sfxinfo_t *sfxinfo)
{
    return sfxinfo->lumpnum;
}

void I_UpdateSound(void)
{
}

void I_UpdateSoundParams(int channel, int vol, int sep)
{
}

int I_StartSound(sfxinfo_t *sfxinfo, int channel, int vol, int sep)
{
    return -1;
}

void I_StopSound(int channel)
{
}

boolean I_SoundIsPlaying(int channel)
{
    return false;
}

void I_PrecacheSounds(sfxinfo_t *sounds, int num_sounds)
{
}

// ── music ───────────────────────────────────────────────────────────────────

void I_InitMusic(void)
{
}

void I_ShutdownMusic(void)
{
}

void I_SetMusicVolume(int volume)
{
}

void I_PauseSong(void)
{
}

void I_ResumeSong(void)
{
}

void *I_RegisterSong(void *data, int len)
{
    return data;
}

void I_UnRegisterSong(void *handle)
{
}

void I_PlaySong(void *handle, boolean looping)
{
}

void I_StopSong(void)
{
}

boolean I_MusicIsPlaying(void)
{
    return false;
}
