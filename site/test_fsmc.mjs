// Node harness for the FSMC bank tap, driven by GUEST firmware
// (fsmc_test/fsmc_test.bin) rather than by poking the MMIO bus.
//
// The firmware treats FSMC BANK1 as an Intel-8080-mode display: a store to
// the bank base is a command, a store to base|(1<<17) is data, because A16
// is wired to the display's RS/DC pin. The JS device below is the other side
// of that bus — it decodes the accesses and answers the RDDID read, which is
// the direction that used to be a no-op stub.
// Usage: node site/test_fsmc.mjs   (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../fsmc_test/fsmc_test.bin', import.meta.url)));

const RS = 1 << 17;              // the address line the display reads as RS/DC

const lcd = { cmds: [], pixels: [], reads: 0 };
function lcdHandler(events, pushData) {
    // Events are PAIRS: header word (bit31 = write, bits30..0 = offset),
    // then the value.
    for (let i = 0; i + 1 < events.length; i += 2) {
        const hdr = events[i] >>> 0;
        const val = events[i + 1] >>> 0;
        if (!(hdr & 0x80000000)) { lcd.reads++; continue; }
        const off = hdr & 0x7fffffff;
        if (off & RS) {
            lcd.pixels.push(val & 0xffff);
        } else {
            lcd.cmds.push(val & 0xff);
            if ((val & 0xff) === 0x04) pushData([0x9341]);   // RDDID -> ILI9341
        }
    }
}

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: { fsmcDevices: [{ bank: 0, handler: lcdHandler }] },
});

let uart = '';
for (let i = 0; i < 400 && !uart.includes('FSMC Test: '); i++) {
    emu.step(100000);
    uart += emu.drainUart();
}
const all = uart.replace(/\r/g, '');
console.log('uart:', all.split('\n').filter(Boolean).join(' | '));
console.log('cmds:', lcd.cmds.map((c) => '0x' + c.toString(16)).join(','),
    '| pixels:', lcd.pixels.length, '| device reads:', lcd.reads);

const pass =
    all.includes('=== FSMC Test: done ===') &&
    !all.includes('FAIL') &&
    // The guest's own command sequence reached the device in order.
    JSON.stringify(lcd.cmds) === JSON.stringify([0x2a, 0x2c, 0x04]) &&
    // CASET's 2 params + RAMWR's 6-pixel burst, all on the RS-high address.
    lcd.pixels.length === 8 &&
    lcd.pixels[0] === 0x0000 && lcd.pixels[1] === 0x00ef &&
    lcd.pixels[2] === 0xf800 && lcd.pixels[7] === 0xf805 &&
    lcd.reads > 0;

console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
