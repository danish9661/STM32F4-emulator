// Boot-probes candidate firmware binaries: runs each for a few M instructions
// and prints the UART banner (first ~200 chars) + whether TX happened.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const candidates = [
    ['hal_test', '../hal_test/hal_test.ino.bin'],
    ['crypto_test', '../crypto_test/build/crypto_test.ino.bin'],
    ['crypto_deep_test', '../crypto_deep_test/crypto_deep_test.bin'],
    ['rx_crypto_test', '../rx_crypto_test/rx_crypto_test.bin'],
    ['periph_test', '../periph_test/periph_test.ino.bin'],
    ['new_periph_test', '../new_periph_test/new_periph_test.ino.bin'],
    ['deep_periph_test', '../deep_periph_test/build/deep_periph_test.ino.bin'],
    ['comprehensive_test', '../comprehensive_test/comprehensive_test.bin'],
    ['edge_test', '../edge_test/edge_test.ino.bin'],
    ['echo_test', '../echo_test/build/echo_test.ino.bin'],
    ['rx_interrupt_test', '../rx_interrupt_test/build/rx_interrupt_test.ino.bin'],
    ['blink_serial', '../blink_serial/build/blink_serial.ino.bin'],
    ['timer_test', '../timer_test/build/timer_test.ino.bin'],
    ['test_firmware', '../test_firmware/test_firmware.bin'],
    ['arduino_test', '../arduino_test/arduino_test.ino.bin'],
    ['i2s_sai_test', '../i2s_sai_test/i2s_sai_test.bin'],
];

const MAX_INST = 4_000_000;
for (const [name, path] of candidates) {
    let fw;
    try { fw = new Uint8Array(readFileSync(new URL(path, import.meta.url))); }
    catch (e) { console.log(name.padEnd(22), 'MISSING'); continue; }
    const emu = await createEmulator({ firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes });
    let uart = '', txCount = 0;
    let res;
    try {
        for (let i = 0; i < 100 && uart.length < 4000; i++) {
            res = emu.step(40000);
            uart += emu.drainUart();
        }
    } catch (e) { uart += '\n[EMU ERROR] ' + e.message; }
    const lines = uart.replace(/\r/g, '').split('\n').filter(Boolean).slice(0, 2).join(' | ').slice(0, 120);
    console.log(name.padEnd(22), 'tx=' + txCount, res ? 'pc=0x' + res.pc.toString(16) : '', '::', lines.slice(0, 100));
    emu.close();
}
