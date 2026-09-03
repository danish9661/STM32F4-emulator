// Smoke test for the WASM-native CPU backend (cpu:'wasm').
// Boots blinky without Unicorn: no static imports of unicorn needed.
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: null, svdXml, wasmInit: wasmBytes, cpu_backend: 'wasm',
});

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
const ok = all.includes('=== Blinky ===') && all.includes('tick 0') && toggles >= 2;
console.log(`toggles=${toggles} banner=${all.includes('=== Blinky ===')}`);
console.log(ok ? 'WASM-CPU PASS' : 'WASM-CPU FAIL');
console.log('tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-3).join(' | '));
process.exit(ok ? 0 : 1);
