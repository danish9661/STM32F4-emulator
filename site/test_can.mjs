// Node harness for the CAN bus arbitration firmware.
// Asserts the full flow: loopback (LBKM) TX echo, then two-node arbitration
// where the lower-ID frame wins and both nodes receive both frames.
// Usage: node site/test_can.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../can_test/can_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });

const uart = [];
for (let i = 0; i < 600 && !uart.join('').includes('=== CAN Test: done'); i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | ');
console.log('uart tail:', tail);

const pass =
    all.includes('=== CAN Test ===') &&
    all.includes('CAN loopback OK: id=0x123 data=CANLOOP!') &&
    all.includes('both TX done') &&
    all.includes('CAN arbitration OK') &&
    all.includes('=== CAN Test: done ===') &&
    !all.includes('FAIL');
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);