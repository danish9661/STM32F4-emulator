// Node harness for the Button component (site/components.js) on PA0 vs
// exti_test's IRQ handler (pin-write / input-injection path).
// Usage: node site/test_component_button.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { Button } from './components.js';

const require = createRequire(import.meta.url);
const unicorn = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../exti_test/exti_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn, svdXml, wasmInit: wasmBytes, enable_irqs: true });

const btn = new Button(emu, 'A', 0, { activeLow: false }); // EXTI0 test wants a rising edge
let uart = '';
let raising = false, raised = 0, done = false;
for (let i = 0; i < 4000 && !done; i++) {
    emu.step(50000);
    uart += emu.drainUart();
    if (!raising && uart.includes('waiting for PA0 rising edge')) {
        raising = true;
        btn.press();
        raised++;
    } else if (raising && raised === 1 && uart.includes('waiting for PA0 2nd edge')) {
        btn.release();
        emu.step(1000); // let the tick observe the low before re-raising
        btn.press();
        raised++;
    }
    if (uart.includes('EXTI TEST DONE')) done = true;
}
emu.close();

const pass = done && !uart.includes('FAIL ') && raised === 2;
console.log(`Button test: done=${done} raised=${raised}`);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
