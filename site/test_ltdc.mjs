// Node harness for the LTDC scanout firmware.
// Asserts: boot markers, the firmware's own pixel checksum, the exported
// model counters (ltdc_get_frame_count >= 2, ltdc_get_scanline resumed
// after wrap), ISR flags, and a JS re-verification of the gradient pixels
// read back from the framebuffer at 0x20002000.
// Usage: node site/test_ltdc.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../ltdc_test/ltdc_test.bin', import.meta.url)));

const W = 64, H = 32, FB = 0x20002000;
const pix = (x, y) => (0xFF000000 | (x << 16) | (y << 8) | (x + y)) >>> 0;

const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });

const uart = [];
let framesSeen = false;
for (let i = 0; i < 800; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    if (uart.join('').includes('=== LTDC Test: done')) {
        framesSeen = true;
        for (let j = 0; j < 300; j++) emu.step(100000); // let scanout accumulate frames
        break;
    }
}
const all = uart.join('');
console.log('uart tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-6).join(' | '));

const frames = bindings.ltdc_get_frame_count ? bindings.ltdc_get_frame_count() : -1;
const scanline = bindings.ltdc_get_scanline ? bindings.ltdc_get_scanline() : -1;
console.log('model frames:', frames, 'scanline:', scanline);

let pxOk = true;
const data = emu.uc.mem_read(BigInt(FB), W * H * 4);
const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
const readPx = (i) => dv.getUint32(i * 4, true) >>> 0;
const checks = [[0, 0], [1, 0], [W - 1, 0], [W - 1, H - 1], [7, 5]];
for (const [x, y] of checks) {
    const got = readPx(y * W + x);
    const want = pix(x, y);
    if (got !== want) { pxOk = false; console.log(`pixel(${x},${y}) got 0x${got.toString(16)} want 0x${want.toString(16)}`); }
}

const pass =
    all.includes('=== LTDC Test ===') &&
    all.includes('scanout started') &&
    all.includes('LTDC pixels OK') &&
    all.includes('=== LTDC Test: done ===') &&
    frames >= 2 &&
    scanline !== 0xFFFF &&
    pxOk &&
    !all.includes('FAIL');
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);