// i_sound replacement for the F407 port (replaces engine i_sound.c, which is
// SDL-bound). 8-channel 8-bit->16-bit mixer for the DOOM sfx lumps, streamed
// to the emulator's I2S1 capture FIFO (SPI1 block @ 0x40013000, I2SMOD set).
// The JS driver drains the FIFO and plays it via WebAudio at 11025 Hz.
//
// DOOM sound lump format (verified from doom1.wad): 8-byte header
// [u16 3][u16 11025][u16 length][u16 0], then length 8-bit unsigned samples.
#include "doomtype.h"
#include "i_sound.h"
#include "w_wad.h"
#include "z_zone.h"
#include "doomplatform.h"

int snd_sfxdevice = 1;
int snd_musicdevice = 0;
int snd_samplerate = 11025;
int snd_cachesize = 1;
int snd_maxslicetime_ms = 0;
char *snd_musiccmd = NULL;

// I2S1 / SPI1 block (0x40013000) — same registers as audio_play_test.
#define RCC_APB2ENR   (*(volatile uint32_t *)0x40023844)
#define SPI1_CR1      (*(volatile uint32_t *)0x40013000)
#define SPI1_SR       (*(volatile uint32_t *)0x40013008)
#define SPI1_DR       (*(volatile uint32_t *)0x4001300C)
#define SPI1_I2SPR    (*(volatile uint32_t *)0x40013020)
#define SPI1_I2SCFGR  (*(volatile uint32_t *)0x4001301C)
#define SPI1_SR_TXE   (1u << 1)

#define SFX_RATE       11025
#define SFX_CHANNELS   8
#define FRAME_SAMPLES  (SFX_RATE / 35)          // 315 samples per game frame
#define SFX_HEADER     8                        // bytes before the samples

typedef struct
{
    boolean active;
    const byte *samples;
    int length;                 // sample count
    uint32_t pos;               // 16.16 fixed point playhead
    int vol;                    // 0..15
} sfxchan_t;

static sfxchan_t chans[SFX_CHANNELS];

static const byte *sfx_samples(sfxinfo_t *sfx, int *length)
{
    if (sfx->driver_data == NULL)
    {
        int lumpnum = sfx->lumpnum;
        int lumpsize = W_LumpLength(lumpnum);
        if (lumpsize <= SFX_HEADER)
            return NULL;
        const byte *lump = W_CacheLumpNum(lumpnum, PU_STATIC);
        sfx->driver_data = (void *)(lump + SFX_HEADER);
        *length = lumpsize - SFX_HEADER;
        return lump + SFX_HEADER;
    }
    *length = W_LumpLength(sfx->lumpnum) - SFX_HEADER;
    return (const byte *)sfx->driver_data;
}

void I_BindSoundVariables(void)
{
}

// ── sound effects ───────────────────────────────────────────────────────────

void I_InitSound(boolean use_sfx_prefix)
{
    // I2S1 TX: enable SPI1 clock, master TX mode, 16-bit (CHLEN=1 like the
    // audio_play_test reference).  The model captures every DR write.
    RCC_APB2ENR |= (1u << 12);
    SPI1_CR1 = 0;
    SPI1_I2SPR = (2u << 0) | (1u << 8);
    SPI1_I2SCFGR = (1u << 11) | (1u << 10) | (1u << 9) | (1u << 0);
}

void I_ShutdownSound(void)
{
}

int I_GetSfxLumpNum(sfxinfo_t *sfxinfo)
{
    char namebuf[9];
    sprintf(namebuf, "ds%s", sfxinfo->name);
    return W_GetNumForName(namebuf);
}

void I_UpdateSound(void)
{
}

void I_UpdateSoundParams(int channel, int vol, int sep)
{
    if (channel < 0 || channel >= SFX_CHANNELS)
        return;
    chans[channel].vol = vol;
}

int I_StartSound(sfxinfo_t *sfxinfo, int channel, int vol, int sep)
{
    if (channel < 0 || channel >= SFX_CHANNELS)
        return -1;
    int length = 0;
    const byte *samples = sfx_samples(sfxinfo, &length);
    if (samples == NULL)
        return -1;
    sfxchan_t *c = &chans[channel];
    c->active = true;
    c->samples = samples;
    c->length = length;
    c->pos = 0;
    c->vol = vol;
    return channel;
}

void I_StopSound(int channel)
{
    if (channel < 0 || channel >= SFX_CHANNELS)
        return;
    chans[channel].active = false;
}

boolean I_SoundIsPlaying(int channel)
{
    if (channel < 0 || channel >= SFX_CHANNELS)
        return false;
    return chans[channel].active;
}

void I_PrecacheSounds(sfxinfo_t *sounds, int num_sounds)
{
}

// ── music (unsupported; shareware MUS playback is out of scope) ─────────────

void I_InitMusic(void) { }
void I_ShutdownMusic(void) { }
void I_SetMusicVolume(int volume) { }
void I_PauseSong(void) { }
void I_ResumeSong(void) { }
void *I_RegisterSong(void *data, int len) { return data; }
void I_UnRegisterSong(void *handle) { }
void I_PlaySong(void *handle, boolean looping) { }
void I_StopSong(void) { }
boolean I_MusicIsPlaying(void) { return false; }

// ── frame mixer ─────────────────────────────────────────────────────────────
// Called once per rendered frame (from DG_DrawFrame): mixes one frame's worth
// of samples (11025/35) and pushes them into the I2S1 TX FIFO.

void DOOM_SubmitAudio(void)
{
    int s, i;
    // Per-frame normalization: each channel contributes sample*vol (max
    // 127*127 = 16129); scale the summed mix so full-scale needs ALL active
    // channels at max vol.  Hard-clipping at ±32767 is then rare.
    int active = 0;
    for (i = 0; i < SFX_CHANNELS; i++)
        if (chans[i].active) active++;
    const uint32_t scale = active
        ? ((32768u << 8) / (16129u * (uint32_t)active))
        : 0;
    for (s = 0; s < FRAME_SAMPLES; s++)
    {
        int total = 0;
        for (i = 0; i < SFX_CHANNELS; i++)
        {
            sfxchan_t *c = &chans[i];
            if (!c->active)
                continue;
            if (c->pos >= (uint32_t)(c->length << 16))
            {
                c->active = false;
                continue;
            }
            total += ((int)c->samples[c->pos >> 16] - 128) * c->vol;
            c->pos += 0x10000;
        }
        int mix16 = (int)((total * (int64_t)scale) >> 8);
        if (mix16 > 32767) mix16 = 32767;
        else if (mix16 < -32768) mix16 = -32768;
        while (!(SPI1_SR & SPI1_SR_TXE))
            ;
        SPI1_DR = (uint32_t)(uint16_t)(mix16 & 0xFFFF);
    }
}