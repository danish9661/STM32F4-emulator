// TB (translated block) correctness pins — runs in `npm test`.
//
// Why this exists: the translator's condition-code emitter had LE backwards
// (`Z || N==V`, i.e. GE). Every existing test loop used BNE/BCS/BGE, so
// nothing noticed until eth_feat_test's volatile delay spins (`ble` loops)
// collapsed from 20000 iterations to one — which silently shortened a delay
// the harness hooks depend on, and surfaced as `LINK UP FAIL`. A silent-wrong
// branch is the worst class of bug this component can have, so the whole
// condition table is swept against an independent truth table, plus the exact
// firmware spin shape and its exact instruction accounting.
//
// Usage: node site/test_tb_xlate.mjs   (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
await bindings.default({ module_or_path: wasmBytes });

const SEGS = new Uint32Array([0x20000000, 65536]);
const MIRR = bindings.tb_scratch_bytes();
const BITB = bindings.tb_valid_base();

let pass = 0, fail = 0;
const check = (name, cond, extra = '') => {
    if (cond) pass++;
    else { fail++; if (fail <= 12) console.log(`FAIL: ${name} ${extra}`); }
};

async function translate(ops, base = 0x1000) {
    const bytes = bindings.tb_translate(new Uint16Array(ops), SEGS, base);
    return WebAssembly.instantiate(await WebAssembly.compile(bytes), {});
}

// ── 1. Every condition code against every NZCV pattern ──────────────────
// Shape: [NOP, Bcc back-to-base] — a 2-instruction loop whose body does not
// touch flags, so seeding xpsr (N/Z/C/V in bits 31/30/29/28) drives the
// latch condition directly.
//   taken     -> the latch loops until fuel runs out (kind 2, insts == fuel)
//   not taken -> one iteration then completion (kind 0, insts == 2)
const condOk = (cc, nzcv) => {
    const N = (nzcv >>> 31) & 1, Z = (nzcv >>> 30) & 1;
    const C = (nzcv >>> 29) & 1, V = (nzcv >>> 28) & 1;
    switch (cc) {
        case 0: return Z === 1;              // EQ
        case 1: return Z === 0;              // NE
        case 2: return C === 1;              // CS/HS
        case 3: return C === 0;              // CC/LO
        case 4: return N === 1;              // MI
        case 5: return N === 0;              // PL
        case 6: return V === 1;              // VS
        case 7: return V === 0;              // VC
        case 8: return C === 1 && Z === 0;   // HI
        case 9: return C === 0 || Z === 1;   // LS
        case 10: return N === V;             // GE
        case 11: return N !== V;             // LT
        case 12: return Z === 0 && N === V;  // GT
        case 13: return Z === 1 || N !== V;  // LE
        default: throw new Error(`bad cc ${cc}`);
    }
};
const CC_NAMES = ['eq', 'ne', 'cs', 'cc', 'mi', 'pl', 'vs', 'vc',
                  'hi', 'ls', 'ge', 'lt', 'gt', 'le'];

for (let cc = 0; cc < 14; cc++) {
    for (let flags = 0; flags < 16; flags++) {
        // Bcc at base+2: pc = base+6, target base -> imm8 = -3 (0xFD).
        const inst = await translate([0xBF00, 0xD000 | (cc << 8) | 0xFD]);
        const nzcv = ((flags & 8) ? 0x80000000 : 0) | ((flags & 4) ? 0x40000000 : 0) |
                     ((flags & 2) ? 0x20000000 : 0) | ((flags & 1) ? 0x10000000 : 0);
        inst.exports.xpsr.value = nzcv | 0;
        const r = inst.exports.run(100);
        const o = { insts: Number(r & 0xFFFFFFFFn), kind: Number(r >> 32n) };
        const want = condOk(cc, nzcv);
        check(`cc-${CC_NAMES[cc]}-nzcv${flags}`,
            o.kind === (want ? 2 : 0) && o.insts === (want ? 100 : 2),
            `want=${want} ${JSON.stringify(o)}`);
    }
}
console.log(`cond table: 14 cc x 16 NZCV = ${14 * 16} cases`);

// ── 2. The eth_feat delay spin, exact firmware halfwords ────────────────
// 0x0801b4c in eth_feat_test.elf:
//   loop: ldr r3,[sp,#40] ; adds r3,#1 ; str r3,[sp,#40]
//         ldr r3,[sp,#40] ; cmp  r3,r2 ; ble loop
// Bound r2 = 19999 -> 20000 iterations x 6 instructions. This is the shape
// that exposed the LE bug (it collapsed to one iteration, so a 20000-count
// delay finished inside a single 20k-instruction step and the PHY read ran
// before the harness could raise link).
const SPIN = [0x9B0A, 0x3301, 0x930A, 0x9B0A, 0x4293, 0xDDF9];
const SPIN_BASE = 0x0801B4C;
const SP = 0x20001000, SLOT = 40;

async function runSpin(seedSlot, fuel) {
    const inst = await translate(SPIN, SPIN_BASE);
    const mem = new Uint8Array(inst.exports.mem.buffer);
    // Mark the segment's mirror pages valid (what the driver's sync does).
    for (let pg = MIRR >>> 12; pg < (MIRR + 65536) >>> 12; pg++)
        mem[BITB + (pg >>> 3)] |= 1 << (pg & 7);
    inst.exports.sp.value = SP | 0;
    inst.exports.r2.value = 19999;
    inst.exports.xpsr.value = 0x01000000;
    const slotIdx = (MIRR + SP + SLOT - 0x20000000) >>> 2;
    new Uint32Array(inst.exports.mem.buffer)[slotIdx] = seedSlot;
    const r = inst.exports.run(fuel);
    return { insts: Number(r & 0xFFFFFFFFn), kind: Number(r >> 32n),
             slot: new Uint32Array(inst.exports.mem.buffer)[slotIdx],
             r3: inst.exports.r3.value | 0 };
}

{
    // Fresh counter: full 20000 iterations, 120000 guest instructions.
    const o = await runSpin(0, 100000000);
    check('spin-full', o.kind === 0 && o.insts === 120000 && o.slot === 20000 && o.r3 === 20000,
        JSON.stringify(o));
}
{
    // Residual at the bound (a previous spin's value): exactly one
    // iteration, because 20000 <= 19999 is false.
    const o = await runSpin(19999, 100000000);
    check('spin-residue-at-bound', o.kind === 0 && o.insts === 6 && o.slot === 20000,
        JSON.stringify(o));
}
{
    // Fuel split must be exact: 20 instructions = 3 iterations + 2 left.
    const o = await runSpin(0, 20);
    check('spin-fuel-exact', o.kind === 2 && o.insts === 18 && o.slot === 3 && o.r3 === 3,
        JSON.stringify(o));
}

// ── 3. The eth_http RX-wait loop must translate to VALID wasm ───────────
// The hottest shape in the whole corpus (~42% of an eth_http run): its cold
// arm is a run of holes (unemittable items). The walk used to drain only
// ONE hole per item iteration, fell behind the latch, and left the skip
// blocks for targets 47/61 unterminated — so the module failed validation
// ("unterminated control structure @+430") and the loop never translated.
// Rust has no wasm validator, so check the emitted bytes with the real one.
// Base + entries from the shipped firmware (objdump): hw 0 = top,
// hw 5 = the lsls/bmi arm, hw 61 = the cbnz target (the taken hot path).
const RX_BASE = 0x080008f8;
const ethBin = new Uint8Array(readFileSync(new URL('../firmware/eth_http/eth_http.bin', import.meta.url)));
const RX_SEGS = new Uint32Array([0x20000000, 0x20000, 0, 0x08000000, 0x100000, 0]);
const rxOps = new Uint16Array(80);
for (let i = 0; i < 80; i++) rxOps[i] = ethBin[0x8f8 + i * 2] | (ethBin[0x8f9 + i * 2] << 8);

for (const [label, entryHw] of [['top', 0], ['mid5', 5], ['mid61', 61]]) {
    const bytes = bindings.tb_translate_at_direct(rxOps, RX_SEGS, RX_BASE, RX_BASE + entryHw * 2);
    let ok = false, err = '';
    try { new WebAssembly.Module(bytes); ok = true; } catch (e) { err = e.message; }
    check(`rx-loop-${label}-valid`, ok, `${bytes.length}B ${err}`);
}
{
    const meta = bindings.tb_translate_meta_at_direct(rxOps, RX_SEGS, RX_BASE, RX_BASE);
    check('rx-loop-top-is-loop', (meta & 1) === 1, `meta=0x${meta.toString(16)}`);
}

if (fail === 0) console.log(`TB-XLATE PASS ${pass} checks (cond ${14 * 16}, spin 3, rx-loop 4)`);
else console.log(`TB-XLATE FAIL ${fail} of ${pass + fail}`);
process.exit(fail === 0 ? 0 : 1);
