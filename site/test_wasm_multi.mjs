// Multi-firmware smoke for cpu='wasm'. Boots each binary, steps a budget,
// reports UART markers / faults. Usage: node site/test_wasm_multi.mjs [fw...]
import { readFileSync, existsSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const wants = process.argv.slice(2);
const allFw = [
    ['blinky', '../blinky/blinky.bin', ['=== Blinky ===', 'tick 0'], 8_000_000],
    ['eth_dhcp', '../eth_dhcp/eth_dhcp.bin', ['=== ETH DHCP Test ==='], 30_000_000],
    ['eth_test', '../eth_test/eth_test.bin', ['ETH Test: done'], 30_000_000],
    ['timer_test', '../timer_test/timer_test.bin', ['TIM'], 10_000_000],
    ['can_test', '../can_test/can_test.bin', ['CAN Test: done'], 60_000_000],
    ['hal_test', '../hal_test/hal_test.ino.bin', ['HAL'], 15_000_000],
    ['exti_test', '../exti_test/exti_test.bin', ['EXTI'], 10_000_000],
    ['rtc_test', '../rtc_test/rtc_test.bin', ['RTC'], 10_000_000],
];
const fws = wants.length ? allFw.filter(([n]) => wants.includes(n)) : allFw;

let pass = 0, fail = 0;
for (const [name, rel, markers, budget] of fws) {
    const url = new URL(rel, import.meta.url);
    if (!existsSync(url)) {
        console.log(`${name}: SKIP (no bin)`);
        continue;
    }
    const firmware = new Uint8Array(readFileSync(url));
    try {
        const emu = await createEmulator({
            firmware, bindings, unicorn: null, svdXml, wasmInit: wasmBytes, cpu_backend: 'wasm',
        });
        let uart = '';
        let stoppedAt = -1;
        let fault = null;
        for (let inst = 0; inst < budget;) {
            const res = emu.step(100000);
            inst = res.instCount;
            uart += emu.drainUart();
            if (uart.length > 20000) uart = uart.slice(-20000);
            if (res.stopped) {
                stoppedAt = inst;
                fault = emu.faultInfo ? emu.faultInfo() : null;
                break;
            }
            if (markers.every((m) => uart.includes(m))) break;
        }
        const okMarkers = markers.filter((m) => uart.includes(m));
        const ok = okMarkers.length === markers.length && stoppedAt < 0;
        console.log(`${name}: ${ok ? 'PASS' : 'FAIL'} markers=[${okMarkers.join('|')}] stoppedAt=${stoppedAt} fault=${JSON.stringify(fault)}`);
        console.log(`  tail: ${uart.replace(/\r/g, '').split('\n').filter(Boolean).slice(-2).join(' | ').slice(0, 160)}`);
        emu.close();
        if (ok) pass++;
        else fail++;
    } catch (e) {
        console.log(`${name}: ERROR ${e.message}`);
        fail++;
    }
}
console.log(`\nwasm-multi: ${pass} pass, ${fail} fail`);
process.exit(fail ? 1 : 0);
