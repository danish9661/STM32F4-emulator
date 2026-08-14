// Node harness for the I2cRegisterDevice component (site/components.js)
// vs rtc_test's DS3231 register file — proves the generic component reads
// the same real, firmware-written I2C register-file state as the built-in
// emu.rtc decoder (see site/test_rtc.mjs for that path).
// Usage: node site/test_component_i2cregfile.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { I2cRegisterDevice } from './components.js';

const require = createRequire(import.meta.url);
const unicorn = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../rtc_test/rtc_test.bin', import.meta.url)));

const init = new Uint8Array(20);
init.set([0x30, 0x45, 0x10, 0x03, 0x15, 0x07, 0x26]); // 10:45:30 dow3 15/07/26
init[0x11] = 0x1B; // +27 C
init[0x12] = 0x80; // +0.50 C fraction

const emu = await createEmulator({
    firmware, bindings, unicorn, svdXml, wasmInit: wasmBytes,
    ext_devices: { rtc: { i2c: 'I2C1', addr: 0x68, init } },
});

let uart = '';
for (let i = 0; i < 300; i++) {
    emu.step(100000);
    uart += emu.drainUart();
    if (uart.includes('RTC test done')) break;
}
const regs = new I2cRegisterDevice(emu, 'I2C1');
const sec = regs.get(0x00), min = regs.get(0x01), hour = regs.get(0x02);
emu.close();

const bcd2n = (v) => ((v >> 4) * 10) + (v & 0x0F);
const pass = uart.includes('RTC verify OK') && uart.includes('RTC test done') &&
    bcd2n(sec) === 30 && bcd2n(min) === 45 && bcd2n(hour) === 10;
console.log(`I2cRegisterDevice test: sec=${bcd2n(sec)} min=${bcd2n(min)} hour=${bcd2n(hour)}`);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
