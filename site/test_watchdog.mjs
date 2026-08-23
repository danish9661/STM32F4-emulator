// Verifies the watchdog (IWDG) end-to-end path: the firmware starts the
// independent watchdog, pets it while "alive", then stops petting so it
// expires. The model requests a reset; the emulator reboots the guest to the
// reset vector, and the firmware detects the IWDG reset cause in RCC->CSR.
// Complements the model (stm32-periph-wasm) which also implements WWDG.
// Usage: node site/test_watchdog.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../watchdog_demo/watchdog_demo.bin', import.meta.url)));

function fail(msg) { console.error('WDOG FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    let uart = '';
    const need = [
        '=== Watchdog Demo ===',
        'IWDG started (pet every 150ms, ~1s timeout)',
        'alive 0',
        'alive 4',
        'stopping pet -> watchdog should reset',
        'IWDG reset detected',
    ];
    // The MCU auto-reboots on watchdog expiry; run until the reset is detected.
    for (let i = 0; i < 300; i++) {
        emu.step(100000);
        uart += emu.drainUart().toString();
        if (uart.includes('IWDG reset detected')) break;
    }
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);
    console.log('WDOG PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
