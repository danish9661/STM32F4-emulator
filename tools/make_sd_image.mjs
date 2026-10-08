// Minimal FAT16 image builder for SPI-SD FS-level tests (no dependencies).
//
// Usage:
//   node tools/make_sd_image.mjs --blocks 8192 --output sd.img
//
// Library use:
//   import { makeFat16Image } from './tools/make_sd_image.mjs';
//   const img = makeFat16Image(8192, [{ name: 'HELLO.TXT', content: 'hello sd world\n' }]);
//
// Layout: superfloppy (boot sector at block 0, no MBR — SdFat mounts this
// fine), FAT16, 512B sectors/cluster=1, reserved=8, 2 FATs, 512 root entries.
// clusters ≈ blocks-104 >> 4085 for the default 8192 (4MB), so FAT16.

const SECTOR = 512;

function le16(buf, off, v) { buf[off] = v & 0xFF; buf[off + 1] = (v >> 8) & 0xFF; }
function le32(buf, off, v) { buf[off] = v & 0xFF; buf[off + 1] = (v >> 8) & 0xFF; buf[off + 2] = (v >> 16) & 0xFF; buf[off + 3] = (v >> 24) & 0xFF; }

function dosName(name) {
    const upper = name.toUpperCase();
    const dot = upper.lastIndexOf('.');
    const base = (dot < 0 ? upper : upper.slice(0, dot)).slice(0, 8).padEnd(8, ' ');
    const ext = (dot < 0 ? '' : upper.slice(dot + 1)).slice(0, 3).padEnd(3, ' ');
    return base + ext;
}

export function makeFat16Image(blocks = 8192, files = [{ name: 'HELLO.TXT', content: 'hello sd world\n' }]) {
    if (blocks < 8200 && files.length > 4) throw new Error('image too small for file count');
    const SPC = 1, RESERVED = 8, FATS = 2, ROOT_ENTRIES = 512;
    const ROOT_SECTORS = (ROOT_ENTRIES * 32) / SECTOR; // 32
    // FAT sectors: iterate once (clusters depend on FAT size).
    let fatSectors = 32;
    for (let i = 0; i < 4; i++) {
        const clusters = Math.floor((blocks - RESERVED - FATS * fatSectors - ROOT_SECTORS) / SPC);
        fatSectors = Math.ceil(((clusters + 2) * 2) / SECTOR);
    }
    const clusters = Math.floor((blocks - RESERVED - FATS * fatSectors - ROOT_SECTORS) / SPC);
    if (clusters < 4085) throw new Error(`needs FAT16 cluster count (>=4085), got ${clusters}; use more --blocks`);
    if (clusters > 65525) throw new Error(`too many clusters for FAT16 (${clusters}); use fewer --blocks`);

    const img = new Uint8Array(blocks * SECTOR).fill(0);

    // ── Boot sector (block 0) ──
    const b = img.subarray(0, SECTOR);
    b[0] = 0xEB; b[1] = 0x3C; b[2] = 0x90;
    b.set(Buffer.from('MSDOS5.0'), 3);
    le16(b, 11, SECTOR);
    b[13] = SPC;
    le16(b, 14, RESERVED);
    b[16] = FATS;
    le16(b, 17, ROOT_ENTRIES);
    if (blocks < 65536) { le16(b, 19, blocks); } else { le32(b, 32, blocks); }
    b[21] = 0xF8;
    le16(b, 22, fatSectors);
    le16(b, 24, 32); // sectors/track (dummy geometry)
    le16(b, 26, 64); // heads
    le32(b, 28, 0);  // hidden
    if (blocks >= 65536) le32(b, 32, blocks);
    b[36] = 0x80; b[38] = 0x29;
    le32(b, 39, 0x12345678);
    b.set(Buffer.from('SD CARD    '), 43);
    b.set(Buffer.from('FAT16   '), 54);
    b[510] = 0x55; b[511] = 0xAA;

    // ── FATs: [media, EOC, chain...] ──
    const fat = new Uint16Array((fatSectors * SECTOR) / 2);
    fat[0] = 0xFFF8; fat[1] = 0xFFFF;
    let nextCluster = 2;
    const chains = [];
    for (const f of files) {
        const bytes = Buffer.isBuffer(f.content) ? f.content : Buffer.from(String(f.content));
        const need = Math.max(1, Math.ceil(bytes.length / (SPC * SECTOR)));
        const chain = [];
        for (let i = 0; i < need; i++) chain.push(nextCluster++);
        for (let i = 0; i < chain.length - 1; i++) fat[chain[i]] = chain[i + 1];
        fat[chain[chain.length - 1]] = 0xFFFF;
        chains.push({ file: f, bytes, chain });
    }
    for (let f = 0; f < FATS; f++) {
        const off = (RESERVED + f * fatSectors) * SECTOR;
        for (let i = 0; i < fat.length; i++) le16(img, off + i * 2, fat[i]);
    }

    // ── Root directory ──
    const rootOff = (RESERVED + FATS * fatSectors) * SECTOR;
    const dataStart = RESERVED + FATS * fatSectors + ROOT_SECTORS;
    chains.forEach(({ file, bytes, chain }, i) => {
        const e = rootOff + i * 32;
        img.set(Buffer.from(dosName(file.name)), e);
        img[e + 11] = 0x20; // archive
        le16(img, e + 26, chain[0]);
        le32(img, e + 28, bytes.length);
        // Write file data (first cluster; multi-cluster chains follow).
        let pos = 0;
        chain.forEach((c, ci) => {
            const sector = dataStart + (c - 2) * SPC;
            const chunk = bytes.subarray(pos, pos + SPC * SECTOR);
            img.set(chunk, sector * SECTOR);
            pos += SPC * SECTOR;
        });
    });

    return { image: img, fatSectors, rootSector: RESERVED + FATS * fatSectors, dataStart, blocks };
}

const isMain = process.argv[1] && import.meta.url.endsWith(process.argv[1].split(/[\\/]/).pop());
if (isMain) {
    const args = process.argv.slice(2);
    const get = (k, d) => {
        const i = args.indexOf(k);
        return i >= 0 && args[i + 1] ? args[i + 1] : d;
    };
    const blocks = parseInt(get('--blocks', '8192'), 10);
    const output = get('--output', 'sd.img');
    const { image } = makeFat16Image(blocks);
    const { writeFileSync } = await import('fs');
    writeFileSync(output, image);
    console.log(`wrote ${output}: ${image.length} bytes (${blocks} blocks), FAT16 superfloppy, HELLO.TXT`);
}
