// Verifies TIM input capture end-to-end: the firmware configures TIM3 CH1 as
// input capture, and the driver injects capture edges (no external signal in
// the emulator). Each injected edge latches the live counter into CCR1 and
// sets CC1IF; the firmware reads it and prints "cap=N". Exercises the
// previously-unmodeled TIM input-capture path.
// Usage: node site/test_tim_capture.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../tim_capture_demo/tim_capture_demo.bin', import.meta.url)));

function fail(msg) { console.error('TIMCAP FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
    const markers = ['=== TIM Input Capture Demo ===', 'TIM capture ready', 'cap='];

    let uart = '';
    let captures = 0;
    for (let i = 0; i < 200; i++) {
        emu.step(100000);
        if (i % 2 === 0) emu.timInjectCapture('TIM3', 0);
        const chunk = emu.drainUart().toString();
        uart += chunk;
        // Count distinct capture lines.
        for (const line of chunk.split('\r\n')) {
            if (line.startsWith('cap=')) captures++;
        }
        if (uart.includes('done')) break;
    }
    const missing = markers.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + JSON.stringify(missing) + '\n--- uart ---\n' + uart);
    if (captures < 2) fail('expected >=2 distinct captures, got ' + captures + '\n--- uart ---\n' + uart);
    console.log('TIMCAP PASS (' + captures + ' capture events)');
})().catch((e) => fail(e.stack || String(e)));
