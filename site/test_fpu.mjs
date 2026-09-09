// Node harness for the fpu_test firmware. Boots real GCC hard-float
// (fpv4-sp-d16) output through the Rust CPU + peripheral model and asserts
// every check line: CPACR enable, add/sub/mul/div/sqrt, fused FMA (the
// 0x28800000 result only appears with single-rounding fusion), compares
// (vcmp/vcmpe path) and conversions (vcvt path).
// Usage: node site/test_fpu.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('../monox/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../fpu_test/fpu_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, svdXml, wasmInit: wasmBytes });

const uart = [];
let done = false;
for (let i = 0; i < 400 && !done; i++) {
    emu.step(100000);
    const chunk = emu.drainUart();
    if (chunk) uart.push(chunk);
    if (chunk && chunk.includes('FPU done')) done = true;
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-14).join(' | ');
console.log('uart tail:', tail);

const want = ['=== FPU Test ===', 'CPACR ok', 'ADD 40700000 PASS', 'SUB C0600000 PASS',
    'MUL 40900000 PASS', 'DIV 40600000 PASS', 'SQRT 3FB504F3 PASS',
    'FMA 28800000 PASS', 'CMP OK', 'CVT OK', 'SPILL 43528000 PASS',
    'FPU all PASS', 'FPU done'];
const missing = want.filter((m) => !all.includes(m));
if (missing.length) console.log('missing:', missing.join(' | '));
const pass = missing.length === 0 && !all.includes('FAIL') && emu.faultInfo() == null;
if (emu.faultInfo()) console.log('fault:', JSON.stringify(emu.faultInfo()));
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
