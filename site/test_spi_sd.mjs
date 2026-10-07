// Verifies the SPI SD card path end-to-end through createEmulator:
// ext_devices.spi_sd registration, init sequence, CMD24 write, and an
// exact 512-byte CMD17 read-back. Uses SPI1 with no CS pin (always
// selected) so no GPIO setup is needed; the CS-gated path is pinned by
// the sd_card.rs through-registers test instead.
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const fw = new Uint8Array(readFileSync(new URL('../firmware/spi_flash_test/spi_flash_test.bin', import.meta.url)));
const emu = await createEmulator({
    firmware: fw, bindings, svdXml, wasmInit: wasmBytes,
    ext_devices: { spi_sd: [{ peripheral: 'SPI1', blocks: 4, cs: null }] },
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

const checks = [];
const ok = (cond, name) => { checks.push([cond, name]); console.log(`  ${cond ? 'PASS' : 'FAIL'} ${name}`); };
ok(cmd(0, 0) === 0x01, 'CMD0 R1 idle');
ok(cmd(8, 0x1AA) === 0x01, 'CMD8 R1');
ok(tr(0xFF) === 0x00 && tr(0xFF) === 0x00 && tr(0xFF) === 0x01 && tr(0xFF) === 0xAA, 'CMD8 R7 echo');
ok(cmd(55, 0) === 0x01 && cmd(41, 0) === 0x01, 'ACMD41 busy');
ok(cmd(55, 0) === 0x01 && cmd(41, 0) === 0x00, 'ACMD41 ready');
ok(cmd(24, 1) === 0x00, 'CMD24 R1');
tr(0xFF); tr(0xFE);
for (let i = 0; i < 512; i++) tr(i & 0xFF);
tr(0xFF); tr(0xFF);
ok(tr(0xFF) === 0xE5, 'CMD24 data accepted');
ok(cmd(17, 1) === 0x00, 'CMD17 R1');
ok(tr(0xFF) === 0xFE, 'CMD17 data token');
let good = 0;
for (let i = 0; i < 512; i++) if (tr(0xFF) === (i & 0xFF)) good++;
ok(good === 512, `CMD17 512-byte block reads back exact (good=${good})`);
emu.close();
const pass = checks.every(([c]) => c);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
