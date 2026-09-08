// Differential fuzz: runs fuzz_test/fuzz_test.bin on BOTH backends (wasm CPU
// and Unicorn) with the same inputs and asserts bit-identical UART output.
// Covers the full decoder census (data-proc, shifts, DSP, SIMD, SAT/PKH,
// LDRD/STRD, branches, IT flag-setting) + fault-stop parity (BKPT/SVC stop
// the run on both backends). Exit 0 = FUZZ-IDENTICAL, 1 = divergence.
// Needs: fuzz_test/fuzz_test.bin (make -C fuzz_test), site/doom1.wad.
import { mkdirSync, readFileSync, writeFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../fuzz_test/fuzz_test.bin', import.meta.url)));
const wad = new Uint8Array(readFileSync(new URL('./doom1.wad', import.meta.url)));
const scratchDir = new URL('../.pw-scratch/', import.meta.url);
mkdirSync(scratchDir, { recursive: true });
// Ops whose line ends with an APSR dump: compare it masked to NZCVQ+GE
// (Unicorn's real MRS leaves low-bit EPSR residue, e.g. ...01D3, while the
// wasm CPU masks to ...0000 — same class as the reset-state difference).
const GE_OPS = new Set(['UADD8','USUB8']);
const APSR_OPS = new Set(['SDIV','UDIV','SMLAD','SMLSD','SSAT8Q','QADD','QSUB','QDADD','QDSUB','MULS','SUBS','MSRCLR','ADDS01','SUBS01','FSUB','FADD','SBC0','ADC1']);
const outs = {};
for (const backend of ['wasm', 'unicorn']) {
  const emu = await createEmulator({ firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes, cpu_backend: backend,
    extra_ram: [{ addr: 0xC0000000, size: 16*1024*1024 }, { addr: 0xB8000000, size: 8*1024*1024 }],
    extra_mem: [{ addr: 0xB8000000, data: new Uint8Array(wad) }] });
  let uart = '';
  try {
  for (let i = 0; i < 500 && !uart.includes('FUZZ-FAULTS'); i++) { emu.step(100000); uart += (emu.drainUart() || ''); }
  } catch (e) { try { uart += (emu.drainUart() || ''); } catch {} console.log(`${backend} phase1-stop: ${String(e).slice(0, 100)}`); }
  // fault phase: the BKPT must stop the run (no FAULT-MISS lines allowed)
  let stopped = false;
  for (let i = 0; i < 60 && !stopped; i++) {
    try {
      const r = emu.step(20000);
      uart += (emu.drainUart() || '');
      stopped = !!r.stopped;
    } catch (e) { console.log(`${backend} fault-stop: ${String(e).slice(0, 80)}`); stopped = true; break; }
  }
  uart += stopped ? '\nFAULTOK\n' : '\nFAULT-NOTSTOPPED\n';
  let stamp = '';
  try {
    stamp = ` stamps=${emu.read32(0x20001000).toString(16)}/${emu.read32(0x20001004).toString(16)}/${emu.read32(0x20001008).toString(16)}`;
    try {
      let it = [];
      for (let k = 0; k < 4; k++) it.push(emu.read32(0x20001010 + k * 4).toString(16));
      stamp += ` iter=[${it.join(',')}]`;
    } catch (e) { stamp += ' iter=READERR'; }
  } catch (e) { stamp = ' stamps=READERR'; }
  outs[backend] = uart;
  console.log(`${backend}${stamp}`);
  writeFileSync(new URL(`../.pw-scratch/fuzz_${backend}.txt`, import.meta.url), uart);
  emu.close();
}
const norm = (t) => t.replace(/\r/g, '').split('\n').map((l) => l.trim()).filter((l) => l.length > 0);
const key = (l) => {
  const toks = l.split(/\s+/);
  if (toks.length >= 2 && (APSR_OPS.has(toks[0]) || GE_OPS.has(toks[0]) || toks[0].startsWith('ITF'))) {
    const mask = GE_OPS.has(toks[0]) ? 0xF80F0000 : 0xF8000000;
    const v = parseInt(toks[toks.length - 1], 16) >>> 0;
    toks[toks.length - 1] = 'F' + (((v & mask) >>> 0).toString(16).toUpperCase().padStart(8, '0'));
    return toks.join(' ');
  }
  return toks.join(' ');
};
const a = norm(outs.wasm).map(key), b = norm(outs.unicorn).map(key);
let diffs = 0;
for (let i = 0; i < Math.max(a.length, b.length); i++) {
  if (a[i] !== b[i]) { diffs++; if (diffs <= 25) console.log(`L${i}:\n  wasm:    ${a[i]}\n  unicorn: ${b[i]}`); }
}
console.log(`wasm lines=${a.length} unicorn lines=${b.length}`);
const identical = diffs === 0 && a.length > 100;
console.log(identical ? 'FUZZ-IDENTICAL' : `FUZZ-DIFFS: ${diffs}`);
process.exit(identical ? 0 : 1);
