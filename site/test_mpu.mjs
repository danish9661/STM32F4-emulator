// Node harness for the mpu_test firmware: enabling MPU_CTRL must halt the
// driver LOUDLY (modelHaltInfo names MPU) instead of running on unprotected.
// Pass = "MPU enabled" printed, emulator stopped, modelHalt mentions MPU,
// no CPU fault, and "MPU SPUN" never appears.
// Usage: node site/test_mpu.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('../monox/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../mpu_test/mpu_test.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, svdXml, wasmInit: wasmBytes });

const uart = [];
let stopped = false;
for (let i = 0; i < 400 && !stopped; i++) {
    const r = emu.step(100000);
    stopped = r.stopped;
    const chunk = emu.drainUart();
    if (chunk) uart.push(chunk);
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | ');
console.log('uart tail:', tail);
console.log('stopped:', stopped, 'modelHalt:', emu.modelHaltInfo());

const pass =
    all.includes('=== MPU Test ===') &&
    all.includes('MPU enabled') &&
    !all.includes('MPU SPUN') &&
    stopped === true &&
    (emu.modelHaltInfo() || '').includes('MPU') &&
    emu.faultInfo() == null;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
