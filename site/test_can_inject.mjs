// Verifies the CAN host-injection API: the emulator delivers a frame injected
// via emu.canInject() into the guest's CAN RX FIFO exactly as if another node
// on the wire sent it. The can_host_rx firmware polls its FIFO and prints each
// received frame. Complements site/test_can.mjs / site/test_candemo.mjs.
// Usage: node site/test_can_inject.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../can_host_rx/can_host_rx.bin', import.meta.url)));

function fail(msg) { console.error('CANINJ FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    let uart = '';
    for (let i = 0; i < 400; i++) {
        emu.step(50000);
        uart += emu.drainUart().toString();
        if (uart.includes('CAN RX ready')) break;
    }
    if (!uart.includes('=== CAN Host RX ===') || !uart.includes('CAN RX ready')) {
        fail('firmware did not reach RX-ready state:\n' + uart);
    }
    // Host injects a frame (id 0x123, "HELLO!!!").
    const data = new Uint8Array([0x48, 0x45, 0x4C, 0x4C, 0x4F, 0x21, 0x21, 0x21]);
    emu.canInject(0x123, 8, data);
    for (let i = 0; i < 400; i++) {
        emu.step(50000);
        uart += emu.drainUart().toString();
        if (uart.includes('data=HELLO!!!')) break;
    }
    const need = ['=== CAN Host RX ===', 'CAN RX ready (pass-all filter)', 'RX id=0x00000123 data=HELLO!!!'];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);
    console.log('CANINJ PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
