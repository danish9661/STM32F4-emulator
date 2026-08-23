// Verifies the window-watchdog (WWDG) end-to-end path: the firmware starts the
// WWDG, pets it while "alive", then stops petting so the counter underflows.
// The model requests a reset; the emulator reboots the guest to the reset
// vector, and the firmware detects the WWDG reset cause (RCC->CSR WWDGRSTF).
// Usage: node site/test_wwdg.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../wwdg_demo/wwdg_demo.bin', import.meta.url)));

function fail(msg) { console.error('WDOG-WW FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    const markers = [
        '=== WWDG Demo ===',
        'WWDG started (pet every 2ms)',
        'alive 0',
        'alive 4',
        'stopping pet -> WWDG should reset',
        'WWDG reset detected',
    ];
    let uart = '';
    for (let i = 0; i < 400; i++) {
        emu.step(100000);
        uart += emu.drainUart().toString();
        if (uart.includes('WWDG reset detected')) break;
    }
    const missing = markers.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + JSON.stringify(missing) + '\n--- uart ---\n' + uart);
    console.log('WDOG-WW PASS');
})();
