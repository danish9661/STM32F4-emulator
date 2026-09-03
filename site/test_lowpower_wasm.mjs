// Verifies low-power (WFI/STOP) on the wasm CPU backend: the
// deep_sleep_demo firmware arms the RTC alarm, enters STOP via WFI, and the
// Rust core halts (sleeping) until the inline-delivered RTC alarm ISR wakes
// it (PWR->CSR WUF set). Mirrors test_lowpower.mjs (Unicorn backend).
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../deep_sleep_demo/deep_sleep_demo.bin', import.meta.url)));

function fail(msg) { console.error('LOWPOWER-WASM FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({
        firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
        lowpower: true, cpu_backend: 'wasm',
    });

    let uart = '';
    for (let i = 0; i < 400 && !uart.includes('WOKE FROM STOP'); i++) {
        const r = emu.step(50000);
        uart += emu.drainUart().toString();
        if (r.stopped) fail('stopped at pc=0x' + r.pc.toString(16) + ' fault=' + JSON.stringify(emu.faultInfo()));
    }

    const need = ['=== Deep Sleep Demo ===', 'entering STOP', 'WOKE FROM STOP', 'Wakeup flag (WUF) set', 'alive: blinking'];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);

    console.log('LOWPOWER-WASM PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
