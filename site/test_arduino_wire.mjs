// Arduino Wire IRQ-driver regression test (2026-10-06 I2C seat fix).
// Runs a real Arduino sketch (firmware/arduino_wire, Disco F407VG build:
// I2C1 + USART2) exercising the HAL IT state machine end to end — manual
// 2-byte master-TX to the OLED tap at 0x3C (the reported hang shape:
// endTransmission() never returned) plus repeated-START reads (1/2/6
// bytes) from the regfile at 0x50 (POS/BTF tails, streaming). Pass =
// every transfer completes with exact bytes and the observer spec on
// 0x3C sees the bus events (broadcast fan-out). Before the fix this
// printed endTransmission()=4 / requestFrom()=0 and hung.
// Usage: node site/test_arduino_wire.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const fw = new Uint8Array(readFileSync(new URL('../firmware/arduino_wire/build-disco_f407vg/arduino_wire.ino.bin', import.meta.url)));

// Regfile pattern: reg N reads back 0xA0+N (asserted exactly below).
const regInit = Array.from({ length: 16 }, (_, i) => 0xA0 + i);
let starts = 0, stops = 0;
const written = [];
const emu = await createEmulator({
    firmware: fw, bindings, svdXml, wasmInit: wasmBytes,
    flash_size: 0x100000, ram_size: 0x30000,
    uart_addr: 0x40004400, enable_irqs: true,
    ext_devices: {
        oled: { i2c: 'I2C1', addr: 0x3C },
        regfile: [{ peripheral: 'I2C1', address: 0x50, size: 16, init: regInit }],
        i2cDevices: [{
            peripheral: 'I2C1', address: 0x3C,
            handler: (events) => {
                for (const v of events) {
                    if (v & 0x80000000) { if (v & 0x40000000) starts++; else stops++; }
                    else if (!(v & 0x40000000)) written.push(v & 0xFF);
                }
            },
        }],
    },
});

let uart = '';
for (let i = 0; i < 1500 && !uart.includes('WIRE LOOP DONE'); i++) {
    const r = emu.step(200000);
    uart += emu.drainUart().toString();
    if (r.stopped || emu.faultInfo()) break;
}
const fault = emu.faultInfo();
emu.close();
const t = uart.replace(/\r/g, '');
const checks = [
    ['wire begun', t.includes('WIRE BEGIN') && t.includes('WIRE DONE')],
    ['manual TX ok (TX1 0)', /TX1 0\b/.test(t)],
    ['pointer writes ok (W1/W2/W3 0)', /W1 0\b/.test(t) && /W2 0\b/.test(t) && /W3 0\b/.test(t)],
    ['1-byte read exact (N1 1, B0 A0)', /N1 1\b/.test(t) && /B0 A0\b/.test(t)],
    ['2-byte read exact (N2 2, B1 A2 A3)', /N2 2\b/.test(t) && /B1 A2 A3\b/.test(t)],
    ['6-byte read exact (N6 6, A0..A5)', /N6 6\b/.test(t) && /B6 A0 A1 A2 A3 A4 A5\b/.test(t)],
    ['loop done', t.includes('WIRE LOOP DONE')],
    ['observer saw START+2 writes+STOP', starts >= 1 && written.join(',') === '0,174' && stops >= 1],
    ['no fault', !fault],
];
let fail = 0;
for (const [name, ok] of checks) {
    console.log(`${ok ? 'ok' : 'FAIL'}: wire: ${name}`);
    if (!ok) fail++;
}
if (fail) console.log('--- uart tail ---\n' + t.split('\n').filter(Boolean).slice(-12).join('\n'));
console.log(fail ? 'WIRE FAIL' : 'WIRE PASS');
process.exit(fail ? 1 : 0);
