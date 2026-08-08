// Node verification harness for the browser demo stack.
// Runs the exact same emulator.js + netsim.js the site uses, and asserts
// the full DHCP -> TCP -> HTTP flow completes.
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../eth_http/eth_http.bin', import.meta.url)));

const maxInst = Number(process.env.MAX_INST || 20_000_000);
const netsim = createNetSim({ log: (m) => console.log('[netsim] ' + m) });

const emu = await createEmulator({
    firmware,
    bindings,
    unicorn: unicornFactory,
    svdXml,
    wasmInit: wasmBytes,
    onTx: (frame) => {
        console.log(`[TX] ${frame.length}B`);
        for (const reply of netsim.onTx(frame)) emu.injectFrame(reply);
    },
});

let uart = '';
let rounds = 0;
let inst = 0;
const t0 = Date.now();
let steps = 0;
let lastPc = '';
while (true) {
    const res = emu.step();
    steps++;
    inst = res.instCount;
    uart += emu.drainUart();
    if (uart.length > 100_000) uart = uart.slice(-100_000);
    rounds = (uart.match(/=== HTTP \d+b ===/g) || []).length;
    if (inst >= maxInst) break;
    if (rounds >= 2) break;
    if (steps % 2000 === 0) {
        const pc = emu.getRegisters().PC;
        if (pc === lastPc && steps > 2000) break; // PC frozen -> stuck
        lastPc = pc;
    }
}
const dt = Date.now() - t0;
const pcFinal = emu.getRegisters().PC;
emu.close();

console.log('── UART (tail) ──');
console.log(uart.slice(-600).replace(/\r/g, ''));

const okBoot = uart.includes('=== ETH HTTP GET Test ===');
const okDhcp = uart.includes('DHCP Ack IP=192.168.004.002 OK');
const okTcp = uart.includes('TCP connected');
const okHttp = uart.includes('Hello from openhw HTTP server');
const okFin = uart.includes('!CONN');
const ok = okBoot && okDhcp && okTcp && okHttp && okFin && rounds >= 2;

console.log(`\n── RESULT ──`);
console.log(`inst=${inst} steps=${steps} rounds=${rounds} wall=${(dt / 1000).toFixed(2)}s PC=0x${(pcFinal >>> 0).toString(16)}`);
console.log(`boot=${okBoot} dhcp=${okDhcp} tcp=${okTcp} http=${okHttp} fin=${okFin}`);
console.log(`netsim stats: ${JSON.stringify(netsim.stats)}`);
console.log(ok ? 'PASS' : 'FAIL');
process.exit(ok ? 0 : 1);
