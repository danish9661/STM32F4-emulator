// Guest-shaped SDIO regression: CMD17 -> CMD24 -> CMD17 round-trip through
// the guest register block (0x40012C00), exactly like guest firmware drives
// it (FlatMemory MMIO path, same call the CPU's STR/LDR take).
//
// Why this exists (2026-10-09): a consumer cell showed CMD24 staging all
// 128 words (DCOUNT 512->0, DATAEND latched) yet the re-read returned
// erased 0xFF while host probes reported a bound 16-block card. That
// signature was first read as a two-instance split (guest driving an
// unbound SDIO while probes reached a bound one). It is NOT: there is
// exactly one Sdio per SVD system (`Sdio::new` fires once in `from_svd`
// for the single SVD "SDIO"; the `new_wasm` chain has no SDIO slot at
// all), `with_sdio` and the address dispatch resolve to it, and this test
// pins that agreement — probe image and guest FIFO readback must match.
// The consumer symptom reproduces byte-exactly when the driver "clears"
// flags via 0x3C (MASK on silicon — ICR is 0x38), leaving the pre-read's
// DATAEND sticky so the CMD24 DATAEND wait exits before the commit window
// (see the mask-fidelity pin below). Correct drivers clear via ICR@0x38.
//
// Usage: node site/test_sdio_guest_roundtrip.mjs   (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../firmware/blinky/blinky.bin', import.meta.url)));

const checks = [];
const ok = (cond, name, extra = '') => {
    checks.push([cond, name]);
    if (cond) console.log(`  ok: ${name}`);
    else console.error(`  FAIL: ${name} ${extra}`);
};

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes, chipHint: 'stm32f407',
    ext_devices: { sdio: { blocks: 16 } },
});

const SD = 0x40012C00;
const W = (off, v) => emu.write32(SD + off, v >>> 0);
const R = (off) => emu.read32(SD + off) >>> 0;
const tick = (n = 200000) => emu.step(n);
// Guest-shaped command recipe: WAITRESP (bit 6) + CPSMEN (bit 10), like the
// Disco register-level driver the consumer cell runs.
const cmd = (idx, arg) => { W(0x08, arg >>> 0); W(0x0C, ((idx & 0x3F) | (1 << 6) | (1 << 10)) >>> 0); };
const ICR = 0x38, MASK = 0x3C;
const STA_DATAEND = 1 << 8, STA_DBCKEND = 1 << 10;
const waitDone = () => { for (let i = 0; i < 300 && !(R(0x34) & (STA_DATAEND | STA_DBCKEND)); i++) tick(); return R(0x34); };
// Consumer pattern words (needle A5 / 7F7F7FDA on words 0 / 127).
const pat = (i) => (((i << 24) | (i << 16) | (i << 8) | (i ^ 0xA5)) >>> 0);
const BLOCK = 5;

ok(bindings.sdio_card_blocks() === 16, 'sdio: 16-block card bound, queryable pre-boot');

// Full guest init: CMD0/8, CMD55+ACMD41, CMD2/3/7 select, CMD16 blocklen.
W(0x00, 3); W(0x04, 118 | (1 << 8)); // POWER on, ~400kHz init clock
cmd(0, 0); tick();
cmd(8, 0x1AA); tick();
ok(R(0x14) === 0x1AA, 'sdio: CMD8 R7 echo');
for (let t = 0; t < 3; t++) { cmd(55, 0); tick(); cmd(41, 0x40000000); tick(); }
cmd(2, 0); tick(); cmd(3, 0); tick();
W(0x08, 0x01D00000); cmd(7, 0x01D00000); tick();
cmd(16, 512); tick();

// CMD17 pre-read: erased image proves the bind + read path.
W(ICR, 0xFFFFFFFF); W(0x24, 0xFFFFFFFF); W(0x28, 512); W(0x2C, (1 | 2 | (9 << 4)) >>> 0);
cmd(17, BLOCK);
waitDone();
let erased = true;
for (let i = 0; i < 128; i++) if (R(0x80) !== 0xFFFFFFFF) { erased = false; break; }
ok(erased, 'sdio: pre-read block 5 erased (bound image, not zeros)');

// MASK-write fidelity (silicon map: ICR=0x38 clears, MASK=0x3C masks):
// programming the mask must NOT clear sticky STA — a driver "clearing"
// via 0x3C keeps the pre-read DATAEND and its next DATAEND wait exits
// instantly, before the commit window (the consumer-cell trap).
W(MASK, 0xFFFFFFFF);
ok((R(0x34) & (STA_DATAEND | STA_DBCKEND)) !== 0, 'sdio: MASK write leaves sticky DATAEND/DBCKEND set');
// Correct clear drops them.
W(ICR, 0xFFFFFFFF);
ok((R(0x34) & (STA_DATAEND | STA_DBCKEND)) === 0, 'sdio: ICR write clears sticky DATAEND/DBCKEND');

// CMD24: no completion/error flag pre-set at arm (flag-model pin), 128
// words stage (DCOUNT 512->0), DATAEND genuinely waits out the window.
W(ICR, 0xFFFFFFFF); W(0x24, 0xFFFFFFFF); W(0x28, 512); W(0x2C, (1 | (9 << 4)) >>> 0);
cmd(24, BLOCK);
ok((R(0x34) & (STA_DATAEND | STA_DBCKEND)) === 0, 'sdio: no DATAEND/DBCKEND pre-set at CMD24 arm');
ok((R(0x34) & ((1 << 1) | (1 << 3))) === 0, 'sdio: no DCRCFAIL/DTIMEOUT pre-set at CMD24 arm');
ok(R(0x30) === 512, 'sdio: DCOUNT=512 at CMD24 arm');
for (let i = 0; i < 128; i++) W(0x80, pat(i));
ok(R(0x30) === 0, 'sdio: DCOUNT=0 after staging 128 words');
const endSTA = waitDone();
ok((endSTA & STA_DATAEND) !== 0, 'sdio: DATAEND after CMD24');

// Single-instance pin: the host probe and the guest FIFO port must agree —
// the probe reaches the same Sdio struct the guest's 0x40012C00 block maps
// to (one Sdio per SVD system; the legacy hardcoded map has no SDIO slot).
const probe = bindings.sdio_read_block(BLOCK);
let probeOk = probe.length === 512;
for (let i = 0; i < 128 && probeOk; i++) {
    const w = (probe[i * 4] | (probe[i * 4 + 1] << 8) | (probe[i * 4 + 2] << 16) | (probe[i * 4 + 3] << 24)) >>> 0;
    if (w !== pat(i)) probeOk = false;
}
ok(probeOk, 'sdio: probe image holds the 128-word pattern (guest slot == bound slot)');

// CMD17 re-read through the guest FIFO port: exact words incl. needle pair.
W(ICR, 0xFFFFFFFF); W(0x28, 512); W(0x2C, (1 | 2 | (9 << 4)) >>> 0);
cmd(17, BLOCK);
waitDone();
let fifoOk = true;
for (let i = 0; i < 128; i++) if ((R(0x80) >>> 0) !== pat(i)) { fifoOk = false; break; }
ok(fifoOk, 'sdio: guest FIFO re-read matches (w0=A5, w127=7F7F7FDA)');

emu.close();
const pass = checks.every(([c]) => c);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
