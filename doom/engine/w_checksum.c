//
// Copyright(C) 1993-1996 Id Software, Inc.
// Copyright(C) 2005-2014 Simon Howard
//
// This program is free software; you can redistribute it and/or
// modify it under the terms of the GNU General Public License
// as published by the Free Software Foundation; either version 2
// of the License, or (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// DESCRIPTION:
//       Generate a checksum of the WAD directory.
//

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "m_misc.h"
#include "sha1.h"
#include "w_checksum.h"
#include "w_wad.h"

static wad_file_t **open_wadfiles = NULL;
static int num_open_wadfiles = 0;

static int GetFileNumber(wad_file_t *handle)
{
    int i;
    int result;

    for (i=0; i<num_open_wadfiles; ++i)
    {
        if (open_wadfiles[i] == handle)
        {
            return i;
        }
    }

    // Not found in list.  This is a new file we haven't seen yet.
    // Allocate another slot for this file.

    open_wadfiles = realloc(open_wadfiles,
                            sizeof(wad_file_t *) * (num_open_wadfiles + 1));
    open_wadfiles[num_open_wadfiles] = handle;

    result = num_open_wadfiles;
    ++num_open_wadfiles;

    return result;
}

static void ChecksumAddLump(sha1_context_t *sha1_context, lumpinfo_t *lump)
{
    char buf[9];

    M_StringCopy(buf, lump->name, sizeof(buf));
    SHA1_UpdateString(sha1_context, buf);
    SHA1_UpdateInt32(sha1_context, GetFileNumber(lump->wad_file));
    SHA1_UpdateInt32(sha1_context, lump->position);
    SHA1_UpdateInt32(sha1_context, lump->size);
}

void W_Checksum(sha1_digest_t digest)
{
    // NOTE: the SHA-1 transform (sha1.c Transform) wedges the Unicorn WASM
    // TCI translator at boot (batch-size-dependent abort, tcg_qemu_tb_exec_arm
    // TODO at tci.c:1272). The digest is only used for net-game/connect
    // verification (d_net.c W_Checksum call), which is meaningless in
    // single-player, so zero it and skip the hashing entirely.
    memset(digest, 0, sizeof(sha1_digest_t));
}

