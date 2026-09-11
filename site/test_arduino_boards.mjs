// Real-firmware board validation: the SAME Arduino sketch (Serial prints,
// LED blink, SysTick delay()) compiled by Arduino-Core for all 8 board
// targets, run against each board's SVD + memory sizes. This exercises the
// full Arduino stack — SystemInit PLL bring-up (RCC ready flags), .data/
// .bss init, GPIO, SysTick IRQs, USART TX — not just our hand blinkies.
// Per-board Serial port + LED pin come from the core variant files.
// Usage: node site/test_arduino_boards.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const BOARDS = [
    // [key, svd, flash, ram, uartAddr, odrAddr, odrMask, label]
    ['bp_f401cc', 'stm32f401', 0x40000, 0x10000, 0x40011000, 0x40020814, 0x2000, 'BlackPill F401CC'],
    ['bp_f411ce', 'stm32f411', 0x80000, 0x20000, 0x40011000, 0x40020814, 0x2000, 'BlackPill F411CE'],
    ['nucleo_f401re', 'stm32f401', 0x80000, 0x18000, 0x40004400, 0x40020014, 0x20, 'Nucleo-F401RE'],
    ['nucleo_f411re', 'stm32f411', 0x80000, 0x20000, 0x40004400, 0x40020014, 0x20, 'Nucleo-F411RE'],
    ['disco_f407vg', 'stm32f407', 0x100000, 0x30000, 0x40004400, 0x40020C14, 0x1000, 'Discovery F407VG'],
    ['disco_f429zi', 'stm32f429', 0x200000, 0x40000, 0x40011000, 0x40021814, 0x2000, 'Discovery F429ZI'],
    ['black_f407ve', 'stm32f407', 0x80000, 0x30000, 0x40011000, 0x40020014, 0x40, 'Black F407VE'],
    ['black_f407ze', 'stm32f407', 0x100000, 0x30000, 0x40011000, 0x40021414, 0x400, 'Black F407ZE'],
];

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdCache = {};
const svdFor = (f) => svdCache[f] || (svdCache[f] = readFileSync(new URL('./vendor/' + f + '.svd', import.meta.url), 'utf8'));

let failures = 0;
for (const [key, svd, flashSize, ramSize, uartAddr, odrAddr, odrMask, label] of BOARDS) {
    const fwPath = new URL(`../arduino_board/build-${key}/arduino_board.ino.bin`, import.meta.url);
    let fwBytes;
    try {
        fwBytes = readFileSync(fwPath);
    } catch (e) {
        console.log(`[${key}] SKIP (no binary — run .pw-scratch/build_arduino.sh first)`);
        continue;
    }
    const emu = await createEmulator({
        firmware: new Uint8Array(fwBytes), bindings, svdXml: svdFor(svd), wasmInit: wasmBytes,
        flash_size: flashSize, ram_size: ramSize, uart_addr: uartAddr, enable_irqs: true,
    });
    let uart = '', toggles = 0, prev = -1;
    // Arduino delay(100) needs ~100 SysTick IRQs each; walk generously.
    for (let i = 0; i < 2500 && !uart.includes('Arduino done'); i++) {
        emu.step(200000);
        uart += emu.drainUart().toString();
        const odr = emu.read32(odrAddr) & odrMask;
        if (prev >= 0 && odr !== prev) toggles++;
        prev = odr;
        if (emu.faultInfo()) break;
    }
    const fault = emu.faultInfo();
    emu.close();
    const ok = uart.includes('=== Arduino Board Test ===') &&
        uart.includes('tick 0 LED=ON') && uart.includes('tick 2 LED=OFF') &&
        uart.includes('Arduino done') && toggles >= 2 && !fault;
    console.log(`[${key}] ${label}: ${ok ? 'ok' : 'FAIL'} (toggles=${toggles} fault=${fault ? JSON.stringify(fault) : 'none'})`);
    if (!ok) {
        failures++;
        console.log('--- uart tail ---\n' + uart.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join('\n'));
    }
}
console.log(failures ? 'FAIL' : 'ARDUINO-BOARDS PASS');
process.exit(failures ? 1 : 0);
