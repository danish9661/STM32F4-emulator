// Node harness for the F407VE/ZE black-board configs (512K flash, 128K
// RAM — same F407 die, bigger packages). Runs the stock blinky.bin under
// the stm32f407ve sizes; asserts boot markers only (board LED pinout
// varies by vendor, so no GPIO assertion here).
// Usage: node site/test_blinky_f407ve.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { boardFor } from './boards.js';

const runOne = async (preset) => {
    const board = boardFor(preset);
    // Both VE/ZE presets share the stock blinky binary (markers only).
    const fwBytes = readFileSync(new URL('../blinky/blinky.bin', import.meta.url));
    const svdXml = readFileSync(new URL('./vendor/' + board.svd, import.meta.url), 'utf8');
    const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
    const emu = await createEmulator({
        firmware: new Uint8Array(fwBytes), bindings, svdXml, wasmInit: wasmBytes,
        flash_size: board.flash_size, ram_size: board.ram_size,
    });
    let uart = '';
    for (let i = 0; i < 200; i++) {
        emu.step(100000);
        uart += emu.drainUart().toString();
    }
    emu.close();
    const ok = uart.includes('=== Blinky ===') && uart.includes('No ethernet required') && uart.includes('tick 0 LED=ON');
    return { preset, board: board.label, ok };
};

const results = [];
for (const preset of ['blinky_f407ve', 'blinky_f407ze']) results.push(await runOne(preset));
for (const r of results) console.log(`${r.preset} on ${r.board}: ${r.ok ? 'ok' : 'MISSING MARKERS'}`);
const pass = results.every((r) => r.ok);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
