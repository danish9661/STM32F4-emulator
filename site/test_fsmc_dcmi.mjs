// Node harness for the two peripherals that had no external-device path:
//
//  - FSMC: read_data/write_data were no-op stubs after the old Rust-side
//    ext_devices/{lcd,display}.rs were dropped. They now forward to a
//    protocol-agnostic bank tap (fsmc_tap), mirroring spi_tap. This drives
//    a memory-mapped ILI9341-style display in Intel-8080 mode: command at
//    bank offset 0, pixel data at the RS/DC-decoded address line, and a
//    register read answered from the device side.
//
//  - DCMI: the model already consumed JS-fed frames with real LINE/FRAME
//    semantics, but nothing fed it. `ext_devices.camera` is now a real
//    source pumped every step, and emu.camera.feed() injects directly.
//
// No firmware exercises either yet (there is no FSMC/DCMI firmware in the
// tree), so this drives the peripherals over the MMIO bus the same way the
// CPU core would, through a booted emulator.
// Usage: node site/test_fsmc_dcmi.mjs   (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));

const FSMC_BANK1 = 0x60000000;   // bank 0 data window
const RS = 0x20000;              // address line the display decodes as data/cmd
const DCMI = 0x50050000;
// The DCMI FIFO is 4 deep and the model consumes 16 px/tick, so a frame
// wider than the FIFO overruns a *polling* drain — that is what real DCMI
// does too, and why real firmware uses DMA. The live camera therefore uses a
// FIFO-sized frame (content is checkable), and the overflow path is asserted
// separately with a larger one.
const CAM_W = 2, CAM_H = 2;

// ── The JS display device: decodes the 8080-mode bus into commands+pixels ──
const lcd = { cmds: [], pixels: [], lastCmd: null };
function lcdHandler(events, push) {
    // Events are PAIRS: header word (bit31 = write, bits30..0 = offset)
    // then the value.
    for (let i = 0; i + 1 < events.length; i += 2) {
        const hdr = events[i] >>> 0;
        const val = events[i + 1] >>> 0;
        const isWrite = (hdr & 0x80000000) !== 0;
        const off = hdr & 0x7fffffff;
        if (!isWrite) continue;                 // reads are traced, not driven
        if (off & RS) {
            lcd.pixels.push(val & 0xffff);      // data phase
        } else {
            lcd.cmds.push(val & 0xff);          // command phase
            lcd.lastCmd = val & 0xff;
            if ((val & 0xff) === 0x04) push([0x009341]);  // RDDID -> ILI9341
        }
    }
}

// ── The JS camera: a moving vertical bar, one new frame per call ──
function cameraFrame(n) {
    const px = new Uint8Array(CAM_W * CAM_H);
    for (let y = 0; y < CAM_H; y++) px[y * CAM_W + (n % CAM_W)] = 0xff;
    return px;
}

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    ext_devices: {
        fsmcDevices: [{ bank: 0, handler: lcdHandler }],
        camera: { width: CAM_W, height: CAM_H, frame: cameraFrame },
    },
});

const fails = [];
const check = (name, cond, extra = '') => {
    console.log(`${cond ? 'ok  ' : 'FAIL'}  ${name}${extra ? '  ' + extra : ''}`);
    if (!cond) fails.push(name);
};

// ── FSMC: an 8080-mode display init + pixel burst ──
// Driven straight over the MMIO bus, which is the same entry point the CPU
// core's memory hook uses.
const { periph_write, periph_read } = bindings;

periph_write(FSMC_BANK1 + 0, 4, 0x2A);            // CASET
periph_write(FSMC_BANK1 + RS, 4, 0x0000);
periph_write(FSMC_BANK1 + RS, 4, 0x00EF);
periph_write(FSMC_BANK1 + 0, 4, 0x2C);            // RAMWR
for (let i = 0; i < 6; i++) periph_write(FSMC_BANK1 + RS, 4, 0xF800 + i);
emu.step(1000);

check('fsmc forwards commands', JSON.stringify(lcd.cmds) === '[42,44]',
    `cmds=[${lcd.cmds.map((c) => '0x' + c.toString(16)).join(',')}]`);
check('fsmc separates data by address line',
    lcd.pixels.length === 8 && lcd.pixels[0] === 0x0000 && lcd.pixels[2] === 0xF800,
    `pixels=${lcd.pixels.length} first=0x${lcd.pixels[0].toString(16)}`);

// Device-answered read: RDDID pushes a value, the next bank read returns it.
periph_write(FSMC_BANK1 + 0, 4, 0x04);
emu.step(1000);                                    // handler runs, pushes 0x9341
const id = periph_read(FSMC_BANK1 + RS, 4);
check('fsmc read answered by the device', id === 0x9341, `id=0x${id.toString(16)}`);

// An untapped bank must stay inert rather than aliasing bank 0.
periph_write(0x70000000, 4, 0xDEAD);
check('untapped bank reads 0', periph_read(0x70000000, 4) === 0);

// ── DCMI: the camera source drives a real capture ──
emu.step(1000);                                    // pump at least one frame
periph_write(DCMI + 0x0C, 4, 0x1F);                // IER: enable all
periph_write(DCMI + 0x00, 4, 0x01);                // CR: CAPTURE

const got = [];
for (let i = 0; i < 200 && !got.some((v) => v === 0xff); i++) {
    emu.step(2000);
    // Drain the FIFO through DR the way a firmware polling loop would.
    for (let k = 0; k < CAM_W * CAM_H; k++) got.push(periph_read(DCMI + 0x28, 4) & 0xff);
    periph_write(DCMI + 0x00, 4, 0x00);            // CAPTURE auto-clears per frame;
    periph_write(DCMI + 0x00, 4, 0x01);            // re-arm with a fresh rising edge
}
const ris = periph_read(DCMI + 0x08, 4);
check('dcmi camera delivered pixels', got.some((v) => v === 0xff),
    `nonzero=${got.filter((v) => v).length}/${got.length}`);
check('dcmi LINE flag set', (ris & (1 << 1)) !== 0, `ris=0x${ris.toString(16)}`);
check('dcmi FRAME flag set', (ris & (1 << 2)) !== 0);
check('camera fed multiple frames', emu.camera.frames > 1, `frames=${emu.camera.frames}`);

// Direct injection path, independent of ext_devices.camera: exact pixels.
emu.camera.stop();
emu.camera.feed(2, 2, new Uint8Array([1, 2, 3, 4]));
periph_write(DCMI + 0x10, 4, 0x1F);                // clear flags
periph_write(DCMI + 0x00, 4, 0x00);                // CAPTURE low: the model
periph_write(DCMI + 0x00, 4, 0x01);                // reloads on the RISING edge
emu.step(5000);
const direct = [0, 1, 2, 3].map(() => periph_read(DCMI + 0x28, 4) & 0xff);
check('camera.feed injects a frame directly', JSON.stringify(direct) === '[1,2,3,4]',
    `dr=[${direct.join(',')}]`);

// A frame larger than the FIFO, drained too slowly, must flag OVR (bit 3) —
// the reason real capture drivers use DMA rather than polling DR.
emu.camera.feed(8, 4, Uint8Array.from({ length: 32 }, (_, i) => i + 1));
periph_write(DCMI + 0x10, 4, 0x1F);                // clear flags
periph_write(DCMI + 0x00, 4, 0x00);
periph_write(DCMI + 0x00, 4, 0x01);                // CAPTURE (rising edge)
emu.step(20000);                                   // consume without draining
const ovr = periph_read(DCMI + 0x08, 4);
check('dcmi flags OVR when the FIFO overruns', (ovr & (1 << 3)) !== 0,
    `ris=0x${ovr.toString(16)}`);

emu.close();
console.log(fails.length ? `FAIL (${fails.join(', ')})` : 'PASS');
process.exit(fails.length ? 1 : 0);
