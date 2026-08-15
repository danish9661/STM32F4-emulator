// Node harness for DCMI, driven by GUEST firmware (dcmi_test/dcmi_test.bin)
// rather than by poking the MMIO bus.
//
// The camera is external hardware, so this file IS the sensor: a 2x2 frame
// (fits the 4-deep FIFO, so a polling drain gets it intact) until the guest
// prints PHASE2, then an 8x4 frame that must overrun that same polling drain
// and raise OVR — which is what real silicon does, and why capture drivers
// use DMA.
// Usage: node site/test_dcmi.mjs   (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../dcmi_test/dcmi_test.bin', import.meta.url)));

const SMALL = { w: 2, h: 2, px: new Uint8Array([0x11, 0x22, 0x33, 0x44]) };
const BIG = { w: 8, h: 4, px: Uint8Array.from({ length: 32 }, (_, i) => i + 1) };

let phase = 1;
let frames = 0;
const camera = {
    get width() { return phase === 1 ? SMALL.w : BIG.w; },
    get height() { return phase === 1 ? SMALL.h : BIG.h; },
    // Re-fed every step: the controller only reloads on a CAPTURE rising
    // edge, so handing it the same frame repeatedly is what a free-running
    // sensor looks like.
    frame() { frames++; return phase === 1 ? SMALL.px : BIG.px; },
};

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { camera },
});

let uart = '';
for (let i = 0; i < 800 && !uart.includes('DCMI Test: '); i++) {
    emu.step(100000);
    uart += emu.drainUart();
    // Swap the sensor to the oversized frame once the guest asks for it.
    if (phase === 1 && uart.includes('PHASE2')) phase = 2;
}
const all = uart.replace(/\r/g, '');
console.log('uart:', all.split('\n').filter(Boolean).join(' | '));
console.log(`camera: phase=${phase} framesFed=${frames}`);

const pass =
    all.includes('=== DCMI Test: done ===') &&
    !all.includes('FAIL') &&
    all.includes('DCMI pixels OK') &&
    all.includes('DCMI ovr OK') &&
    all.includes('px=11223344') &&
    phase === 2;

console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
