// Verifies the new FLASH program/erase emulation: the firmware unlocks the
// flash, erases sector 5 (0x08020000), programs 4 words, re-erases, relocks,
// and asserts every step. The JS driver applies program writes and erase
// fills to guest memory via the model's flash_is_programming/flash_take_erase.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const fw = new Uint8Array(readFileSync(new URL('../flash_test/flash_test.bin', import.meta.url)));
const emu = await createEmulator({ firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes });

let uart = '';
let done = false;
for (let i = 0; i < 4000 && !done; i++) {
    emu.step(50000);
    uart += emu.drainUart();
    if (uart.includes('FLASH TEST DONE')) done = true;
}

const ok = done && !uart.includes('FAIL ');
console.log(ok ? 'PASS' : 'FAIL');
console.log(uart);
emu.close();
process.exit(ok ? 0 : 1);
