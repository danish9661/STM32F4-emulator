// Verifies the low-power (WFI/STOP) model: the deep_sleep_demo firmware arms
// the RTC alarm, enters STOP via WFI, and the emulator halts the core until
// the RTC alarm wakes it (PWR->CSR WUF set). Exercises the emulator's sleep
// trap + virtual-clock wakeup path (opts.lowpower).
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../deep_sleep_demo/deep_sleep_demo.bin', import.meta.url)));

function fail(msg) { console.error('LOWPOWER FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({
        firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
        lowpower: true,
    });

    let uart = '';
    let exited = false;
    for (let i = 0; i < 400 && !exited; i++) {
        const r = emu.step(50000);
        uart += emu.drainUart().toString();
        if (r.stopped) exited = true;
        if (uart.includes('WOKE FROM STOP')) break;
    }

    const need = ['=== Deep Sleep Demo ===', 'entering STOP', 'WOKE FROM STOP', 'Wakeup flag (WUF) set', 'alive: blinking'];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);

    console.log('LOWPOWER PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
