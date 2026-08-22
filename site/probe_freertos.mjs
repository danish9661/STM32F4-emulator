import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../freertos_test/freertos_test.bin', import.meta.url)));

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml,
    wasmInit: wasmBytes, enable_irqs: true, freertos: true,
});

const uc = emu.uc, M = emu.Module;
const rd32 = (a) => { const b = uc.mem_read(BigInt(a), 4); return (b[3]<<24)|(b[2]<<16)|(b[1]<<8)|b[0]; };

// Resolve kernel variable addresses from the firmware ELF symbol table so the
// probe doesn't break every time the .bss layout shifts (e.g. adding a
// .testvars section moves xTickCount/pxCurrentTCB/pxReadyTasksLists around).
const elfSym = (() => {
    const buf = new Uint8Array(readFileSync(new URL('../freertos_test/freertos_test.elf', import.meta.url)));
    const dv = new DataView(buf.buffer, buf.byteOffset, buf.byteLength);
    const isLE = buf[5] === 1;
    const u16 = (o) => dv.getUint16(o, isLE);
    const u32 = (o) => dv.getUint32(o, isLE);
    const e_shoff = u32(0x20), e_shentsize = u16(0x2e), e_shnum = u16(0x30), e_shstrndx = u16(0x32);
    const sections = [];
    for (let i = 0; i < e_shnum; i++) {
        const o = e_shoff + i * e_shentsize;
        sections.push({ type: u32(o + 4), offset: u32(o + 16), size: u32(o + 20), link: u32(o + 24), entsize: u32(o + 36) });
    }
    const shstr = sections[e_shstrndx];
    const strAt = (base, off) => { let p = base + off, s = ''; while (buf[p] !== 0) s += String.fromCharCode(buf[p++]); return s; };
    const syms = {};
    for (const s of sections) {
        if (s.type !== 2) continue; // SHT_SYMTAB
        const str = sections[s.link];
        const n = s.entsize ? s.size / s.entsize : 0;
        for (let i = 0; i < n; i++) {
            const o = s.offset + i * s.entsize;
            const nameOff = u32(o), value = u32(o + 4);
            const nm = strAt(str.offset, nameOff);
            if (nm && !(nm in syms)) syms[nm] = value;
        }
    }
    return syms;
})();
const XTICK = elfSym.xTickCount;
const PCUR = elfSym.pxCurrentTCB;
const READY = elfSym.pxReadyTasksLists;
const HIGH = elfSym.g_high_count;
if (!XTICK || !PCUR || !READY || !HIGH) {
    console.error('FATAL: could not resolve kernel symbols', { xTickCount: XTICK, pxCurrentTCB: PCUR, pxReadyTasksLists: READY, g_high_count: HIGH });
    process.exit(2);
}
console.log(`[syms] xTickCount=0x${XTICK.toString(16)} pxCurrentTCB=0x${PCUR.toString(16)} pxReadyTasksLists=0x${READY.toString(16)} g_high_count=0x${HIGH.toString(16)}`);

let instCount = 0;
const MAX = 120000000;
let pc = 0;
const seen = new Set();
const tcbSet = new Set();   // count distinct TCBs (TASK1/TASK2/IDLE/HIGH) -> proves preemption
let maxTick = 0;
let maxHigh = 0;
let allOut = '';

// TCBs are heap-allocated and shift between builds, so discover them from the
// FreeRTOS ready lists instead of hardcoding.  Each List_t is 20 bytes; the
// first real list item's pointer lives at list+12 and is the TCB's
// xStateListItem (TCB offset 4).
let TCB_T1 = 0, TCB_T2 = 0, TCB_IDLE = 0;
const discoverTcbs = () => {
    if (TCB_T1 && TCB_T2 && TCB_IDLE) return;
    if (rd32(PCUR) === 0) return; // scheduler not up yet
    const t1 = rd32(READY + 20 * 2 + 12) - 4;
    const t2 = rd32(READY + 20 * 1 + 12) - 4;
    const idle = rd32(READY + 12) - 4;
    if (t1 && t2 && idle) { TCB_T1 = t1; TCB_T2 = t2; TCB_IDLE = idle; }
};

let tim2c0 = -1, tim2c1 = -1, tim3isr = -1, timPass = false, timFail = false;
const gTim3IsrAddr = elfSym.g_tim3_isr, gHighAddr = HIGH;
let maxTim3 = 0, diagDone = false;

try {
    for (; instCount < MAX; instCount += 250000) {
        const r = emu.step(250000);
        pc = r.pc;
        const u = emu.drainUart().toString();
        allOut += u;
        const tick = rd32(XTICK);
        const tcb = rd32(PCUR);
        const high = rd32(HIGH);
        if (high > maxHigh) maxHigh = high;
        const g3 = rd32(gTim3IsrAddr);
        if (g3 > maxTim3) maxTim3 = g3;
        discoverTcbs();
        if (tcb === TCB_T1) seen.add('TASK1');
        else if (tcb === TCB_T2) seen.add('TASK2');
        else if (tcb === TCB_IDLE) seen.add('IDLE');
        else if (tcb !== 0) seen.add('HIGH');
        tcbSet.add(tcb);
        if (tick > maxTick) maxTick = tick;
        const m2 = u.match(/TIM2 adv (-?\d+)->(-?\d+)[^\r\n]*/);
        if (m2) { tim2c0 = parseInt(m2[1]); tim2c1 = parseInt(m2[2]); console.log('   [parse]', m2[0]); }
        const m3 = u.match(/TIM3 isr (-?\d+) high (-?\d+)[^\r\n]*/);
        if (m3) { tim3isr = parseInt(m3[1]); maxHigh = Math.max(maxHigh, parseInt(m3[2])); console.log('   [parse]', m3[0]); }
        if (u.includes('TIM TEST PASS')) timPass = true;
        if (u.includes('TIM TEST FAIL')) timFail = true;
        // Stop early once every success marker has been observed. This only
        // shortens the run (never converts a pass into a fail): the full
        // scheduler + ISR -> semaphore -> context-switch path has already
        // been exercised by the time all of these hold.
        const tickHits = allOut.split('tick=').length - 1;
        if (tim3isr > 0 && maxHigh > 0 && timPass && tickHits > 0 &&
            seen.has('TASK1') && seen.has('TASK2') && seen.has('IDLE') &&
            allOut.includes('TASK1') && allOut.includes('TASK2')) {
            console.log(`[probe] all success markers observed at inst ${instCount}; stopping early`);
            break;
        }
        if (instCount % 5000000 === 0) {
            console.log(`[${instCount}] pc=0x${pc.toString(16)} tick=${tick} tcb=0x${tcb.toString(16)} uartLen=${u.length} tail=${JSON.stringify(u.slice(-60))}`);
            if (TCB_T1) console.log(`   TCBs idle=0x${TCB_IDLE.toString(16)} T1=0x${TCB_T1.toString(16)} T2=0x${TCB_T2.toString(16)}`);
        }
        // Only stop early on a hard failure; a TIM TEST PASS is just a
        // milestone — keep running so the scheduler (TASK1/TASK2/IDLE) is
        // also exercised and observed.
        if (timFail) break;
    }
} catch (e2) {
    console.log('STEP ERROR at inst', instCount, 'pc=0x' + pc.toString(16), ':', e2.message);
    try {
        const rd = (r) => uc.reg_read_i32(r).toString(16);
        console.log('PC=', rd(M.ARM_REG_PC), 'LR=', rd(M.ARM_REG_LR), 'PSP=', rd(M.ARM_REG_PSP), 'MSP=', rd(M.ARM_REG_SP), 'CONTROL=', rd(M.ARM_REG_CONTROL));
    } catch (e3) { console.log('diag failed:', e3.message); }
    process.exit(1);
}
const all = allOut;
console.log('=== UART OUTPUT (last 240 chars) ===');
console.log(all.slice(-240));
console.log('=== END ===');
const t1 = (all.split('TASK1').length - 1);
const t2 = (all.split('TASK2').length - 1);
const ticks = (all.split('tick=').length - 1);
console.log('TASK1 hits:', t1, 'TASK2 hits:', t2, 'tick= hits:', ticks, 'total chars:', all.length);
console.log('TCBs observed:', [...seen].join(','), 'maxTick:', maxTick, 'distinct TCBs:', tcbSet.size);
console.log('TCB addrs: idle=0x' + TCB_IDLE.toString(16) + ' T1=0x' + TCB_T1.toString(16) + ' T2=0x' + TCB_T2.toString(16));
console.log('TIM2 c0=', tim2c0, 'c1=', tim2c1, 'TIM3 isr=', tim3isr, 'high=', maxHigh, 'timPass=', timPass, 'timFail=', timFail);

const okSched = t1 > 0 && t2 > 0 && ticks > 0 && seen.has('TASK1') && seen.has('TASK2') && seen.has('IDLE');
const okTim2 = tim2c1 > tim2c0;
const okTim3 = tim3isr > 0;
const okSem = maxHigh > 0;                 // ISR -> semaphore -> ctx switch to vHighTask
const ok = okSched && okTim2 && okTim3 && okSem && timPass && !timFail;
console.log(ok ? 'PROBE PASS' : 'PROBE FAIL');
emu.close();
process.exit(ok ? 0 : 1);
