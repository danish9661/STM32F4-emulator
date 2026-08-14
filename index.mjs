// stm32f4-emulator — Node API entry.
// Wraps the browser/Node-universal emulator.js with the bundled assets
// (SVD, wasm bindings, Unicorn) so Node consumers get a one-call setup.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import * as bindings from './site/vendor/stm32_periph_wasm.js';
import { createEmulator } from './site/emulator.js';
import { createNetSim } from './site/netsim.js';
import { FIRMWARES } from './site/firmware.js';
import { LED, Button, Pwm, I2cRegisterDevice, Potentiometer } from './site/components.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./site/vendor/unicorn_arm.cjs');

const svdXml = readFileSync(new URL('./site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

// Decode a base64-encoded firmware from FIRMWARES.
export function decodeFirmware(key) {
    const fw = FIRMWARES[key];
    if (!fw) throw new Error(`unknown firmware '${key}' (have: ${Object.keys(FIRMWARES).join(', ')})`);
    const bin = atob(fw.bytes);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
    return out;
}

// Convenience: create an emulator with the bundled STM32F407 assets.
//   await createSTM32F407({ firmware })           // firmware: Uint8Array
//   await createSTM32F407({ firmware: 'eth_http' })  // or a FIRMWARES key
// All extra options pass through to createEmulator().
export async function createSTM32F407(opts = {}) {
    const { firmware } = opts;
    const bin = typeof firmware === 'string' ? decodeFirmware(firmware) : firmware;
    if (!bin) throw new Error('createSTM32F407 requires `firmware` (Uint8Array or a FIRMWARES key)');
    return createEmulator({ ...opts, firmware: bin, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes });
}

export { createEmulator, createNetSim, FIRMWARES, bindings, unicornFactory, svdXml, LED, Button, Pwm, I2cRegisterDevice, Potentiometer };
