// Base STM32F4 facade tests: firmware loading (bin/hex/elf), GPIO/USART
// event wiring, and pass-through accessors. Run: `node site/test_stm32f4_api.mjs`
import { readFileSync, existsSync } from 'node:fs';
import { STM32F4, decodeFirmware } from '../index.mjs';

let failures = 0;
function check(cond, msg) {
    if (!cond) { failures++; console.error('FAIL:', msg); }
    else console.log('ok  :', msg);
}

// Minimal correct Intel HEX converter (extended-linear base + data + EOF).
function byteSum(arr) { let s = 0; for (const b of arr) s = (s + b) & 0xFF; return s; }
function rec(count, addr, type, data) {
    const body = [count, (addr >> 8) & 0xFF, addr & 0xFF, type, ...data];
    const cs = (0x100 - byteSum(body)) & 0xFF;
    return ':' + body.map((b) => b.toString(16).padStart(2, '0')).join('') + cs.toString(16).padStart(2, '0');
}
function binToHex(bytes, base = 0x08000000) {
    const out = [];
    const upper = Math.floor(base / 0x10000);
    out.push(rec(2, 0, 4, [(upper >> 8) & 0xFF, upper & 0xFF]));
    for (let i = 0; i < bytes.length; i += 16) {
        const chunk = bytes.slice(i, i + 16);
        out.push(rec(chunk.length, (base + i) & 0xFFFF, 0, [...chunk]));
    }
    out.push(rec(0, 0, 1, []));
    return out.join('\n') + '\n';
}

const blinky = decodeFirmware('blinky');

// ── Test 1: loadBin + GPIO/USART events ──
const mcu = await STM32F4.create();
const ledStates = [];
mcu.gpio.pin('A', 5).on('change', (v) => ledStates.push(v));
let uart = '';
mcu.usart.onData = (b) => { uart += String.fromCharCode(b); };
mcu.loadBin(blinky);
check(mcu.read32(0x08000000) !== 0, 'loadBin: vector SP nonzero');

for (let i = 0; i < 400 && !(uart.includes('LED=ON') && uart.includes('LED=OFF')); i++) mcu.execute(50000);
check(uart.includes('LED=ON'), 'loadBin: UART prints LED=ON');
check(uart.includes('LED=OFF'), 'loadBin: UART prints LED=OFF');
check(ledStates.includes(true) && ledStates.includes(false), 'loadBin: PA5 toggled (gpio change events)');

// ── Test 2: loadHex path ──
let uart2 = '';
mcu.usart.onData = (b) => { uart2 += String.fromCharCode(b); };
mcu.loadHex(binToHex(blinky));
for (let i = 0; i < 400 && !uart2.includes('LED=ON'); i++) mcu.execute(50000);
check(uart2.includes('LED=ON'), 'loadHex: boots blinky (LED=ON)');

// ── Test 3: loadELF path (if a built .elf is present) ──
const elfPath = existsSync('qspi_test/qspi_test.elf') ? 'qspi_test/qspi_test.elf'
    : (existsSync('blinky/blinky.elf') ? 'blinky/blinky.elf' : null);
if (elfPath) {
    mcu.loadELF(readFileSync(elfPath));
    check(mcu.read32(0x08000000) !== 0, `loadELF: SP set from vector (${elfPath})`);
} else {
    console.log('skip: no .elf available for loadELF test');
}
mcu.close();

// ── Test 4: create with firmware option + DMAStream ──
const mcu2 = await STM32F4.create({ firmware: blinky });
check(mcu2.read32(0x08000000) !== 0, 'create({firmware}): vector SP nonzero');
check(typeof mcu2.dma.stream(0).pendingCount() === 'number', 'dma.stream(0).pendingCount() returns a number');
check(typeof mcu2.getRegisters().PC === 'number', 'getRegisters() returns PC');
mcu2.close();

if (failures) { console.error(`\n${failures} FAILED`); process.exit(1); }
console.log('\nALL PASS');
