// End-to-end USB OTG FS device test: the guest (usb_cdc_test firmware,
// CDC-ACM echo, polling) enumerates against THIS file playing USB host
// through the model's usb_* exports (reset, enum-done, SETUP/OUT inject,
// IN take) — the netsim pattern, but for USB control transfers.
// Flow: init -> reset -> enum-done -> GET_DESCRIPTOR(device/config/string)
// -> SET_ADDRESS -> SET_CONFIGURATION ("USB enum done") -> CDC line coding
// -> 2x bulk echo ("USB echo OK").
// Usage: node site/test_usb.mjs  (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../usb_cdc_test/usb_cdc_test.bin', import.meta.url)));

function fail(msg) { console.error('USB FAIL: ' + msg); process.exit(1); }
const eq = (a, b, what) => {
    const x = Array.from(a), y = Array.from(b);
    if (x.length !== y.length || x.some((v, i) => v !== y[i]))
        fail(`${what}: got [${x}] want [${y}]`);
};

(async () => {
    const emu = await createEmulator({ firmware, bindings, svdXml, wasmInit: wasmBytes });
    let uart = '';
    const step = (n = 20000) => { emu.step(n); uart += emu.drainUart().toString(); };
    const waitFor = (marker, budget = 300) => {
        for (let i = 0; i < budget && !uart.includes(marker); i++) step();
        if (!uart.includes(marker)) fail(`timeout waiting for ${JSON.stringify(marker)}\n--- uart ---\n${uart}`);
    };
    // IN take with bounded wait: poll the model status, stepping the guest.
    const takeIn = (ep, budget = 300) => {
        for (let i = 0; i < budget; i++) {
            const st = bindings.usb_in_status(ep);
            if (st === 2) fail(`EP${ep} answered STALL`);
            if (st === 1) return bindings.usb_take_in(ep);
            step();
        }
        fail(`timeout waiting for IN data on EP${ep}\n--- uart ---\n${uart}`);
    };

    waitFor('USB init done');
    bindings.usb_reset();
    waitFor('USBRST');
    bindings.usb_enumerated();
    waitFor('ENUMDNE');

    // SETUP exchange: inject the 8-byte SETUP, take the device's IN reply
    // (data blob, or the status ZLP when there is no data stage).
    const setupXfer = (setup) => {
        bindings.usb_inject_setup(Uint8Array.from(setup));
        return takeIn(0);
    };
    // Status-stage OUT ZLP after an IN data transfer (no reply to take).
    const statusOut = (budget = 200) => {
        bindings.usb_inject_out(0, new Uint8Array(0));
        for (let i = 0; i < budget; i++) step();
    };

    // GET_DESCRIPTOR device (18B), then its status OUT.
    let blob = setupXfer([0x80, 0x06, 0x00, 0x01, 0x00, 0x00, 0x40, 0x00]);
    eq(blob.slice(0, 18), [18, 1, 0, 2, 2, 0, 0, 64, 0x83, 4, 0x40, 0x57, 0, 2, 1, 2, 3, 1], 'device desc');
    statusOut();

    // GET_DESCRIPTOR configuration (67B over 2 packets; the word-pad
    // byte never travels — DIEPTSIZ.XFRSIZ rules the transfer length).
    blob = setupXfer([0x80, 0x06, 0x00, 0x02, 0x00, 0x00, 0xFF, 0x00]);
    if (blob[1] !== 2) fail(`config desc type ${blob[1]}`);
    const total = blob[2] | (blob[3] << 8);
    if (total !== 67) fail(`wTotalLength ${total}`);
    if (blob.length !== 67) fail(`config length ${blob.length}`);
    if (blob[4] !== 2) fail(`bNumInterfaces ${blob[4]}`);
    statusOut();

    // GET_DESCRIPTOR string 0 (lang).
    blob = setupXfer([0x80, 0x06, 0x00, 0x03, 0x00, 0x00, 0xFF, 0x00]);
    eq(blob.slice(0, 4), [4, 3, 9, 4], 'lang string');
    statusOut();

    // SET_ADDRESS(5): device answers its own status ZLP (already taken).
    blob = setupXfer([0x00, 0x05, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00]);
    if (blob.length !== 0) fail(`SET_ADDRESS should ZLP, got ${blob.length}B`);

    // SET_CONFIGURATION(1) -> "USB enum done".
    blob = setupXfer([0x00, 0x09, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00]);
    if (blob.length !== 0) fail(`SET_CONFIGURATION should ZLP, got ${blob.length}B`);
    waitFor('USB enum done');

    // CDC: SET_CONTROL_LINE_STATE then SET_LINE_CODING (7-byte OUT).
    blob = setupXfer([0x21, 0x22, 0x03, 0x00, 0x00, 0x00, 0x00, 0x00]);
    if (blob.length !== 0) fail(`SET_LINE_STATE should ZLP, got ${blob.length}B`);
    bindings.usb_inject_setup(Uint8Array.from([0x21, 0x20, 0, 0, 7, 0, 0, 0]));
    bindings.usb_inject_out(0, Uint8Array.from([0x00, 0xC2, 0x01, 0x00, 0x00, 0x00, 0x08]));
    blob = takeIn(0); // status ZLP after the 7-byte data stage
    if (blob.length !== 0) fail(`SET_LINE_CODING status should ZLP, got ${blob.length}B`);

    // Bulk echo x2 on EP1.
    for (const round of [[72, 101, 108, 108, 111, 85, 83, 66], [87, 111, 114, 108, 100, 33, 33, 33]]) {
        bindings.usb_inject_out(1, Uint8Array.from(round));
        const echo = takeIn(1, 400);
        eq(echo, round, `echo round`);
    }
    waitFor('USB echo OK');

    const need = ['=== USB CDC Test ===', 'USB init done', 'USBRST', 'ENUMDNE',
        'USB enum done', 'USB echo 1', 'USB echo 2', 'USB echo OK', 'USB done'];
    const missing = need.filter((m) => !uart.includes(m));
    if (missing.length) fail('missing markers: ' + missing.join(', ') + '\n--- uart ---\n' + uart);
    if (uart.includes('USB FAIL')) fail('firmware reported FAIL\n--- uart ---\n' + uart);
    console.log('USB PASS');
    emu.close();
    process.exit(0);
})().catch((e) => fail(e.stack || String(e)));
