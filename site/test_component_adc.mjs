// Node harness for the Potentiometer component (site/components.js) and
// the underlying adc_set_channel_value/adc_clear_channel_value wasm
// exports: drives ADC1 registers directly via bindings.periph_write/read
// (the same low-level path the CPU-driven MMIO hooks use — see
// emulator.js's memReadHook/memWriteHook) since no bundled firmware
// exercises ADC, matching the "poke registers independent of firmware"
// style test_exti.mjs already uses for GPIO.
// Usage: node site/test_component_adc.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { Potentiometer } from './components.js';

const require = createRequire(import.meta.url);
const unicorn = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, unicorn, svdXml, wasmInit: wasmBytes });

const ADC1 = 0x40012000;
const CHANNEL = 3;

function runConversion() {
    bindings.periph_write(ADC1 + 0x34, 4, CHANNEL);        // SQR3: select channel
    bindings.periph_write(ADC1 + 0x08, 4, (1 << 30) | 1);  // CR2: SWSTART, keep ADON
    // tick_n() (which advances the global instruction counter start_conversion
    // gates on) only fires once per `tickEvery` batch (default 5000) — the
    // step budget must cross that threshold or the elapsed-instructions gate
    // never opens.
    emu.step(6000);
    bindings.periph_read(ADC1 + 0x08, 4);                  // CR2 read drives start_conversion
    return bindings.periph_read(ADC1 + 0x4C, 4);            // DR
}

const pot = new Potentiometer(emu, 'ADC1', CHANNEL, { min: 0, max: 4095 });
pot.value = 2048;
const overridden = runConversion();

pot.release();
const freed = runConversion();
emu.close();

const pass = overridden === 2048 && freed < 4096;
console.log(`Potentiometer test: overridden=${overridden} freed=${freed}`);
console.log(pass ? 'PASS' : 'FAIL');
process.exit(pass ? 0 : 1);
