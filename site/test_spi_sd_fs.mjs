// FS-level SPI SD test: seeds the card with a real FAT16 image, then walks
// the filesystem structures through the SPI wire path (CMD17 reads):
// boot sector (0x55AA + "FAT16"), root directory (HELLO.TXT entry with
// cluster + size), and the file's data cluster (byte-exact content).
// Also pins the SdFat-hang regression: CMD59/32/33/38 answer R1 (not 0x04).
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { makeFat16Image } from '../tools/make_sd_image.mjs';

const BLOCKS = 8192;
const CONTENT = 'hello sd world\n';
const { image, rootSector, dataStart } = makeFat16Image(BLOCKS, [{ name: 'HELLO.TXT', content: CONTENT }]);

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const fw = new Uint8Array(readFileSync(new URL('../firmware/spi_flash_test/spi_flash_test.bin', import.meta.url)));
const emu = await createEmulator({
    firmware: fw, bindings, svdXml, wasmInit: wasmBytes,
    ext_devices: { spi_sd: [{ peripheral: 'SPI1', blocks: BLOCKS, cs: null, data: image }] },
});

const SPI1 = 0x40013000;
const W = (a, v) => emu.write32(a, v >>> 0);
const R = (a) => emu.read32(a) >>> 0;
W(SPI1, (1 << 2) | (1 << 6)); // MSTR + SPE
const cmd = (idx, arg) => {
    let last = 0;
    for (const b of [0x40 | idx, (arg >>> 24) & 0xFF, (arg >>> 16) & 0xFF, (arg >>> 8) & 0xFF, arg & 0xFF, 0xFF]) {
        W(SPI1 + 0x0C, b); last = R(SPI1 + 0x0C) & 0xFF;
    }
    return last;
};
const tr = (b) => { W(SPI1 + 0x0C, b); return R(SPI1 + 0x0C) & 0xFF; };
const readBlock = (n) => {
    if (cmd(17, n) !== 0x00) return null;
    if (tr(0xFF) !== 0xFE) return null;
    const buf = new Uint8Array(512);
    for (let i = 0; i < 512; i++) buf[i] = tr(0xFF);
    tr(0xFF); tr(0xFF);
    return buf;
};

const checks = [];
const ok = (cond, name) => { checks.push([cond, name]); console.log(`  ${cond ? 'PASS' : 'FAIL'} ${name}`); };

// Init sequence (same as block-RW test).
ok(cmd(0, 0) === 0x01, 'CMD0 R1 idle');
ok(cmd(8, 0x1AA) === 0x01, 'CMD8 R1');
tr(0xFF); tr(0xFF); tr(0xFF); tr(0xFF);
ok(cmd(55, 0) === 0x01 && cmd(41, 0) === 0x01, 'ACMD41 busy');
ok(cmd(55, 0) === 0x01 && cmd(41, 0) === 0x00, 'ACMD41 ready (2nd poll)');
ok(cmd(58, 0) === 0x00, 'CMD58 R1 ready');
tr(0xFF); tr(0xFF); tr(0xFF); tr(0xFF);

// SdFat-hang regression: CRC/erase commands answer R1, not illegal-command.
ok(cmd(59, 0) === 0x00, 'CMD59(0) accepted');
ok(cmd(32, 0) === 0x00, 'CMD32 accepted');
ok(cmd(33, 7) === 0x00, 'CMD33 accepted');
ok(cmd(38, 0) === 0x00, 'CMD38 accepted');

// FS walk: boot sector.
const boot = readBlock(0);
ok(!!boot, 'block 0 reads');
ok(boot && boot[510] === 0x55 && boot[511] === 0xAA, 'boot signature 55AA');
ok(boot && Buffer.from(boot.subarray(54, 59)).toString() === 'FAT16', 'boot fstype FAT16');

// FS walk: root directory entry.
const root = readBlock(rootSector);
ok(!!root, `root dir block ${rootSector} reads`);
const entryName = root ? Buffer.from(root.subarray(0, 11)).toString() : '';
ok(entryName === 'HELLO   TXT', `root entry name (${entryName})`);
const cluster = root ? (root[26] | (root[27] << 8)) : -1;
const size = root ? (root[28] | (root[29] << 8) | (root[30] << 16) | (root[31] << 24)) : -1;
ok(cluster === 2, `first cluster == 2 (got ${cluster})`);
ok(size === CONTENT.length, `file size == ${CONTENT.length} (got ${size})`);

// FS walk: file data cluster.
const data = readBlock(dataStart);
ok(!!data, `data cluster block ${dataStart} reads`);
ok(data && Buffer.from(data.subarray(0, CONTENT.length)).toString() === CONTENT, 'file content byte-exact');

emu.close();
const pass = checks.every(([c]) => c);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
