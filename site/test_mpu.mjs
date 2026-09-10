// Node harness for the mpu_test firmware: MPU enforcement end-to-end.
// Regions programmed, then legal accesses pass while violations take
// MemManage with exact MMFSR/MMFAR (no-access write, XN exec, unprivileged
// SRAM-priv write, unprivileged PPB read), and the firmware COMPLETES.
// Pass = all phase markers, fault counts exact, no halt, no CPU fault.
// Usage: node site/test_mpu.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('../monox/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../mpu_test/mpu_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes, enable_irqs: true,
});

const uart = [];
let done = false;
for (let i = 0; i < 600 && !done; i++) {
    const r = emu.step(100000);
    if (r.stopped) break;
    const chunk = emu.drainUart();
    if (chunk) uart.push(chunk);
    if (chunk && chunk.includes('MPU done')) done = true;
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-14).join(' | ');
console.log('uart tail:', tail);

const want = ['=== MPU Test ===', 'REGIONS OK', 'MPU enabled', 'LEGAL OK',
    'NOACC 00000001 PASS', 'XNEXEC 00000001 PASS', 'UPRIV-RO OK',
    'UPRIV-W 00000001 PASS', 'UPRIV-PPB 00000001 PASS', 'UPRIV-FULL OK',
    'MPU all PASS', 'MPU done'];
const missing = want.filter((m) => !all.includes(m));
if (missing.length) console.log('missing:', missing.join(' | '));
const pass = missing.length === 0 && !all.includes('FAIL') && emu.faultInfo() == null;
if (emu.faultInfo()) console.log('fault:', JSON.stringify(emu.faultInfo()));
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
