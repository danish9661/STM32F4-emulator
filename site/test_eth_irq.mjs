// Verifies the interrupt-driven Ethernet path end-to-end:
//   eth_irq_test firmware enables NVIC ETH IRQ 61 + DMAIER; the pump runs
//   ETH_IRQHandler which reads DMASR (TS/RS), sets its own SRAM flag,
//   scans/re-arms RX descriptors. The driver only signals the model
//   (eth_tx_done/eth_rx_done) and injects frames — no SRAM flag writes.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from '../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../site/emulator.js';

const require = createRequire(import.meta.url);
const unicorn = require('../site/vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('../site/vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const fw = new Uint8Array(readFileSync(new URL('../eth_irq_test/eth_irq_test.bin', import.meta.url)));

// Firmware globals (nm eth_irq_test.elf)
const eth = {
    rxDesc: 0x20000050, rxBuf: 0x2000005c, rxStride: 1536, rxDescs: 1,
};

const frames = [];
const emu = await createEmulator({
    firmware: fw, bindings, unicorn, svdXml, wasmInit: wasmBytes,
    enable_irqs: true, irq_eth: true, eth,
    onTx: (pkt) => { frames.push(new Uint8Array(pkt)); },
});

let uart = '';
let fail = false;
let injected = false;
for (let i = 0; i < 600 && !fail; i++) {
    emu.step(40000);
    uart += emu.drainUart();
    // Respond to the PING with a PONG frame as soon as TX is captured
    if (!injected && frames.length >= 1) {
        injected = true;
        const pong = new Uint8Array(60);
        pong[0] = 0x02; pong[5] = 0x02; // dst 02:00:00:00:00:02
        pong[6] = 0x02; pong[11] = 0x01; // src 02:00:00:00:00:01
        pong[12] = 0x12; pong[13] = 0x34; // type 0x1234
        const msg = 'ETH IRQ PONG';
        for (let k = 0; k < msg.length; k++) pong[14 + k] = msg.charCodeAt(k);
        emu.injectFrame(pong);
    }
    if (uart.includes('TX TIMEOUT') || uart.includes('RX TIMEOUT')) fail = true;
}
console.log(uart.replace(/\r/g, ''));
console.log(`TX frames captured: ${frames.length}`);

const txOk = frames.length >= 2 &&
    new TextDecoder().decode(frames[0].subarray(14, 27)).includes('PING') &&
    new TextDecoder().decode(frames[1].subarray(14, 27)).includes('PONG');
const ok = txOk &&
    uart.includes('TX done via IRQ') &&
    uart.includes('PONG TX via IRQ') &&
    uart.includes('RX via IRQ len=60') &&
    uart.includes('ETH IRQ Test: done') &&
    !fail;

console.log(ok ? 'PASS' : 'FAIL');
emu.close();
process.exit(ok ? 0 : 1);
