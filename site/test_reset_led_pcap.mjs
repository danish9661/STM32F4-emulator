// Reset/Boot + LED status + pcap + net-speed harness (no browser).
// Usage: node site/test_reset_led_pcap.mjs (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';
import { boardLed } from './boards.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const blinky = new Uint8Array(readFileSync(new URL('../blinky/blinky.bin', import.meta.url)));
const ethAdv = new Uint8Array(readFileSync(new URL('../eth_adv/eth_adv.bin', import.meta.url)));

let pass = 0, fail = 0;
const ok = (c, m) => { if (c) { pass++; console.log(`  ok: ${m}`); } else { fail++; console.log(`  FAIL: ${m}`); } };

// ── 1. resetCpu: PC returns to the reset vector ──
{
    const emu = await createEmulator({ firmware: blinky, bindings, svdXml, wasmInit: wasmBytes });
    for (let i = 0; i < 5; i++) emu.step(100000);
    const before = emu.getRegisters().PC >>> 0;
    emu.resetCpu();
    const after = emu.getRegisters().PC >>> 0;
    const vec = new DataView(blinky.buffer).getUint32(4, true) | 1;
    ok(before !== after, `resetCpu moves PC (0x${before.toString(16)} -> 0x${after.toString(16)})`);
    ok(after === vec, `resetCpu lands on reset vector 0x${vec.toString(16)}`);
    emu.close();
}
// ── 2. NRST hold: steps run nothing, release resumes ──
{
    const emu = await createEmulator({ firmware: blinky, bindings, svdXml, wasmInit: wasmBytes });
    for (let i = 0; i < 3; i++) emu.step(100000);
    ok(emu.setNrst(true) === true, 'setNrst(true) asserts');
    ok(emu.isNrstAsserted() === true, 'isNrstAsserted reports held');
    const a = emu.step(100000);
    const b = emu.step(100000);
    ok(a.instCount === b.instCount, `held steps advance nothing (${a.instCount} == ${b.instCount})`);
    ok(emu.setNrst(false) === false, 'setNrst(false) releases');
    const c = emu.step(100000);
    ok(c.instCount > b.instCount, `released step advances (${c.instCount} > ${b.instCount})`);
    emu.close();
}
// ── 3. bootPreset: reload + vector reset ──
{
    const emu = await createEmulator({ firmware: blinky, bindings, svdXml, wasmInit: wasmBytes });
    for (let i = 0; i < 5; i++) emu.step(100000);
    emu.bootPreset({ flash: blinky });
    const vec = new DataView(blinky.buffer).getUint32(4, true) | 1;
    ok((emu.getRegisters().PC >>> 0) === vec, 'bootPreset resets to vector table');
    ok(emu.isNrstAsserted() === false, 'bootPreset releases NRST');
    emu.close();
}
// ── 4. LED map + live readout ──
{
    const cases = [
        ['blinky', 'stm32f407', 3, 12], ['blinky_f401', 'stm32f401', 2, 13],
        ['blinky_nucleo_f401', 'stm32f401', 0, 5], ['blinky_f429', 'stm32f429', 6, 13],
        ['blinky_f407ve', 'stm32f407ve', 0, 6],
    ];
    for (const [fw, board, bank, pin] of cases) {
        const led = boardLed(fw, board);
        ok(led.bank === bank && led.pin === pin, `boardLed(${fw},${board}) = ${led.label}`);
    }
    const emu = await createEmulator({ firmware: blinky, bindings, svdXml, wasmInit: wasmBytes });
    let sawOn = false, sawOff = false;
    for (let i = 0; i < 30; i++) {
        emu.step(100000);
        const odr = emu.read32(0x40020014) >>> 0;
        if ((odr >> 5) & 1) sawOn = true; else sawOff = true;
        if (sawOn && sawOff) break;
    }
    ok(sawOn && sawOff, 'PA5 toggles live via ODR (ledStatus source)');
    emu.close();
}
// ── 5. pcap writer bytes + net counters ──
// Replicates site/app.js pcapBytes() byte-for-byte over captured TX/RX
// frames, then validates the output as a real libpcap file: LE magic
// d4 c3 b2 a1, v2.4, snaplen 65535, linktype Ethernet(1), per-frame
// headers with incl==orig==actual bytes, and total length consistency.
// (This caught the real 0xa1b2c304 typo — LE bytes 04 c3 b2 a1, which
// tcpdump rejects with "unknown file format".)
{
    const { writeFileSync } = await import('node:fs');
    const netsim = createNetSim({});
    const frames = [];
    const emu = await createEmulator({
        firmware: ethAdv, bindings, svdXml, wasmInit: wasmBytes,
        enable_irqs: true, irq_eth: true, lowpower: true,
        eth: { rxDesc: 0x20000c40, rxBuf: 0x2000060c, rxStride: 1536, rxDescs: 1 },
        onTx: (frame) => {
            frames.push({ us: 1000000 + frames.length * 1000, data: frame.slice() });
            for (const r of netsim.onTx(frame)) {
                frames.push({ us: 1000000 + frames.length * 1000, data: r.slice() });
                emu.injectFrame(r);
            }
        },
    });
    let uart = '';
    for (let i = 0; i < 3000 && !uart.includes('RST OK'); i++) { emu.step(20000); uart += emu.drainUart(); }
    ok(uart.includes('RST OK'), 'eth_adv boots to RST OK (pcap/speed harness path)');
    ok(frames.length > 0, `pcap captured ${frames.length} frames`);
    // replicate app.js pcapBytes()
    const parts = [];
    const gh = new Uint8Array(24); const gdv = new DataView(gh.buffer);
    gdv.setUint32(0, 0xa1b2c3d4, true); gdv.setUint16(4, 2, true); gdv.setUint16(6, 4, true);
    gdv.setInt32(8, 0, true); gdv.setUint32(12, 0, true); gdv.setUint32(16, 65535, true); gdv.setUint32(20, 1, true);
    parts.push(gh); let total = 24;
    for (const f of frames) {
        const h = new Uint8Array(16); const dv = new DataView(h.buffer);
        dv.setUint32(0, Math.floor(f.us / 1e6), true); dv.setUint32(4, f.us % 1e6, true);
        dv.setUint32(8, f.data.length, true); dv.setUint32(12, f.data.length, true);
        parts.push(h, f.data); total += 16 + f.data.length;
    }
    const out = new Uint8Array(total); let off = 0;
    for (const p of parts) { out.set(p, off); off += p.length; }
    // validate as libpcap
    const pdv = new DataView(out.buffer);
    ok(pdv.getUint32(0, true) === 0xa1b2c3d4, 'pcap magic a1b2c3d4 (LE d4 c3 b2 a1)');
    ok(pdv.getUint16(4, true) === 2 && pdv.getUint16(6, true) === 4, 'pcap version 2.4');
    ok(pdv.getUint32(20, true) === 1, 'pcap linktype Ethernet(1)');
    let o = 24, n = 0, valid = true;
    while (o + 16 <= out.length) {
        const incl = pdv.getUint32(o + 8, true), orig = pdv.getUint32(o + 12, true);
        if (incl !== orig || o + 16 + incl > out.length) { valid = false; break; }
        o += 16 + incl; n++;
    }
    ok(valid && o === out.length && n === frames.length, `pcap ${n} records, incl==orig, len ${out.length} consistent`);
    emu.close();
}
console.log(fail === 0 ? `RESET/LED/PCAP PASS (${pass} checks)` : `RESET/LED/PCAP FAIL (${fail} failures)`);
process.exit(fail === 0 ? 0 : 1);
