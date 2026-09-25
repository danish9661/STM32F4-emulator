// stm32f4-emu — Node API entry.
// Wraps the browser/Node-universal emulator.js with the bundled assets
// (SVD, wasm bindings) so Node consumers get a one-call setup.
import { readFileSync } from 'node:fs';
import * as bindings from './site/vendor/stm32_periph_wasm.js';
import { createEmulator } from './site/emulator.js';
import { createNetSim } from './site/netsim.js';
import { FIRMWARES } from './site/firmware.js';
import { LED, Button, Pwm, I2cRegisterDevice, Potentiometer } from './site/components.js';
import { STM32F4, GPIOPin, GPIO, USART, SPI, I2C, DMAStream, DMAController, Display, CHIPS, chipInfo } from './site/stm32f4.js';

// Node consumers get a one-call setup that injects the bundled assets. The
// `firmware` option is optional (defer to loadBin/loadHex/loadELF after
// create), matching the rp2040js / avr8js ergonomics.
// NOTE: svdXml is deliberately NOT injected here. STM32F4._create resolves
// the SVD per `chip` (F401/F411/F407/F429) itself; injecting the F407 default
// would shadow the chip map (right sizes + IDCODE, wrong registers). A caller
// passing svdXml explicitly still wins (override path in _create).
STM32F4.create = (opts = {}) => STM32F4._create({
    bindings, wasmInit: wasmBytes, ...opts,
});

const svdXml = readFileSync(new URL('./site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

// Decode a base64-encoded firmware from FIRMWARES.
// Uses single-pass Uint8Array.from for fewer allocations than manual loop.
export function decodeFirmware(key) {
    const fw = FIRMWARES[key];
    if (!fw) throw new Error(`unknown firmware '${key}' (have: ${Object.keys(FIRMWARES).join(', ')})`);
    const bin = atob(fw.bytes);
    return Uint8Array.from(bin, c => c.charCodeAt(0));
}

// Convenience: create an emulator with the bundled assets.
//   await createSTM32F407({ firmware })           // firmware: Uint8Array
//   await createSTM32F407({ firmware: 'eth_http' })  // or a FIRMWARES key
//   await createSTM32F407({ firmware, chip: 'stm32f401' })  // other chips
// `chip` resolves SVD + flash/RAM + IDCODE from the facade CHIPS table
// (default stm32f407); explicit svdXml/flash_size/ram_size opts override it.
// All extra options pass through to createEmulator().
export async function createSTM32F407(opts = {}) {
    const { firmware, chip = 'stm32f407' } = opts;
    const bin = typeof firmware === 'string' ? decodeFirmware(firmware) : firmware;
    if (!bin) throw new Error('createSTM32F407 requires `firmware` (Uint8Array or a FIRMWARES key)');
    const info = chipInfo(chip);
    const cSvd = (opts.svdXml !== undefined) ? opts.svdXml
        : readFileSync(new URL(`./site/vendor/${info.svd}`, import.meta.url), 'utf8');
    return createEmulator({
        ...opts, firmware: bin, bindings, wasmInit: wasmBytes,
        svdXml: cSvd, svdFile: info.svd,
        flash_size: opts.flash_size ?? info.flash_size,
        ram_size: opts.ram_size ?? info.ram_size,
        chipHint: info.svd.replace(/\.svd$/, ''),
    });
}

export { createEmulator, createNetSim, FIRMWARES, bindings, svdXml, LED, Button, Pwm, I2cRegisterDevice, Potentiometer, STM32F4, GPIOPin, GPIO, USART, SPI, I2C, DMAStream, DMAController, Display, CHIPS, chipInfo };
export { boardLed, BOARD_LED, BOARD_LED_ALIASES } from './site/boards.js';
