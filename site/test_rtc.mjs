// Node harness for the DS3231 RTC over I2C (rtc_test firmware).
// Asserts: boot markers, the guest-set BCD time decoded by the JS
// regfile reader (emu.rtc.time), the temperature decode (27.50 C from
// seed 0x1B/0x80), and the verify-OK marker.
// Usage: node site/test_rtc.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../rtc_test/rtc_test.bin', import.meta.url)));

// Register-file seed: BCD time regs 0x00-0x06 (sec/min/hr/dow/day/mon/yr),
// temp MSB/LSB 0x11/0x12. The guest overwrites the time regs via I2C; the
// temp regs stay at the seed (read-only on real silicon).
const init = new Uint8Array(20);
init.set([0x30, 0x45, 0x10, 0x03, 0x15, 0x07, 0x26]); // 10:45:30 dow3 15/07/26
init[0x11] = 0x1B;   // +27 C
init[0x12] = 0x80;   // +0.50 C fraction (bits 7-6 = 2 quarters)

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { rtc: { i2c: 'I2C1', addr: 0x68, init } },
});

const uart = [];
for (let i = 0; i < 300; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    if (uart.join('').includes('RTC test done')) break;
}
const all = uart.join('');
console.log('uart:', all.replace(/\r/g, '').split('\n').filter(Boolean).join(' | '));

const t = emu.rtc.time;
const temp = emu.rtc.temp;
console.log(`rtc.time=${JSON.stringify(t)} temp=${temp} change=${emu.rtc.change}`);

const pass =
    all.includes('RTC test') &&
    all.includes('RTC set done') &&
    all.includes('RTC read done') &&
    all.includes('RTC verify OK') &&
    all.includes('RTC time=10:45:30 DOW=3 15/07/26') &&
    all.includes('RTC temp=27.50') &&
    all.includes('RTC test done') &&
    !all.includes('RTC verify FAIL') &&
    t && t.sec === 30 && t.min === 45 && t.hour === 10 && t.dow === 3 &&
    t.day === 15 && t.mon === 7 && t.year === 26 &&
    temp === 27.5;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
