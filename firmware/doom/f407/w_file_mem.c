// stdc_wad_file replacement: the "file" is the WAD image already resident in
// guest memory at DOOM_FB_ADDR (preloaded by the JS driver). Exposes the
// mapped pointer so w_wad.c takes the zero-copy lump path.
#include "w_file.h"
#include "doomplatform.h"

#include <string.h>

// w_file.c references this symbol for the fallback WAD class.
extern wad_file_class_t stdc_wad_file;

static wad_file_t s_memWad;

static wad_file_t *OpenFile(char *path)
{
    if (doom_wad_name(path) == NULL) return NULL;
    s_memWad.file_class = &stdc_wad_file;
    s_memWad.mapped = (byte *)DOOM_FB_ADDR;
    s_memWad.length = DOOM_FB_SIZE;
    return &s_memWad;
}

static void CloseFile(wad_file_t *wad)
{
}

static size_t Read(wad_file_t *wad, unsigned int offset,
                   void *buffer, size_t buffer_len)
{
    if (offset + buffer_len > wad->length) buffer_len = wad->length - offset;
    memcpy(buffer, wad->mapped + offset, buffer_len);
    return buffer_len;
}

wad_file_class_t stdc_wad_file =
{
    OpenFile,
    CloseFile,
    Read,
};
