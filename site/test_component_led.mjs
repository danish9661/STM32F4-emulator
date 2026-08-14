// Node harness for the LED component (site/components.js) on PA5 vs
// blinky's UART tick markers (pin-read path via emu.pin()/watchPin()).
// Usage: node site/test_component_led.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { LED } from './components.js';

const require = createRequire(import.meta.url);
const unicorn = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn, svdXml, wasmInit: wasmBytes });

const led = new LED(emu, 'A', 5);
const ledStates = [];
led.watch((v) => ledStates.push(v));

let uart = '';
for (let i = 0; i < 400 && ledStates.length < 4; i++) {
    emu.step(100000);
    uart += emu.drainUart();
}
emu.close();

// blinky prints "tick N LED=ON" then "tick N LED=OFF" for each N; the
// watched LED should report the same ON/OFF sequence via the public API.
const uartSeq = [...uart.matchAll(/LED=(ON|OFF)/g)].map((m) => m[1] === 'ON');
const pass = ledStates.length >= 4 &&
    uartSeq.length >= ledStates.length &&
    ledStates.every((v, i) => v === uartSeq[i]);
console.log(`LED test: watched=[${ledStates.join(',')}] uart=[${uartSeq.slice(0, ledStates.length).join(',')}]`);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
