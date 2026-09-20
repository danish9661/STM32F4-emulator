// Node harness for the Servo component (site/components.js) + the
// model-side tim_oc_mode/tim_pwm_pulse_us probes: programs TIM2 CH1 to a
// 50 Hz / 1.5 ms servo pulse via raw MMIO (no firmware needed — the probes
// read the model, and Servo reads the same regs through the emulator),
// then asserts mode decodes PWM1, pulse reads ~1500 us, angle ~90°.
// Usage: node site/test_component_servo.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { Servo } from './components.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../firmware/blinky/blinky.bin', import.meta.url)));

const emu = await createEmulator({ firmware, bindings, svdXml, wasmInit: wasmBytes });
const T2 = 0x40000000;
emu.write32(T2 + 0x28, 83); // PSC: 84 MHz/84 = 1 MHz tick
emu.write32(T2 + 0x2C, 19999); // ARR: 20 ms frame
emu.write32(T2 + 0x18, 6 << 4); // CCMR1: OC1M=PWM1
emu.write32(T2 + 0x34, 1500); // CCR1: 1.5 ms pulse
emu.write32(T2 + 0x20, 0x01); // CCER: CC1E
emu.write32(T2, 1); // CR1: CEN
emu.step(1000);

const servo = new Servo(emu, 'TIM2', 1);
const mode = bindings.tim_oc_mode('TIM2', 0);
const us = bindings.tim_pwm_pulse_us('TIM2', 0, 84e6);
const angle = servo.angle;
console.log(`Servo: mode=${mode} (${servo.pwm.modeName}) pulseUs=${servo.pulseUs.toFixed(1)} modelUs=${us.toFixed(1)} angle=${angle.toFixed(1)}`);
const pass = mode === 6 && Math.abs(us - 1500) < 5 && Math.abs(servo.pulseUs - 1500) < 5 && Math.abs(angle - 90) < 1;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
