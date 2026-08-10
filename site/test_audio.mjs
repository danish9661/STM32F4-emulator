// Node harness for the I2S/DMA audio path.
// Phase A: load a WAV into the model (audio_load_wav), the firmware runs
//   DMA1 Stream0 PERIPH->MEM from I2S1_DR; the sample checksum printed by
//   the firmware must equal the JS-computed sum of the WAV samples.
// Phase B: firmware writes 16 known 16-bit values to I2S1_DR; the model's
//   capture FIFO (audio_take_capture) must return exactly those values.
// Usage: node site/test_audio.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../audio_test/audio_test.bin', import.meta.url)));

const RX_N = 64;
const TX_N = 16;
const samples = Array.from({ length: RX_N }, (_, i) => (i * 300 + 7) & 0xFFFF);
const expectedSum = samples.reduce((a, b) => a + b, 0) >>> 0;

function makePcm16Wav(samples) {
    const dataLen = samples.length * 2;
    const w = Buffer.alloc(44 + dataLen);
    w.write('RIFF', 0, 'ascii');
    w.writeUInt32LE(36 + dataLen, 4);
    w.write('WAVE', 8, 'ascii');
    w.write('fmt ', 12, 'ascii');
    w.writeUInt32LE(16, 16);
    w.writeUInt16LE(1, 20);
    w.writeUInt16LE(1, 22);
    w.writeUInt32LE(44100, 24);
    w.writeUInt32LE(44100 * 2, 28);
    w.writeUInt16LE(2, 32);
    w.writeUInt16LE(16, 34);
    w.write('data', 36, 'ascii');
    w.writeUInt32LE(dataLen, 40);
    samples.forEach((s, i) => w.writeInt16LE(s, 44 + i * 2));
    return w;
}

const wav = makePcm16Wav(samples);
const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });

bindings.audio_load_wav(wav);
console.log('wav samples:', samples.length, 'expectedSum: 0x' + expectedSum.toString(16).toUpperCase());

const uart = [];
for (let i = 0; i < 800 && !uart.join('').includes('=== Audio Test: done'); i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
}
const all = uart.join('');
console.log('uart tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | '));

const cap = bindings.audio_take_capture ? bindings.audio_take_capture() : [];
const capOk = cap.length === TX_N && cap.every((v, i) => v === 1000 + i);
const sumHex = expectedSum.toString(16).toUpperCase().padStart(8, '0');
const pass =
    all.includes('=== Audio Test ===') &&
    all.includes('DMA RX OK') &&
    all.includes('RX n=64 sum=' + sumHex) &&
    all.includes('TX n=16 OK') &&
    all.includes('=== Audio Test: done ===') &&
    capOk &&
    !all.includes('FAIL');
console.log(pass ? 'PASS' : 'FAIL', pass ? '' : `(cap=${Array.from(cap).join(',')})`);
emu.close();
process.exit(pass ? 0 : 1);