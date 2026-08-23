// Verifies the CAN bus demo firmware: single-controller loopback TX/RX and
// two-node arbitration where the lower arbitration ID wins and both nodes
// receive both frames. Complements site/test_can.mjs (which checks the raw
// model arbitration mechanics).
// Usage: node site/test_candemo.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../can_demo/can_demo.bin', import.meta.url)));

function fail(msg) { console.error('CANDEMO FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    let uart = '';
    let exited = false;
    for (let i = 0; i < 400 && !exited; i++) {
        const r = emu.step(50000);
        uart += emu.drainUart().toString();
        if (r.stopped) exited = true;
        if (uart.includes('=== CAN Demo: done ===')) break;
    }
    const need = ['=== CAN Demo ===', 'CAN loopback OK: id=0x123 data=CANLOOP!', 'CAN arbitration OK', '=== CAN Demo: done ==='];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);
    console.log('CANDEMO PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
