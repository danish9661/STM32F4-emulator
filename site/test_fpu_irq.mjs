// Node harness for the fpu_irq_test firmware: lazy FP stacking across
// real SysTick IRQs. Main seeds S0-S3 + FPSCR=0 (FPCA sets); the SysTick
// handler does its own float work and dirties FPSCR. Pass = S0-S3 intact,
// FPSCR clean, IRQs observed — only possible with entry reserve +
// first-use stacking + full restore on return.
// Usage: node site/test_fpu_irq.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('../monox/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../fpu_irq_test/fpu_irq_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes, enable_irqs: true,
});

const uart = [];
let done = false;
for (let i = 0; i < 600 && !done; i++) {
    emu.step(100000);
    const chunk = emu.drainUart();
    if (chunk) uart.push(chunk);
    if (chunk && chunk.includes('FPU IRQ done')) done = true;
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-12).join(' | ');
console.log('uart tail:', tail);

const want = ['=== FPU IRQ Test ===', 'CPACR ok', 'S0 11111111 ok', 'S1 22222222 ok',
    'S2 33333333 ok', 'S3 44444444 ok', 'FPSCR 00000000 ok',
    'FPU IRQ all PASS', 'FPU IRQ done'];
const missing = want.filter((m) => !all.includes(m));
if (missing.length) console.log('missing:', missing.join(' | '));
const m = all.match(/IRQ count (\d+)/);
console.log('irq count:', m ? m[1] : '(none)');
const pass = missing.length === 0 && !all.includes('FAIL')
    && m && +m[1] >= 10 && emu.faultInfo() == null;
if (emu.faultInfo()) console.log('fault:', JSON.stringify(emu.faultInfo()));
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
