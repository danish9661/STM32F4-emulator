// Node harness for the Pwm component (site/components.js) on TIM2 CH1 vs
// buzzer_test's known melody (C4 262 -> C5 523 -> rest), reading the timer
// via the generic register-decode reader instead of the built-in buzzer
// ext_device (see site/test_buzzer.mjs for that path).
// Usage: node site/test_component_pwm.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { Pwm } from './components.js';

const require = createRequire(import.meta.url);
const unicorn = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../buzzer_test/buzzer_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn, svdXml, wasmInit: wasmBytes });

const pwm = new Pwm(emu, 'TIM2', 1);
let uart = '';
const freqs = [];
let last = -1;
for (let i = 0; i < 2000; i++) {
    emu.step(100000);
    uart += emu.drainUart();
    const f = Math.round(pwm.freq);
    if (f !== last) { freqs.push(f); last = f; }
    if (uart.includes('Buzzer done')) break;
}
emu.close();

const has = (f, tol) => freqs.some((v) => Math.abs(v - f) <= tol);
const pass = uart.includes('Buzzer test') && uart.includes('Buzzer done') &&
    has(262, 3) && has(523, 3) && freqs.includes(0);
console.log(`Pwm test: freqs=[${freqs.join(',')}]`);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
