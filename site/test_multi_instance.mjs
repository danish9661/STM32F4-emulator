// Multi-instance harness: boots several firmwares SEQUENTIALLY in ONE process
// and asserts each one reaches its own UART marker.
//
// This is the regression test for the SYS OnceLock bug (stm32-periph-wasm/
// src/lib.rs): OnceLock::set() accepted only the first value, so every
// createEmulator() after the first silently kept running on instance 1's
// peripheral tree. Before the fix, run #2 onwards fail here.
//
// Usage: node site/test_multi_instance.mjs   (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const fw = (p) => new Uint8Array(readFileSync(new URL(p, import.meta.url)));

// Buzzer is the slow one: it needs ~2000 iterations of step(100000) to finish
// its melody. 400 looks exactly like a failure.
const CASES = {
    blinky: { firmware: '../blinky/blinky.bin', marker: 'LED=', iters: 300, ext_devices: {} },
    rtc: {
        firmware: '../rtc_test/rtc_test.bin', marker: 'RTC test done', iters: 300,
        ext_devices: { rtc: { i2c: 'I2C1', addr: 0x68, init: rtcSeed() } },
    },
    buzzer: {
        firmware: '../buzzer_test/buzzer_test.bin', marker: 'Buzzer done', iters: 2000,
        ext_devices: { buzzer: { tim: 'TIM2' } },
    },
};

function rtcSeed() {
    const init = new Uint8Array(20);
    init.set([0x30, 0x45, 0x10, 0x03, 0x15, 0x07, 0x26]);
    init[0x11] = 0x1B;
    init[0x12] = 0x80;
    return init;
}

async function run(name) {
    const c = CASES[name];
    const emu = await createEmulator({
        firmware: fw(c.firmware), bindings, unicorn: unicornFactory, svdXml,
        wasmInit: wasmBytes, ext_devices: c.ext_devices,
    });
    let uart = '';
    for (let i = 0; i < c.iters && !uart.includes(c.marker); i++) {
        emu.step(100000);
        uart += emu.drainUart();
    }
    emu.close();
    return { ok: uart.includes(c.marker), uart };
}

// Every order, plus a 7-instance run, all in this one process.
const ORDERS = [
    ['blinky', 'rtc', 'buzzer'],
    ['buzzer', 'blinky', 'rtc'],
    ['rtc', 'buzzer', 'blinky'],
    ['blinky', 'blinky', 'rtc', 'rtc', 'buzzer', 'buzzer', 'blinky'],
];

let pass = true;
for (const order of ORDERS) {
    const results = [];
    for (let i = 0; i < order.length; i++) {
        const r = await run(order[i]);
        results.push(`${i + 1}:${order[i]}=${r.ok ? 'ok' : 'MISSING-MARKER'}`);
        if (!r.ok) {
            pass = false;
            console.log(`  uart of failing instance:\n${r.uart.replace(/\r/g, '')}`);
        }
    }
    console.log(`[${order.length} instances] ${results.join(' ')}`);
}

console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
