// Node harness for the UART echo demo (echo_test, UART4 polling).
// Asserts: boot banner, and bytes injected via sendUartTo(UART4) come back
// out of the UART — the same path the console Send box drives.
// Usage: node site/test_uart_echo.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../firmware/echo_test/build/echo_test.ino.bin', import.meta.url)));

const UART4 = 0x40004C00;
const emu = await createEmulator({ firmware, bindings, svdXml, wasmInit: wasmBytes });

let out = '';
for (let i = 0; i < 100 && !out.includes('Echo ready'); i++) {
    emu.step(200000);
    out += emu.drainUart();
}
const booted = out.includes('Echo ready');
// Drive the firmware exactly like the console Send box does (rxPort auto
// resolves to UART4 for echo_test): raw bytes, no terminator needed.
emu.sendUartTo(UART4, new TextEncoder().encode('hello'));
for (let i = 0; i < 100; i++) {
    emu.step(200000);
    out += emu.drainUart();
}
// The banner ends 'Echo ready\n', so a live echo shows up as '\nhello' —
// the console's own '> hello' input-echo line is '\n> hello' and cannot
// match this (same convention as the browser smoke's sendExpect).
const echoed = out.includes('\nhello');
console.log(`booted=${booted} echoed=${echoed}`);
console.log('uart tail:', JSON.stringify(out.replace(/\r/g, '').split('\n').filter(Boolean).slice(-4).join(' | ')));
const pass = booted && echoed;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
