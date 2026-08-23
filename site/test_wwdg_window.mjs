// Verifies the WWDG window feature end-to-end: with a window W=0x50, correct
// refreshes (counter <= W) keep the MCU alive, but a refresh while the counter
// is still above the window is a window violation that resets the MCU. On
// reboot the firmware detects the WWDG reset cause (RCC->CSR WWDGRSTF).
// Usage: node site/test_wwdg_window.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../wwdg_window_demo/wwdg_window_demo.bin', import.meta.url)));

function fail(msg) { console.error('WDOG-WW-WIN FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    const markers = [
        '=== WWDG Window Demo ===',
        'WWDG windowed (W=0x50), pet in window',
        'alive 0',
        'alive 4',
        'trigger window violation (refresh above W)',
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
    console.log('WDOG-WW-WIN PASS');
})().catch((e) => fail(e.stack || String(e)));
