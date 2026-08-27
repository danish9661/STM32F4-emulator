// Node harness for the qspi_test firmware. Validates the modeled QUADSPI
// peripheral end-to-end: a flash image is registered with the driver before
// boot, the firmware writes known words and reads them back, and we assert
// the round-trip succeeds ("QSPI OK").
// Usage: node site/test_qspi.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../stm32-periph-wasm/pkg/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('../stm32-periph-wasm/pkg/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../monox/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../stm32-periph-wasm/pkg/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../qspi_test/qspi_test.bin', import.meta.url)));

// Register a 256-byte flash backend BEFORE the model constructs its QSPI
// peripheral (it binds the flash at init time).
bindings.qspi_register_flash('QUADSPI', new Uint8Array(256));

const emu = await createEmulator({
    firmware,
    bindings,
    unicorn: unicornFactory,
    svdXml,
    wasmInit: wasmBytes,
});

const uart = [];
let done = false;
for (let i = 0; i < 400 && !done; i++) {
    emu.step(100000);
    const chunk = emu.drainUart();
    if (chunk) uart.push(chunk);
    if (chunk && (chunk.includes('QSPI Test done') || chunk.includes('QSPI FAIL'))) {
        done = true;
    }
}
const all = uart.join('');
const tail = all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | ');
console.log('uart tail:', tail);

const pass =
    all.includes('=== QSPI Test ===') &&
    all.includes('QSPI OK') &&
    all.includes('QSPI Test done') &&
    !all.includes('QSPI FAIL') &&
    all.includes('wrote DEADBEEF read DEADBEEF');

console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
