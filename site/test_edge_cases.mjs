// High-value emulator-edge tests (covers item 7): validates the actionable
// firmware-load error paths, rejects out-of-range MMIO, and stress-tests
// repeated createEmulator instances (per-instance isolation / reset).
// Usage: node site/test_edge_cases.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { parseIntelHex, parseElf } from './loaders.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const blinky = new Uint8Array(readFileSync(new URL('../firmware/blinky/blinky.bin', import.meta.url)));

let failures = 0;
function check(cond, msg) { if (!cond) { console.error('  FAIL: ' + msg); failures++; } else { console.log('  ok: ' + msg); } }

function makeEmu() {
    return createEmulator({ firmware: new Uint8Array(blinky), bindings, svdXml, wasmInit: wasmBytes });
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

    // ── 5. Emulator-defect edges (not guest-library re-tests) ──
    // Each pins a real defect class the emulator actually had, so a
    // regression fails here instead of silently returning wrong data.
    console.log('[emulator-defect-edges]');

    // 5a. faultInfo: a BKPT firmware stops with a fault PC (never a silent
    // wrong result — the loud-halt contract in AGENTS §21). The BKPT is
    // baked into the image: the core loads firmware straight into flash,
    // so a mem_write-then-step races nothing, but the image path is the
    // canonical one (no post-boot patching).
    {
        // 16-byte image: vector table + bkpt #0 at 0x08000008.
        const img = new Uint8Array(16);
        const dv = new DataView(img.buffer);
        dv.setUint32(0, 0x20020000, true);   // SP
        dv.setUint32(4, 0x08000009, true);   // PC (Thumb) -> offset 8
        img[8] = 0x00; img[9] = 0xBE;        // bkpt #0
        const e2 = await createEmulator({ firmware: img, bindings, svdXml, wasmInit: wasmBytes });
        const r = e2.step(100);
        const fi = e2.faultInfo();
        check(r.stopped === true, 'BKPT firmware stops (stopped=true)');
        check(fi !== null && (fi.pc >>> 0) === 0x08000008, 'faultInfo reports the BKPT pc');
        e2.close();
    }

    // 5b. resetCpu: re-running the same image restarts its UART from the
    // top (regression: stale per-instance state leaking across resets).
    {
        const e3 = await makeEmu();
        for (let i = 0; i < 60; i++) e3.step(100000);
        const before = e3.drainUart().toString();
        check(/blinky/i.test(before) || before.includes('tick'), 'resetCpu probe instance booted');
        e3.resetCpu();
        let after = '';
        for (let i = 0; i < 60; i++) { e3.step(100000); after += e3.drainUart().toString(); }
        check(/blinky/i.test(after) || after.includes('tick'), 'resetCpu reboots to banner output');
        e3.close();
    }

    // 5c. Peripheral-space holes read benign-0 (HALs probe reserved regs;
    // wild *memory* BusFaults — the documented split, AGENTS §21).
    {
        const e4 = await makeEmu();
        for (let i = 0; i < 5; i++) e4.step(100000);
        // 0x40002C00 is WWDG (modeled); a hole like 0x40010000+0x3C00
        // (between real blocks) must read 0, not throw.
        let hole = null, threwHole = false;
        try { hole = e4.read32(0x40010C00); } catch { threwHole = true; }
        check(!threwHole && (hole >>> 0) === 0, 'peripheral-space hole reads benign-0');
        e4.close();
    }

    // 5d. FPU file visible: fpu_test boots and S-regs/FPSCR read back
    // (regression: core-side-only FPU state, AGENTS §25). Path is
    // site-relative (this harness lives in site/, firmware/ is one up).
    {
        let fpu = null;
        try {
            const fpuBin = new Uint8Array(readFileSync(new URL('../firmware/fpu_test/fpu_test.bin', import.meta.url)));
            const e5 = await createEmulator({ firmware: fpuBin, bindings, svdXml, wasmInit: wasmBytes });
            for (let i = 0; i < 40; i++) e5.step(100000);
            fpu = e5.getFpuState();
            e5.close();
        } catch (err) { console.error('  fpu probe: ' + (err && err.message)); }
        check(fpu !== null && Array.isArray(fpu.s) && fpu.s.length === 32, 'FPU S0-S31 + FPSCR visible via getFpuState');
    }

    // 5e. Trace buffer: traceStart/takeTrace round-trips guest PCs
    // (regression: trace API present but unwired). takeTrace returns a
    // Uint32Array view over wasm memory (not a JS Array) — assert the
    // typed-array contract, not Array.isArray.
    {
        const e6 = await makeEmu();
        e6.traceStart();
        for (let i = 0; i < 5; i++) e6.step(100000);
        let tr = null;
        try { tr = e6.takeTrace(); } catch {}
        e6.traceStop();
        check(tr !== null && typeof tr.length === 'number' && tr.length > 0, 'takeTrace returns guest PCs after traceStart');
        e6.close();
    }

    if (failures) { console.error('EDGE FAIL: ' + failures + ' check(s) failed'); process.exit(1); }
    console.log('EDGE PASS');
    process.exit(0);
})().catch((e) => { console.error('EDGE FAIL: ' + (e.stack || e)); process.exit(1); });
