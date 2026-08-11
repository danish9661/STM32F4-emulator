// Node harness for the I2S speaker (audio_play_test firmware).
// Asserts: boot + I2S ready markers, and that the JS-drained capture
// FIFO yields a non-zero periodic (sine) sample stream.
// Usage: node site/test_audio_play.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../audio_play_test/audio_play_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { speaker: true },
});

const uart = [];
let samples = new Float32Array(0);
for (let i = 0; i < 300; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    const s = emu.takeSpeakerSamples();
    if (s.length) {
        const merged = new Float32Array(samples.length + s.length);
        merged.set(samples, 0);
        merged.set(s, samples.length);
        samples = merged;
    }
    if (uart.join('').includes('TICK') && samples.length > 4096) break;
}
const all = uart.join('');
console.log('uart:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(0, 6).join(' | '));

let nz = 0, min = 1, max = -1;
for (const v of samples) {
    if (v !== 0) nz++;
    if (v < min) min = v;
    if (v > max) max = v;
}
// periodicity: count sign changes of the mid-level crossings
let crossings = 0;
for (let i = 1; i < samples.length; i++) {
    if ((samples[i - 1] < 0 && samples[i] >= 0) || (samples[i - 1] >= 0 && samples[i] < 0)) crossings++;
}
console.log(`samples=${samples.length} nonzero=${nz} min=${min.toFixed(3)} max=${max.toFixed(3)} zeroCrossings=${crossings}`);

const pass =
    all.includes('Audio play test') &&
    all.includes('I2S ready') &&
    samples.length > 2048 &&
    nz > samples.length * 0.9 &&          // sine: almost all samples non-zero
    max > 0.5 && min < -0.5 &&             // amplitude ~30000/32768 = 0.915
    crossings > samples.length / 200;      // 256-sample sine: ~2 crossings per period
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
