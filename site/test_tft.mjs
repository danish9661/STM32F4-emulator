// Node harness for the ILI9341 TFT over SPI (tft_test firmware).
// Asserts: boot + init + fill markers, and the JS-parsed 240x320 RGB565
// framebuffer quadrant colors (red/green/blue/white).
// Usage: node site/test_tft.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../tft_test/tft_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { tft: { spi: 'SPI2', cs: 'PB12', dc: 'PB11' } },
});

const uart = [];
for (let i = 0; i < 400; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    if (uart.join('').includes('TFT fill done') && emu.tft.frame() > 300) break;
}
const all = uart.join('');
console.log('uart:', all.replace(/\r/g, '').split('\n').filter(Boolean).join(' | '));

const px = (x, y) => {
    const off = (y * 240 + x) * 2;
    return (emu.tft.fb[off] << 8) | emu.tft.fb[off + 1];
};
const hex = (v) => '0x' + v.toString(16).padStart(4, '0');
const red = px(10, 10), green = px(230, 10), blue = px(10, 310), white = px(230, 310);
console.log(`tftFrame=${emu.tft.frame()} px(10,10)=${hex(red)} px(230,10)=${hex(green)} px(10,310)=${hex(blue)} px(230,310)=${hex(white)}`);

const pass =
    all.includes('TFT ILI9341 test') &&
    all.includes('TFT init done') &&
    all.includes('TFT fill done') &&
    emu.tft.frame() >= 319 &&           // the fill finished (320 rows)
    red === 0xF800 && green === 0x07E0 &&
    blue === 0x001F && white === 0xFFFF;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
