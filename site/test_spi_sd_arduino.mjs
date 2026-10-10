// Arduino-SD.h-equivalent sequence against the model-side SPI SD card.
//
// Replicates the CLASSIC Arduino SD library (Sd2Card, as resolved for the
// STM32 core: no bundled SD lib, ~/.Arduino/libraries/SD) byte-for-byte at
// the SPI wire level — poll-only cardCommand (NO discard read), 74-clock
// init, CMD0/8/55/41/58, MBR-partition-then-superfloppy mount fallback
// (SdVolume::init part 1 -> part 0), CMD17 reads, CMD24 + CMD13 write check,
// and the writeStart (ACMD23 + CMD25 + STOP_TRAN) path. The FS walk test
// covers SdFat-shaped (discard+poll) reads; this one pins the exact driver
// shape the failing e2e firmware uses, so an Ncr/discard-class regression
// fails here first.
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { makeFat16Image } from '../tools/make_sd_image.mjs';

const BLOCKS = 8192;
const CONTENT = 'hello sd world\n';
const { image } = makeFat16Image(BLOCKS, [{ name: 'HELLO.TXT', content: CONTENT }]);

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
const tr = (b) => { W(SPI1 + 0x0C, b); return R(SPI1 + 0x0C) & 0xFF; };
const spiRec = () => tr(0xFF);

// Exact Sd2Card::cardCommand: wait-not-busy, 6-byte frame (paired reads
// discarded, like spiSend), then poll until MSB clear (max 256).
const cardCommand = (idx, arg, crc = 0xFF) => {
    for (let i = 0; i < 64; i++) { if (spiRec() === 0xFF) break; }
    for (const b of [0x40 | idx, (arg >>> 24) & 0xFF, (arg >>> 16) & 0xFF, (arg >>> 8) & 0xFF, arg & 0xFF, crc]) tr(b);
    let st = 0xFF;
    for (let i = 0; i < 256; i++) { st = spiRec(); if (!(st & 0x80)) break; }
    return st;
};
const cardAcmd = (idx, arg) => { cardCommand(55, 0); return cardCommand(idx, arg); };
// Exact Sd2Card::waitStartBlock: poll for the 0xFE token.
const waitStartBlock = () => {
    for (let i = 0; i < 10000; i++) { const s = spiRec(); if (s !== 0xFF) return s; }
    return 0xFF;
};
const readBlock = (n) => {
    if (cardCommand(17, n) !== 0x00) return null;
    if (waitStartBlock() !== 0xFE) return null;
    const buf = new Uint8Array(512);
    for (let i = 0; i < 512; i++) buf[i] = spiRec();
    spiRec(); spiRec(); // CRC
    return buf;
};
// Exact Sd2Card::writeBlock(block, src, blocking=true): CMD24, token, data,
// data-response, wait-not-busy, CMD13 + R2-second-byte check.
const writeBlock = (n, data) => {
    if (cardCommand(24, n) !== 0x00) return 'CMD24';
    for (let i = 0; i < 64; i++) { if (spiRec() === 0xFF) break; }
    tr(0xFE);
    for (let i = 0; i < 512; i++) tr(data[i]);
    tr(0xFF); tr(0xFF); // dummy CRC
    const dr = spiRec();
    if ((dr & 0x1F) !== 0x05) return `data-response ${dr.toString(16)}`;
    for (let i = 0; i < 100000; i++) { if (spiRec() === 0xFF) break; }
    if (cardCommand(13, 0) !== 0x00) return 'CMD13';
    if (spiRec() !== 0x00) return 'R2';
    return null;
};

const checks = [];
const ok = (cond, name) => { checks.push([cond, name]); console.log(`  ${cond ? 'PASS' : 'FAIL'} ${name}`); };

// ── Init: 74+ clocks, CMD0, CMD8, ACMD41(HCS), CMD58 (Sd2Card::init) ──
for (let i = 0; i < 10; i++) tr(0xFF);
let r = 0xFF;
for (let t = 0; t < 10 && r !== 0x01; t++) r = cardCommand(0, 0, 0x95);
ok(r === 0x01, `CMD0 idle (got ${r.toString(16)})`);
r = cardCommand(8, 0x1AA, 0x87);
ok(!(r & 0x04), `CMD8 legal (R1=${r.toString(16)})`);
const r7 = [spiRec(), spiRec(), spiRec(), spiRec()];
ok(r7[3] === 0xAA, `CMD8 R7 echo AA (got ${r7.map((x) => x.toString(16)).join(' ')})`);
r = 0xFF;
for (let t = 0; t < 10 && r !== 0x00; t++) r = cardAcmd(41, 0x40000000);
ok(r === 0x00, `ACMD41 ready (got ${r.toString(16)})`);
ok(cardCommand(58, 0) === 0x00, 'CMD58 R1 ready');
const ocr = [spiRec(), spiRec(), spiRec(), spiRec()];
ok((ocr[0] & 0xC0) === 0xC0, `OCR CCS set (got ${ocr[0].toString(16)})`);

// ── Mount: SdVolume::init part=1 (MBR, must be invalid) -> part=0 ──
const mbr = readBlock(0);
ok(!!mbr, 'MBR block reads');
let partValid = false;
if (mbr) {
    const p = 446; // first partition entry
    const boot = mbr[p], first = mbr[p + 8] | (mbr[p + 9] << 8) | (mbr[p + 10] << 16) | (mbr[p + 11] << 24);
    const total = mbr[p + 12] | (mbr[p + 13] << 8) | (mbr[p + 14] << 16) | (mbr[p + 15] << 24);
    partValid = (boot & 0x7F) === 0 && total >= 100 && first !== 0;
}
ok(!partValid, 'superfloppy: MBR partition invalid (falls back to part 0)');
const le16 = (b, o) => b[o] | (b[o + 1] << 8);
const le32 = (b, o) => (b[o] | (b[o + 1] << 8) | (b[o + 2] << 16) | (b[o + 3] << 24)) >>> 0;
const boot = mbr;
ok(boot && le16(boot, 11) === 512 && boot[16] !== 0 && le16(boot, 14) !== 0 && boot[13] !== 0, 'boot BPB sane');
const rsvd = le16(boot, 14), fats = boot[16], spc = boot[13];
const fatSectors = le16(boot, 22) || le32(boot, 36);
const fatStart = rsvd, rootStart = fatStart + fats * fatSectors;
const rootEnts = le16(boot, 17);
const dataStart = rootStart + Math.floor((32 * rootEnts + 511) / 512);
const totalBlocks = le16(boot, 19) || le32(boot, 32);
const clusterCount = (totalBlocks - dataStart) >> 0;
ok(clusterCount >= 4085 && clusterCount < 65525, `FAT16 cluster count (${clusterCount})`);

// ── Root scan: find HELLO.TXT ──
let found = null;
for (let s = 0; s < Math.floor((32 * rootEnts + 511) / 512) && !found; s++) {
    const blk = readBlock(rootStart + s);
    if (!blk) break;
    for (let e = 0; e < 16; e++) {
        const name = Buffer.from(blk.subarray(e * 32, e * 32 + 11)).toString();
        if (name === 'HELLO   TXT') {
            found = { cl: blk[e * 32 + 26] | (blk[e * 32 + 27] << 8), size: le32(blk, e * 32 + 28) };
            break;
        }
    }
}
ok(!!found, 'HELLO.TXT in root dir');
ok(found && found.cl === 2 && found.size === CONTENT.length, `cluster 2 size ${CONTENT.length} (got ${found && found.cl}/${found && found.size})`);

// ── FAT chain + file data (cluster 2) ──
const fatBlk = readBlock(fatStart);
ok(!!fatBlk, 'FAT block reads');
ok(fatBlk && le16(fatBlk, 4) === 0xFFFF, 'cluster 2 EOC');
const dataBlk = readBlock(dataStart);
ok(!!dataBlk, 'data cluster reads');
ok(dataBlk && Buffer.from(dataBlk.subarray(0, CONTENT.length)).toString() === CONTENT, 'HELLO.TXT byte-exact');

// ── CSD capacity path (Sd2Card::cardSize) ──
ok(cardCommand(9, 0) === 0x00, 'CMD9 R1');
ok(waitStartBlock() === 0xFE, 'CMD9 token');
const csd = new Uint8Array(16);
for (let i = 0; i < 16; i++) csd[i] = spiRec();
spiRec(); spiRec();
ok(((csd[0] >> 6) & 3) === 1, 'CSD v2 (SDHC)');
const csize = ((csd[7] & 0x3F) << 16) | (csd[8] << 8) | csd[9];
ok((csize + 1) * 1024 === BLOCKS, `CSD capacity == ${BLOCKS} blocks (got ${(csize + 1) * 1024})`);

// ── Write: new-file cluster (CMD24 + CMD13/R2), then read back ──
const payload = new Uint8Array(512).map((_, i) => (i * 7 + 3) & 0xFF);
const tBlock = dataStart + 1; // free cluster 3
const werr = writeBlock(tBlock, payload);
ok(werr === null, `CMD24 write+CMD13 clean (${werr || 'ok'})`);
const back = readBlock(tBlock);
ok(!!back && back.every((v, i) => v === payload[i]), 'written block reads back exact');

// ── writeStart path: ACMD23 + CMD25 + STOP_TRAN, then read back ──
ok(cardAcmd(23, 1) === 0x00, 'ACMD23 accepted');
ok(cardCommand(25, dataStart + 2) === 0x00, 'CMD25 R1');
for (let i = 0; i < 64; i++) { if (spiRec() === 0xFF) break; }
tr(0xFC);
for (let i = 0; i < 512; i++) tr((i ^ 0xFF) & 0xFF);
tr(0xFF); tr(0xFF);
const dr25 = spiRec();
ok((dr25 & 0x1F) === 0x05, `CMD25 data accepted (got ${dr25.toString(16)})`);
for (let i = 0; i < 100000; i++) { if (spiRec() === 0xFF) break; }
tr(0xFD); // STOP_TRAN
let busyOk = false;
for (let i = 0; i < 100000; i++) { if (spiRec() === 0xFF) { busyOk = true; break; } }
ok(busyOk, 'STOP_TRAN busy drains to idle');
const back25 = readBlock(dataStart + 2);
ok(!!back25 && back25.every((v, i) => v === ((i ^ 0xFF) & 0xFF)), 'CMD25 block reads back exact');

emu.close();
const pass = checks.every(([c]) => c);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
