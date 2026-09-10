// Node harness for the BlackPill F401CC blinky firmware.
// Asserts: boot banner, tick prints, and the emulated GPIOC ODR (PC13)
// LED toggles — through the F401 SVD map with 256K/64K sizes.
// Usage: node site/test_blinky_f401.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { boardFor } from './boards.js';

const board = boardFor('blinky_f401');
const svdXml = readFileSync(new URL('./vendor/' + board.svd, import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky_f401/blinky_f401.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes,
    flash_size: board.flash_size, ram_size: board.ram_size,
});

const uart = [];
let prevOdr = -1, toggles = 0;
for (let i = 0; i < 400 && toggles < 8; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    const odr = emu.read32(0x40020814) & 0x2000;
    if (prevOdr >= 0 && odr !== prevOdr) toggles++;
    prevOdr = odr;
}
const all = uart.join('');
const count = (s) => (all.split(s).length - 1);
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-6).join(' | ');
console.log(`board=${board.label} led_toggles=${toggles} ledOn=${count(' LED=ON')} ledOff=${count(' LED=OFF')}`);
console.log('uart tail:', tail);

const pass =
    all.includes('=== Blinky F401 ===') &&
    all.includes('No ethernet required') &&
    count('tick 0 LED=ON') === 1 &&
    count('tick 0 LED=OFF') === 1 &&
    count('tick 1 LED=ON') === 1 &&
    toggles >= 2;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
