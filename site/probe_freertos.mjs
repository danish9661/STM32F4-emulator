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

let instCount = 0;
const MAX = 20000000;
let pc = 0;
const TCB_IDLE = 0x20000dc8, TCB_T1 = 0x20000508, TCB_T2 = 0x20000968;
const seen = new Set();
let maxTick = 0;
let allOut = '';
try {
    for (; instCount < MAX; instCount += 250000) {
        const r = emu.step(250000);
        pc = r.pc;
        const u = emu.drainUart().toString();
        allOut += u;
        const tick = rd32(0x20000068);
        const tcb = rd32(0x20000074);
        if (tcb === TCB_T1) seen.add('TASK1');
        else if (tcb === TCB_T2) seen.add('TASK2');
        else if (tcb === TCB_IDLE) seen.add('IDLE');
        if (tick > maxTick) maxTick = tick;
        if (instCount % 5000000 === 0) {
            console.log(`[${instCount}] pc=0x${pc.toString(16)} tick=${tick} tcb=0x${tcb.toString(16)} uartLen=${u.length} tail=${JSON.stringify(u.slice(-50))}`);
        }
        if (u.includes('TASK1') && u.includes('TASK2') && u.includes('tick=')) {
            console.log('EARLY SUCCESS at', instCount);
            break;
        }
    }
} catch (e2) {
    console.log('STEP ERROR at inst', instCount, 'pc=0x' + pc.toString(16), ':', e2.message);
    try {
        const uc = emu.uc;
        const M = emu.Module;
        const rd = (r) => uc.reg_read_i32(r).toString(16);
        console.log('PC=', rd(M.ARM_REG_PC), 'LR=', rd(M.ARM_REG_LR), 'PSP=', rd(M.ARM_REG_PSP), 'MSP=', rd(M.ARM_REG_SP), 'CONTROL=', rd(M.ARM_REG_CONTROL));
        const psp = uc.reg_read_i32(M.ARM_REG_PSP);
        if (psp > 0x20000000 && psp < 0x20020000) {
            const fr = uc.mem_read(BigInt(psp), 32);
            const dv = new DataView(fr.buffer, fr.byteOffset, fr.byteLength);
            console.log('PSP frame: R0-R3=', [0,4,8,12].map(o=>dv.getUint32(o,true).toString(16)), 'R12=', dv.getUint32(16,true).toString(16), 'LR=', dv.getUint32(20,true).toString(16), 'PC=', dv.getUint32(24,true).toString(16), 'xPSR=', dv.getUint32(28,true).toString(16));
        }
    } catch (e3) { console.log('diag failed:', e3.message); }
    process.exit(1);
}
const all = allOut;
console.log('=== UART OUTPUT (last 800 chars) ===');
console.log(all.slice(-800));
console.log('=== END ===');
const t1 = (all.split('TASK1').length - 1);
const t2 = (all.split('TASK2').length - 1);
const ticks = (all.split('tick=').length - 1);
console.log('TASK1 hits:', t1, 'TASK2 hits:', t2, 'tick= hits:', ticks, 'total chars:', all.length);
console.log('TCBs observed:', [...seen].join(','), 'maxTick:', maxTick);
const ok = t1 > 0 && t2 > 0 && ticks > 0 && seen.has('TASK1') && seen.has('TASK2') && seen.has('IDLE');
console.log(ok ? 'PROBE PASS' : 'PROBE FAIL');
emu.close();
process.exit(ok ? 0 : 1);
