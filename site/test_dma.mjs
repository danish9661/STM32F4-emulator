// Verifies the DMA2 memcopy path (comprehensive_test §6): NDTR cleared +
// IRQ56 ISR executed, plus the chunked periph-side DMA helpers already
// probed directly. Runs with the interrupt pump on (needed for irq_wait).
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const fw = new Uint8Array(readFileSync(new URL('../comprehensive_test/comprehensive_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes, enable_irqs: true });
let uart = '';
let fail = false;
const MAX_STEPS = 400;
for (let i = 0; i < MAX_STEPS && !fail; i++) {
    emu.step(40000);
    uart += emu.drainUart();
    if (uart.includes('FAIL:') || uart.includes('=== FAILED')) fail = true;
}
const dmaOk = uart.includes('PASS DMA2 NDTR=0') && uart.includes('PASS DMA2 IRQ56 ISR executed');
const result = uart.match(/PASS: ([0-9A-F]+)/);
console.log('DMA checks:', dmaOk ? 'PASS' : 'FAIL');
console.log('summary:', result ? 'PASS: ' + result[1] : '(none yet)');
console.log('uart tail:', JSON.stringify(uart.slice(-300)));
emu.close();
process.exit(dmaOk ? 0 : 1);
