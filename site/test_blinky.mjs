// Node harness for the blinky (non-ethernet) firmware.
// Asserts: boot banner, tick prints, and the emulated GPIO ODR LED toggles.
// Usage: node site/test_blinky.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });

const uart = [];
let prevOdr = -1, toggles = 0;
for (let i = 0; i < 400 && toggles < 8; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    const odr = emu.read32(0x40020014) & 0x20;
    if (prevOdr >= 0 && odr !== prevOdr) toggles++;
    prevOdr = odr;
}
const all = uart.join('');
const count = (s) => (all.split(s).length - 1);
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-6).join(' | ');
console.log(`led_toggles=${toggles} ledOn=${count(' LED=ON')} ledOff=${count(' LED=OFF')}`);
console.log('uart tail:', tail);

const pass =
    all.includes('=== Blinky ===') &&
    all.includes('No ethernet required') &&
    count('tick 0 LED=ON') === 1 &&
    count('tick 0 LED=OFF') === 1 &&
    count('tick 1 LED=ON') === 1 &&
    toggles >= 2;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
