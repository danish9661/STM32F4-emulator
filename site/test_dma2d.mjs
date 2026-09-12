// DMA2D Chrom-ART test (F429 only): R2M fill, M2M copy, PFC convert,
// alpha blend, and line-offset stride, all completing through TCIF+IRQ56.
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f429.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../dma2d_test/dma2d_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes,
    flash_size: 0x200000, ram_size: 0x40000, enable_irqs: true,
});

let uart = '';
let done = false;
for (let i = 0; i < 600 && !done; i++) {
    emu.step(100000);
    uart += emu.drainUart();
    if (uart.includes('=== DMA2D Test: done ===')) done = true;
}
const want = ['DMA2D R2M OK', 'DMA2D M2M OK', 'DMA2D PFC OK', 'DMA2D blend OK',
    'DMA2D stride OK', '=== DMA2D Test: done ==='];
const missing = want.filter((m) => !uart.includes(m));
const ok = done && missing.length === 0 && !uart.includes('TIMEOUT') && !uart.includes('FAIL ') && !emu.faultInfo();
console.log(ok ? 'PASS' : 'FAIL');
if (!ok) {
    if (missing.length) console.log('missing:', missing.join(', '));
    console.log(uart.replace(/\r/g, '').split('\n').filter(Boolean).slice(-10).join('\n'));
}
emu.close();
process.exit(ok ? 0 : 1);
