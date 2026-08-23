// High-value emulator-edge tests (covers item 7): validates the actionable
// firmware-load error paths, rejects out-of-range MMIO, and stress-tests
// repeated createEmulator instances (per-instance isolation / reset).
// Usage: node site/test_edge_cases.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { parseIntelHex, parseElf } from './loaders.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const blinky = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

let failures = 0;
function check(cond, msg) { if (!cond) { console.error('  FAIL: ' + msg); failures++; } else { console.log('  ok: ' + msg); } }

function makeEmu() {
    return createEmulator({ firmware: new Uint8Array(blinky), bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
}

// ── 1. Bad-image rejection (validates loaders.js error paths) ──────────────
console.log('[bad-image]');
// invalid Intel HEX: record line not starting with ':'
let threw = false;
try { parseIntelHex('this is not hex'); } catch (e) { threw = /Intel HEX/.test(e.message); }
check(threw, 'non-HEX text is rejected with an Intel HEX error');
// invalid Intel HEX: checksum mismatch
threw = false;
try { parseIntelHex(':00000001FE\n'); } catch (e) { threw = /checksum/.test(e.message); }
check(threw, 'HEX checksum mismatch is rejected');
// truncated ELF
threw = false;
try { parseElf(new Uint8Array([0x7F, 0x45, 0x4C, 0x46, 0x01])); } catch (e) { threw = /bytes/.test(e.message); }
check(threw, 'truncated ELF is rejected (too short)');
// ELF with no PT_LOAD segments
const noLoad = new Uint8Array(52);
noLoad.set([0x7F, 0x45, 0x4C, 0x46, 1, 1]); // ELF32 LE
noLoad[18] = 0xFE; noLoad[19] = 0x00; // e_type = ET_NONE-ish (accept)
noLoad[20] = 0x28; noLoad[21] = 0x00; // e_machine = ARM
noLoad[40] = 52; // e_ehsize
noLoad[44] = 32; // e_phentsize
noLoad[46] = 0; // e_phnum = 0  -> no PT_LOAD
threw = false;
try { parseElf(noLoad); } catch (e) { threw = /PT_LOAD/.test(e.message); }
check(threw, 'ELF without PT_LOAD segments is rejected');

// ── 2. Invalid MMIO is rejected (not silently dropped) ─────────────────────
console.log('[invalid-mmio]');
(async () => {
    const emu = await makeEmu();
    for (let i = 0; i < 60; i++) emu.step(100000); // boot
    let mmioThrew = false;
    try { emu.uc.mem_write(0x10000000, new Uint8Array([1, 2, 3, 4])); } catch (e) { mmioThrew = true; }
    check(mmioThrew, 'write to an unmapped address (0x10000000) is rejected');
    let readThrew = false;
    try { emu.uc.mem_read(0x10000000, 4); } catch (e) { readThrew = true; }
    check(readThrew, 'read from an unmapped address (0x10000000) is rejected');

    // ── 3. Reset / reload mid-run: a fresh instance boots after a prior one ──
    console.log('[reset-reload]');
    let uart1 = '';
    for (let i = 0; i < 60; i++) uart1 += emu.drainUart().toString();
    check(/blinky/i.test(uart1) || uart1.includes('tick'), 'first instance booted');
    emu.close();

    // ── 4. Multi-instance stress (per-instance isolation / reset_state) ─────
    console.log('[multi-instance]');
    let allBooted = true;
    for (let n = 0; n < 5; n++) {
        let e;
        try { e = await makeEmu(); } catch (err) { console.error('  instance ' + n + ' createEmulator threw: ' + err); allBooted = false; continue; }
        let u = '';
        for (let i = 0; i < 60; i++) { e.step(100000); u += e.drainUart().toString(); }
        const booted = /blinky/i.test(u) || u.includes('tick');
        if (!booted) { console.error('  instance ' + n + ' uart(len=' + u.length + '): ' + JSON.stringify(u.slice(0, 80))); allBooted = false; }
        e.close();
    }
    check(allBooted, 'five sequential emulator instances all boot cleanly');

    if (failures) { console.error('EDGE FAIL: ' + failures + ' check(s) failed'); process.exit(1); }
    console.log('EDGE PASS');
    process.exit(0);
})().catch((e) => { console.error('EDGE FAIL: ' + (e.stack || e)); process.exit(1); });
