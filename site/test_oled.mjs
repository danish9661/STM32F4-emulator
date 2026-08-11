// Node harness for the SSD1306 OLED over I2C (oled_test firmware).
// Asserts: boot + draw markers, and the JS-parsed framebuffer content
// (text page on, empty rows, solid bottom bar on page 7).
// Usage: node site/test_oled.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../oled_test/oled_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { oled: { i2c: 'I2C1', addr: 0x3C } },
});

const uart = [];
for (let i = 0; i < 300; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    if (uart.join('').includes('OLED draw done')) break;
}
const all = uart.join('');
console.log('uart:', all.replace(/\r/g, '').split('\n').filter(Boolean).join(' | '));

const fb = emu.oled.fb;
const onCount = () => { let n = 0; for (const v of fb) if (v) n++; return n; };
// page p = rows p*8 .. p*8+7, cols 0..127 (row-major fb)
const pagePixels = (p, x0 = 0, x1 = 127) => {
    let n = 0;
    for (let bit = 0; bit < 8; bit++) for (let x = x0; x <= x1; x++) if (fb[(p * 8 + bit) * 128 + x]) n++;
    return n;
};
const textPixels = pagePixels(0, 4, 80);      // "F407 OLED" on page 0
const barPixels = pagePixels(7);              // solid bar on page 7
let emptyOK = true;
for (const p of [1, 2, 4, 6]) if (pagePixels(p)) emptyOK = false;
console.log(`textPixels(page0)=${textPixels} barPixels(page7)=${barPixels} onTotal=${onCount()} emptyOK=${emptyOK}`);

const pass =
    all.includes('OLED test') &&
    all.includes('OLED init done') &&
    all.includes('OLED draw done') &&
    textPixels > 30 &&          // "F407 OLED" 5x7 glyphs are lit
    barPixels === 128 * 8 &&     // full-width bottom bar (8 rows of page 7)
    emptyOK;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
