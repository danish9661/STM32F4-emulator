// Verifies EXTI0 (PA0 rising edge) with the interrupt pump: the driver raises
// PA0 via gpio_set_input; the model's EXTI tick detects the edge, pends IRQ6,
// the pump runs the guest EXTI0_IRQHandler, and the firmware's wait loop sees
// exti0_count increment. Two edges, one at a time.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const fw = new Uint8Array(readFileSync(new URL('../exti_test/exti_test.bin', import.meta.url)));
const emu = await createEmulator({ firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes, enable_irqs: true });

const bootUart = () => emu.drainUart();

let uart = '';
let raising = false;
let done = false;
let raised = 0;
for (let i = 0; i < 4000 && !done; i++) {
    emu.step(50000);
    uart += bootUart();
    if (!raising && uart.includes('waiting for PA0 rising edge')) {
        raising = true;
        bindings.gpio_set_input(0, 0, true);  // PA0 rising edge
        raised++;
    } else if (raising && raised === 1 && uart.includes('waiting for PA0 2nd edge')) {
        bindings.gpio_set_input(0, 0, false); // back low (edge must be fresh)
        emu.step(1000);                       // let the tick observe the low
        bindings.gpio_set_input(0, 0, true);  // second rising edge
        raised++;
    }
    if (uart.includes('EXTI TEST DONE')) done = true;
}

const ok = done && !uart.includes('FAIL ') && raised === 2;
console.log(ok ? 'PASS' : 'FAIL');
console.log(uart);
emu.close();
process.exit(ok ? 0 : 1);
