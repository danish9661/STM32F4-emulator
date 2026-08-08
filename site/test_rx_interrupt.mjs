// Verifies the opt-in interrupt pump (enable_irqs) for the RX firmware:
// USART1 RXNEIE handlers must run for the firmware to make progress.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

const cases = [
    ['rx_interrupt_test', '../rx_interrupt_test/build/rx_interrupt_test.ino.bin', 'CRC='],
    ['rx_crypto_test', '../rx_crypto_test/rx_crypto_test.bin', 'DONE'],
];

let fail = 0;
for (const [name, path, marker] of cases) {
    const fw = new Uint8Array(readFileSync(new URL(path, import.meta.url)));
    const emu = await createEmulator({ firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes, enable_irqs: true });
    let uart = '';
    emu.sendUart(new TextEncoder().encode('Hello\n'));
    let ok = false;
    for (let i = 0; i < 3000 && !ok; i++) {
        emu.step(50000);
        uart += emu.drainUart();
        if (uart.includes(marker)) ok = true;
    }
    console.log(`${name}: ${ok ? 'PASS' : 'FAIL'}`);
    if (!ok) console.log('  uart tail:', JSON.stringify(uart.slice(-200)));
    if (!ok) fail++;
    emu.close();
}
process.exit(fail ? 1 : 0);
