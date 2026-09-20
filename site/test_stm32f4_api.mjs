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
const elfPath = existsSync('firmware/qspi_test/qspi_test.elf') ? 'firmware/qspi_test/qspi_test.elf'
    : (existsSync('firmware/blinky/blinky.elf') ? 'firmware/blinky/blinky.elf' : null);
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
// F1-parity: execute/step result shapes, index-port gpio, unsub, uartRx/output
const r = mcu2.execute(50000);
check(typeof r.instCount === 'number' && typeof r.stopped === 'boolean', 'execute() returns {instCount, stopped}');
const s = mcu2.step(1000);
check(typeof s.pc === 'number' && typeof s.instCount === 'number', 'step() returns {pc, instCount, stopped}');
let fired = 0;
const unsub = mcu2.gpio.pin(0, 5).on('change', () => fired++);
check(typeof unsub === 'function', "gpio.pin(0,5): index port + on() returns unsub");
unsub();
check(typeof mcu2.uartRx(0x41) === 'boolean', 'uartRx(byte) callable');
check(typeof mcu2.uartOutput === 'string', 'uartOutput is a string');
check(mcu2.usart1 && mcu2.usart6 && mcu2.usarts[6] === mcu2.usart6, 'usart1..6 + usarts map present');
check(mcu2.spi1 && mcu2.spiBus[2] === mcu2.spi2, 'spi1..3 + spiBus present');
check(mcu2.i2c1 && mcu2.i2cBus[3] === mcu2.i2c3, 'i2c1..3 + i2cBus present');
mcu2.spi2.injectMiso([0xAA]);
mcu2.i2c1.injectRx([0x55]);
check(true, 'spi2.injectMiso / i2c1.injectRx callable');
check(mcu2.memRead32(0x08000000) !== 0, 'memRead32() reads flash');
check(mcu2.periphRead(0x40011000) !== undefined, 'periphRead() callable');
mcu2.periphWrite(0x40011000, 4, 0);
check(mcu2.getPc() === mcu2.getRegisters().PC, 'getPc() matches getRegisters().PC');
check(mcu2.getSp() === mcu2.getRegisters().SP, 'getSp() matches getRegisters().SP');
check(mcu2.takeFault() === null, 'takeFault() null on clean run');
mcu2.close();

// ── Test 5: fromBin/fromHex factories + setSymbols/resolveSymbol ──
const mcu3 = await STM32F4.fromBin(blinky);
check(mcu3.read32(0x08000000) !== 0, 'fromBin: vector SP nonzero');
mcu3.close();
const mcu4 = await STM32F4.fromHex(binToHex(blinky));
check(mcu4.read32(0x08000000) !== 0, 'fromHex: vector SP nonzero');
const nSym = mcu4.setSymbols('0x08000185                _start\n0x08000200                main\n');
check(nSym === 2, 'setSymbols() parses 2 symbols');
check(mcu4.resolveSymbol(0x08000185) === '_start', 'resolveSymbol() exact hit');
check(mcu4.resolveSymbol(0x08000190) === '_start+0xb', 'resolveSymbol() nearest+offset');
check(mcu4.resolveSymbol(0x08000000) === null, 'resolveSymbol() null below range');
mcu4.close();

// ── Test 6: platform integration surface (Wokwi/OpenHW/Velxio) ──
const mcu5 = await STM32F4.create({ firmware: blinky });
// callbacks exist, default null
check(mcu5.onExtiEdge === null && mcu5.onCanTx === null && mcu5.onCanRx === null, 'callbacks default null (exti/can)');
check(mcu5.onTimUpdate === null && mcu5.onTimCapture === null && mcu5.onDmaTc === null, 'callbacks default null (tim/dma)');
check(mcu5.onWdogReset === null && mcu5.onUsbIn === null && mcu5.onItmByte === null && mcu5.onFsmcAccess === null, 'callbacks default null (wdog/usb/itm/fsmc)');
// debug-port shim
check(mcu5.swdHalted() === false, 'swdHalted() false (no halt path)');
check(mcu5.swdTakeTrip().length === 0, 'swdTakeTrip() empty');
check(mcu5.swdRegRead(15) === mcu5.getPc(), 'swdRegRead(15) == PC');
check(typeof mcu5.jtagIdcode() === 'number', 'jtagIdcode() returns a number');
check(mcu5.addJsPeripheral(0x50000000, 0x100, () => 0, () => {}) === true, 'addJsPeripheral() records region');
check(mcu5.pwrMode() === 0, 'pwrMode() RUN on blinky');
check(mcu5.pwrEstimate() > 0, 'pwrEstimate() positive');
// DMA controller view
const d1 = mcu5.dmaController(1);
check(d1.base === 0x40026000, 'dmaController(1) base DMA1');
const d2 = mcu5.dma.controller(2);
check(d2.base === 0x40026400, 'dma.controller(2) base DMA2');
check(d1.tcif(0) === false && d1.ndtr(0) === 0, 'dma idle: tcif false, ndtr 0');
check(typeof d1.stream(0).cr() === 'number', 'dma stream cr() readable');
// Display surface (no devices enabled: nulls, not throws)
check(mcu5.display.oled === null && mcu5.display.tft === null, 'display oled/tft null when not enabled');
check(mcu5.display.ltdc() === null, 'display ltdc() null when layer off');
// memWriteBytes probe path
mcu5.memWriteBytes(0x20000000, [0x11, 0x22, 0x33, 0x44]);
check((mcu5.memRead32(0x20000000) >>> 0) === 0x44332211, 'memWriteBytes() probe write round-trips');
mcu5.close();

if (failures) { console.error(`\n${failures} FAILED`); process.exit(1); }
console.log('\nALL PASS');
