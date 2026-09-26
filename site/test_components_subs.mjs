// Substitute-component tests: the eight hardware-gap classes in
// site/components.js, each against the REAL Rust model (wasm bindings +
// SVD map, no emulator.js driver) — the same pattern as
// test_periph_mock_consumer.mjs, but through the public component API
// (and through STM32F4 facade methods where they exist).
// Each check asserts the SUBSTITUTE the gap docs promise (NOT silicon).
// Usage: node site/test_components_subs.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import {
    CameraSensor, DacLoad, RngNoise, I2cPeer, UsbLink, UlpiMeter,
    UartBaud, NandEcc,
} from './components.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
if (typeof bindings.default === 'function') await bindings.default({ module_or_path: wasmBytes });
bindings.init_svd(svdXml);

let failures = 0;
function check(cond, msg) {
    if (!cond) { failures++; console.error('FAIL:', msg); }
    else console.log('ok  :', msg);
}

// ── CameraSensor: gradient + noise are real pixel data ──
{
    const s = new CameraSensor({ camera: { feed() {} } }, { width: 8, height: 4, scene: 'gradient', noise: 0 });
    // Minimal emu stub: only .camera.feed is touched.
    let fed = null;
    s.emu = { camera: { feed: (w, h, px) => { fed = { w, h, px }; } } };
    check(s.capture() === true, 'camera: capture() feeds');
    check(fed && fed.w === 8 && fed.h === 4 && fed.px.length === 32, 'camera: 8x4 frame, 32 px');
    check(fed.px[0] < fed.px[31], 'camera: gradient rises across the frame');
    const n = new CameraSensor({ camera: { feed() {} } }, { width: 4, height: 4, scene: 'noise', noise: 0 });
    n.emu = { camera: { feed: (w, h, px) => { fed = px; } } };
    n.capture();
    check(new Set(fed).size > 4, 'camera: noise scene varies');
}

// ── DacLoad: DOR code -> voltage ──
{
    bindings.periph_write(0x40007400, 4, 1); // DAC CR EN1
    bindings.periph_write(0x40007408, 4, 0xABC); // DHR12R1
    const emuStub = { read32: (a) => bindings.periph_read(a, 4), dacTrigger() {}, dacUnderrun: () => false };
    const d = new DacLoad(emuStub, { channel: 1, vref: 3.3 });
    check(d.code === 0xABC, `dac: DOR readback 0xABC (got 0x${d.code.toString(16)})`);
    const v = d.voltage;
    check(Math.abs(v - (0xABC / 4095) * 3.3) < 1e-9, `dac: voltage ${(v).toFixed(4)} V from code`);
    check(d.underrun === false, 'dac: no underrun when idle');
}

// ── RngNoise: seeded pool feeds DR (deterministic test source) ──
{
    bindings.periph_write(0x40023800, 4, bindings.periph_read(0x40023800, 4) | (1 << 31)); // RCC AHB2ENR RNGEN (bit31? keep simple)
    const emuStub = {
        rngSeedEntropy: (w) => bindings.rng_seed_entropy(w),
        rngEntropyAvail: () => bindings.rng_entropy_avail(),
    };
    const r = new RngNoise(emuStub, { source: 'seed', seed: 42 });
    r.seed(4);
    check(r.avail === 4, `rng: pool depth 4 after seed (got ${r.avail})`);
}

// ── I2cPeer: arms never throw, route to the right export ──
{
    const calls = [];
    const emuStub = {
        i2cArmArbLoss: (b) => calls.push(['arb', b]),
        i2cArmSmbusAlert: (b, a) => calls.push(['alert', b, a]),
    };
    const p = new I2cPeer(emuStub, { base: 0x40005400 });
    p.loseNextArbitration();
    p.alert(0x2A);
    check(calls.length === 2 && calls[1][2] === 0x2A, 'i2c: arb-loss + alert arms route');
}

// ── UsbLink: VBUS plug/unplug + frame/rate reads ──
{
    const emuStub = {
        usbSetVbus: (v) => bindings.usb_set_vbus(v),
        usbHsSetVbus: (v) => bindings.usb_hs_set_vbus(v),
        usbFrame: () => bindings.usb_uframe(),
        usbHsFrame: () => bindings.usb_hs_uframe(),
        usbUlpiRate: () => bindings.usb_ulpi_rate(),
        usbHsUlpiRate: () => bindings.usb_hs_ulpi_rate(),
    };
    const l = new UsbLink(emuStub, { hs: false });
    l.unplug(); l.plug();
    check(typeof l.frame === 'number', 'usb: frame counter readable');
    check(l.ulpiRate === 12, `usb: FS ULPI rate 12 (got ${l.ulpiRate})`);
    const h = new UsbLink(emuStub, { hs: true });
    check(h.ulpiRate === 480, `usb: HS ULPI rate 480 (got ${h.ulpiRate})`);
}

// ── UlpiMeter: budget math from the link rate ──
{
    const emuStub = { usbUlpiRate: () => 12, usbHsUlpiRate: () => 480 };
    const m = new UlpiMeter(emuStub, { hs: false });
    check(m.rateMbps === 12, 'ulpi: FS 12 Mbit/s');
    check(m.budgetBytes(1000) === 1500, `ulpi: 1ms@12Mbit = 1500 B (got ${m.budgetBytes(1000)})`);
    const h = new UlpiMeter(emuStub, { hs: true });
    check(h.budgetBytes(125) === 7500, `ulpi: 125us@480Mbit = 7500 B (got ${h.budgetBytes(125)})`);
}

// ── UartBaud: BRR/OVER8/clock -> programmed bit rate ──
{
    const emuStub = { read32: (a) => bindings.periph_read(a, 4), uartTxLen: () => 0 };
    bindings.periph_write(0x40011000 + 0x08, 4, 0x222E); // BRR mantissa 546.875 -> 9600 @84MHz/16
    bindings.periph_write(0x40011000 + 0x0C, 4, 0); // CR1 OVER8=0
    const u = new UartBaud(emuStub, { base: 0x40011000, clockHz: 84000000 });
    const baud = u.baud;
    check(Math.abs(baud - 9600) < 500, `uart: BRR mantissa 546.9 @84MHz ~= 9600 (got ${baud})`);
}

// ── NandEcc: round-trip contract (order-sensitive, reset on enable) ──
{
    const e = new NandEcc();
    e.enable();
    const a1 = e.write(0x1234);
    const a2 = e.write(0x5678);
    check(a1 !== a2, 'ecc: parity advances per write');
    const e2 = new NandEcc();
    e2.enable();
    e2.write(0x5678); e2.write(0x1234);
    check(e2.eccr !== e.eccr, 'ecc: order-sensitive (reversed writes differ)');
    e.enable(); // reset
    check(e.eccr === 0, 'ecc: enable resets parity');
}

if (failures) { console.error(`\n${failures} FAILED`); process.exit(1); }
console.log('\nALL PASS');
