// Verifies the low-power STANDBY (PDDS=1) model: the standby_demo firmware
// sets PDDS + SLEEPDEEP, enters WFI, and the emulator must take the standby
// path (SBF set on entry) rather than the stop path, then wake on the RTC
// alarm with WUF set and SBF still set. Exercises the emulator's
// PDDS-sample + pwr_enter_standby/pwr_wakeup_standby path (opts.lowpower).
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../firmware/standby_demo/standby_demo.bin', import.meta.url)));

function fail(msg) { console.error('STANDBY FAIL: ' + msg); process.exit(1); }

(async () => {
    const emu = await createEmulator({
        firmware, bindings, svdXml, wasmInit: wasmBytes,
        lowpower: true,
    });

    let uart = '';
    let exited = false;
    for (let i = 0; i < 400 && !exited; i++) {
        const r = emu.step(50000);
        uart += emu.drainUart().toString();
        if (r.stopped) exited = true;
        if (uart.includes('WOKE FROM STANDBY')) break;
    }

    const need = ['=== Standby Demo ===', 'entering STANDBY', 'WOKE FROM STANDBY', 'Wakeup flag (WUF) set', 'Standby flag (SBF) set', 'alive: blinking'];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);

    console.log('STANDBY PASS');
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
