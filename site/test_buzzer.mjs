// Node harness for the TIM2 PWM buzzer (buzzer_test firmware).
// Asserts: boot + melody markers, and the JS-observed TIM2 frequency
// sequence (C4 262 -> C5 523 -> rest 0 ...).
// Usage: node site/test_buzzer.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../buzzer_test/buzzer_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { buzzer: { tim: 'TIM2' } },
});

const uart = [];
const freqs = [];
let last = -1;
for (let i = 0; i < 2000; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    const f = Math.round(emu.buzzer.freq);
    if (f !== last) { freqs.push(f); last = f; }
    if (uart.join('').includes('Buzzer done')) break;
}
const all = uart.join('');
console.log('uart:', all.replace(/\r/g, '').split('\n').filter(Boolean).join(' | '));
console.log('freqs:', freqs.join(','));

const has = (f, tol) => freqs.some((v) => Math.abs(v - f) <= tol);
const pass =
    all.includes('Buzzer test') &&
    all.includes('Buzzer melody') &&
    all.includes('BUZZ 262 Hz') &&
    all.includes('BUZZ 523 Hz') &&
    all.includes('Buzzer done') &&
    has(262, 3) && has(523, 3) && freqs.includes(0);
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
