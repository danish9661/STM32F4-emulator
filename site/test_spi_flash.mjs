// Verifies the SPI NOR flash write path (spi_flash.rs rewrite): WEL tracking,
// page program committed on CS deassert, readback, 4k sector erase. The flash
// is attached to SPI3 with a software CS on PB12 (device has cs:"PB12").
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const fw = new Uint8Array(readFileSync(new URL('../spi_flash_test/spi_flash_test.bin', import.meta.url)));
const emu = await createEmulator({
    firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes,
    ext_devices: {
        spi_flash: [{
            peripheral: 'SPI3',
            jedec_id: 0xEF4015,
            size: 0x200000, // 2 MB
            cs: 'PB12',
            data: new Uint8Array(0x200000).fill(0xFF),
        }],
    },
});

let uart = '';
let done = false;
for (let i = 0; i < 4000 && !done; i++) {
    emu.step(50000);
    uart += emu.drainUart();
    if (uart.includes('SPI FLASH TEST DONE')) done = true;
}

const ok = done && !uart.includes('FAIL ');
console.log(ok ? 'PASS' : 'FAIL');
console.log(uart);
emu.close();
process.exit(ok ? 0 : 1);
