// Cross-board matrix: every portable demo build on every compatible map.
// Results define BOARDS_OF_FIRMWARE for the web UI (site/boards.js) — only
// pairs listed here (and passing here) get presets. Entry shape:
//   [demo, boards, binRel, markers, anti, opts, script, iters, step]
// scripts: exti | timcap | canhost | echo | wav | dcmi | null (usb uses
// test_usb.mjs with a bin argv, run separately — see below).
// Usage: node site/test_board_matrix.mjs [substr-filter...]  (exit 0 = all pass)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdCache = {};
const svdFor = (f) => svdCache[f] || (svdCache[f] = readFileSync(new URL('./vendor/' + f + '.svd', import.meta.url), 'utf8'));

const BOARDS = {
    f401: { svd: 'stm32f401', flash: 0x80000, ram: 0x18000 },
    f411: { svd: 'stm32f411', flash: 0x80000, ram: 0x20000 },
    f429: { svd: 'stm32f429', flash: 0x200000, ram: 0x40000 },
};
// Arduino per-board builds (8 FQBN keys -> map + sizes).
const ABOARDS = {
    bp_f401cc: { ...BOARDS.f401 }, bp_f411ce: { ...BOARDS.f411 },
    nucleo_f401re: { ...BOARDS.f401 }, nucleo_f411re: { ...BOARDS.f411 },
    disco_f407vg: { svd: 'stm32f407', flash: 0x100000, ram: 0x30000 },
    disco_f429zi: { ...BOARDS.f429 },
    black_f407ve: { svd: 'stm32f407', flash: 0x80000, ram: 0x30000 },
    black_f407ze: { svd: 'stm32f407', flash: 0x100000, ram: 0x30000 },
};
const AKEYS = Object.keys(ABOARDS);

// Shared device configs (mirror app.js DEVICE_FIRMWARES shapes).
const RTC_INIT = (() => {
    const b = new Uint8Array(20);
    b.set([0x30, 0x45, 0x10, 0x03, 0x15, 0x07, 0x26]);
    b[0x11] = 0x1B; b[0x12] = 0x80;
    return b;
})();
const DEV = {
    rtc: { rtc: { i2c: 'I2C1', addr: 0x68, init: RTC_INIT } },
    oled: { oled: { i2c: 'I2C1', addr: 0x3C } },
    tft: { tft: { spi: 'SPI2', cs: 'PB12', dc: 'PB11' } },
    buzzer: { buzzer: { tim: 'TIM2' } },
    spiflash: { spi_flash: [{ peripheral: 'SPI3', jedec_id: 0xEF4015, size: 0x200000, cs: 'PB12', data: new Uint8Array(0x200000).fill(0xFF) }] },
    qspi: { qspi: [{ peripheral: 'QUADSPI', size: 256 }] },
    camera64: { camera: { width: 64, height: 48, pixels: Uint8Array.from({ length: 64 * 48 }, (_, i) => i & 0xFF) } },
    eeprom: { i2c_eeprom: [{ peripheral: 'I2C1', address: 0x50, data: new Uint8Array(256).fill(0xFF) }] },
    edge: {
        i2c_eeprom: [{ peripheral: 'I2C1', address: 0x50, data: new Uint8Array(256).fill(0xFF) }],
        spi_flash: [{ peripheral: 'SPI3', jedec_id: 0xEF4016, size: 0x200000, cs: null, data: new Uint8Array(0x200000).fill(0xFF) }],
    },
    fsmc: {
        fsmcDevices: [{
            bank: 0,
            handler: (events, pushData) => {
                for (let i = 0; i + 1 < events.length; i += 2) {
                    if ((events[i] >>> 0) & 0x80000000) {
                        if ((events[i + 1] & 0xFF) === 0x04) pushData([0x9341]);
                    }
                }
            },
        }],
    },
};
const DCMI_SMALL = new Uint8Array([0x11, 0x22, 0x33, 0x44]);
const DCMI_BIG = Uint8Array.from({ length: 32 }, (_, i) => i + 1);
// audio_test WAV: 64 PCM16 samples (i*300+7), same shape as test_audio.mjs.
const makeWav = () => {
    const N = 64, w = new Uint8Array(44 + N * 2), dv = new DataView(w.buffer);
    const ws = (o, s) => { for (let i = 0; i < s.length; i++) w[o + i] = s.charCodeAt(i); };
    ws(0, 'RIFF'); dv.setUint32(4, 36 + N * 2, true); ws(8, 'WAVE');
    ws(12, 'fmt '); dv.setUint32(16, 16, true); dv.setUint16(20, 1, true);
    dv.setUint16(22, 1, true); dv.setUint32(24, 44100, true); dv.setUint32(28, 88200, true);
    dv.setUint16(32, 2, true); dv.setUint16(34, 16, true); ws(36, 'data'); dv.setUint32(40, N * 2, true);
    for (let i = 0; i < N; i++) dv.setInt16(44 + i * 2, (i * 300 + 7) & 0xFFFF, true);
    return w;
};

// [demo, boards, bin, markers, anti, opts, script, iters, step]
const FAM = ['f401', 'f411', 'f429'];
const E = [];
const famBin = (d, f) => `${d}/${d}_${f}.bin`;
const inoBin = (d, k) => `${d}/build-${k}/${d}.ino.bin`;
const bare = (demo, boards, m, anti = null, opts = {}, script = null, iters = 600, step = 100000) => {
    for (const b of boards) E.push([demo, b, famBin(demo, b), m, anti, opts, script, iters, step]);
};
const ino = (demo, keys, m, anti = null, opts = {}, script = null, iters = 1200, step = 200000) => {
    for (const k of keys) E.push([demo, k, inoBin(demo, k), m, anti, opts, script, iters, step]);
};
const IRQ = { enable_irqs: true };
const ALL3 = FAM;
bare('adc_demo', ALL3, ['=== ADC Demo: done ===']);
bare('pwm_demo', ALL3, ['=== PWM Demo: done ===']);
bare('oled_test', ALL3, ['OLED draw done'], null, { ext_devices: DEV.oled }, null, 2000, 100000);
bare('tft_test', ALL3, ['TFT fill done'], null, { ext_devices: DEV.tft });
bare('buzzer_test', ALL3, ['Buzzer done'], null, { ext_devices: DEV.buzzer }, null, 2000, 100000);
bare('audio_play_test', ALL3, ['I2S1 TX sine 256 samples']);
bare('watchdog_demo', ALL3, ['IWDG reset detected'], null, {}, null, 1500, 200000);
bare('wwdg_demo', ALL3, ['WWDG reset detected'], null, {}, null, 1500, 200000);
bare('wwdg_window_demo', ALL3, ['WWDG reset detected'], null, {}, null, 1500, 200000);
bare('tim_capture_demo', ALL3, ['cap=', 'done'], null, {}, 'timcap');
bare('spi_flash_test', ALL3, ['SPI FLASH TEST DONE'], ['FAIL '], { ext_devices: DEV.spiflash });
bare('flash_test', ALL3, ['FLASH TEST DONE']);
bare('exti_test', ALL3, ['EXTI TEST DONE'], null, IRQ, 'exti');
bare('comprehensive_test', ['f429'], ['=== DONE ===', 'FAIL: 00000000'], ['FAILED', 'FAIL '], IRQ);
bare('comprehensive_test', ['f401', 'f411'], ['=== DONE ==='], ['FAILED', 'FAIL '], { ...IRQ, expectFail: 'absent-CRYP/HASH/CAN/DAC/SAI' });
bare('crypto_deep_test', ['f401', 'f411'], ['=== DONE ==='], ['FAIL '], { expectFail: 'no-CRYP-silicon' });
bare('i2s_sai_test', ['f401', 'f411'], ['DONE', 'FAIL: 00000000'], null, { expectFail: 'no-SAI-silicon' });
ino('timer_test', ['bp_f401cc', 'bp_f411ce', 'nucleo_f401re', 'nucleo_f411re'], ['Timer start'], null, { expectFail: 'no-UART4-silicon' });
ino('echo_test', ['bp_f401cc', 'bp_f411ce', 'nucleo_f401re', 'nucleo_f411re'], ['Echo ready'], null, { expectFail: 'no-UART4-silicon' });
bare('crypto_deep_test', ['f429'], ['=== DONE ===', 'PASS DT8 w3'], ['FAIL ']);
bare('rx_crypto_test', ALL3, ['PASS: INT CRC matches polling'], ['FAIL: CRC mismatch'], IRQ, 'rxhost');
bare('rx_interrupt_test', ALL3, ['CRC=EFE8B569'], null, IRQ, 'rxhost');
bare('test_firmware', ALL3, ['DONE'], ['FAIL']);
bare('i2s_sai_test', ['f429'], ['DONE', 'FAIL: 00000000'], ['FAIL ']);
bare('spi_tft_test', ALL3, ['DONE', 'FAIL: 00000000'], ['FAIL '], { ext_devices: DEV.spiflash });
bare('deep_sleep_demo', ALL3, ['WOKE FROM STOP'], null, { lowpower: true });
bare('rtc_test', ALL3, ['RTC test done'], ['FAIL'], { ext_devices: DEV.rtc });
bare('audio_test', ALL3, ['=== Audio Test: done'], null, {}, 'wav');
bare('fpu_test', ALL3, ['FPU all PASS'], ['FAIL']);
bare('fpu_irq_test', ALL3, ['FPU IRQ all PASS'], ['FAIL'], IRQ);
bare('mpu_test', ALL3, ['MPU all PASS', 'MPU done'], ['FAIL'], IRQ);
bare('freertos_test', ['f411', 'f429'], ['TIM TEST PASS'], ['FAIL'], { ...IRQ, freertos: true }, null, 3000, 100000);
bare('can_demo', ['f429'], ['=== CAN Demo: done ===']);
bare('can_test', ['f429'], ['CAN loopback OK'], ['CAN Test: FAIL']);
bare('can_host_rx', ['f429'], ['data=HELLO!!!'], null, {}, 'canhost');
bare('dac_demo', ['f429'], ['=== DAC Demo: done ===']);
bare('fsmc_test', ['f429'], ['=== FSMC Test: done ==='], ['FAIL'], { ext_devices: DEV.fsmc });
bare('dcmi_test', ['f429'], ['=== DCMI Test: done ==='], ['FAIL'], {}, 'dcmi');
bare('qspi_test', ['f429'], ['QSPI Test done'], ['QSPI FAIL'], { ext_devices: DEV.qspi });
bare('ltdc_test', ['f429'], ['LTDC pixels OK']);
E.push(['dma2d_test', 'f429', 'dma2d_test/dma2d_test.bin', ['=== DMA2D Test: done ==='], ['TIMEOUT', 'FAIL '], IRQ, null, 600, 100000]);
// F429 Ethernet (netsim backend; same ARP/DHCP/TCP/HTTP coverage as the
// gateway runs). eth_http runs polling (no IRQs); the rest use irq_eth.
E.push(['eth_http_f429', 'f429', 'eth_http/eth_http_f429.bin', ['TCP connected'], ['TCP fail'], {}, 'netsim', 600, 100000]);
const IRQETH = { enable_irqs: true, irq_eth: true };
E.push(['eth_dhcp_f429', 'f429', 'eth_dhcp/eth_dhcp_f429.bin', ['=== DHCP SUCCESS ==='], null, IRQETH, 'netsim', 600, 100000]);
E.push(['eth_test_f429', 'f429', 'eth_test/eth_test_f429.bin', ['ETH Test: done', 'ICMP reply OK', 'ICMP RX reply sent', 'DNS IP=093.184.216.034', 'UDP echo OK'], ['TIMEOUT!'], IRQETH, 'netsim', 900, 200000]);
E.push(['eth_feat_test_f429', 'f429', 'eth_feat_test/eth_feat_test_f429.bin', ['PHY link OK', 'PHY AN restart OK', 'PHY force OK', 'CSUM TX insert OK', 'CSUM RX IPHCE OK', 'CSUM RX PCE OK', 'MCAST OK', 'VLAN OK', 'PTP target OK', 'PTP drift OK', 'PTP TX snap OK', 'PTP RX snap OK', 'WOL OK', 'WOL filter OK', 'WIRE RATE OK', 'PPS window done', 'FEAT Test: done'], ['FAIL', 'TIMEOUT'], IRQETH, 'netsim', 8000, 5000,
    // PPS scope probe: 200k inst at 32768 Hz (edge per ~5041 inst) ~= 39.
    (b) => { const n = b.eth_pps_count(); return (n >= 20 && n <= 60) ? null : ('pps_count=' + n); }]);
E.push(['lwip_demo_f429', 'f429', 'lwip_demo/lwip_demo_f429.bin', ['LWIP init OK', 'LWIP DNS 093.184.216.034', 'LWIP TCP echo OK', 'LWIP UDP echo OK', 'LWIP DEMO DONE'], ['FAIL'], IRQETH, 'netsim', 1200, 200000]);
const IRQETH_LAYOUT = { rxDesc: 0x20000050, rxBuf: 0x2000005c, rxStride: 1536, rxDescs: 1 };
E.push(['eth_irq_test_f429', 'f429', 'eth_irq_test/eth_irq_test_f429.bin', ['ETH IRQ Test: done'], ['TIMEOUT'], { ...IRQETH, eth: IRQETH_LAYOUT }, 'netsim', 600, 100000]);
ino('blink_serial', ['disco_f429zi'], ['Hello from UART4!']);
const UART4KEYS = ['disco_f407vg', 'disco_f429zi', 'black_f407ve', 'black_f407ze'];
const HASHKEYS = ['disco_f407vg', 'disco_f429zi', 'black_f407ve', 'black_f407ze'];
ino('timer_test', UART4KEYS, ['Timer start']);
ino('echo_test', UART4KEYS, ['Echo ready', 'hello'], null, { ...IRQ, uart_addr: 0x40004C00 }, 'echo');
ino('hal_test', AKEYS, ['HAL INIT OK', 'SYSTICK OK']);
ino('edge_test', AKEYS, ['=== DONE ===', 'FAIL: 00000000'], null, { ...IRQ, ext_devices: DEV.edge });
ino('periph_test', AKEYS, ['=== DONE ===', 'FAIL: 0'], null, { ...IRQ, ext_devices: DEV.eeprom });
ino('crypto_test', HASHKEYS, ['=== DONE ===', 'FAIL: 00000000']);
ino('arduino_test', AKEYS, ['ARDUINO OK']);

const filters = process.argv.slice(2);
let pass = 0, fail = 0;
const failed = [];
const runOne = async (demo, boardKey, bin, markers, anti, opts, script, iters, step, post) => {
    const label = `${demo}/${boardKey}`;
    if (filters.length && !filters.some((f) => label.includes(f))) return 'skip';
    let fw;
    try { fw = new Uint8Array(readFileSync(new URL('../' + bin, import.meta.url))); }
    catch { console.log(`[${label}] MISSING ${bin}`); fail++; failed.push(label + ' missing'); return 'fail'; }
    const B = BOARDS[boardKey] || ABOARDS[boardKey];
    const netsim = script === 'netsim' ? createNetSim({}) : null;
    const emu = await createEmulator({
        firmware: fw, bindings, svdXml: svdFor(B.svd), wasmInit: wasmBytes,
        flash_size: B.flash, ram_size: B.ram, ...opts,
        ...(netsim ? { onTx: (frame) => { for (const r of netsim.onTx(frame)) emu.injectFrame(r); } } : {}),
    });
    if (script === 'wav') bindings.audio_load_wav(makeWav());
    if (script === 'dcmi') bindings.dcmi_feed_frame(2, 2, DCMI_SMALL);
    let uart = '', fed2 = false, fed3 = false, raised = 0, raising = false, echoed = false;
    let ok = false;
    for (let i = 0; i < iters; i++) {
        emu.step(step);
        uart += emu.drainUart();
        if (script === 'exti') {
            if (!raising && uart.includes('waiting for PA0 rising edge')) {
                raising = true; bindings.gpio_set_input(0, 0, true); raised++;
            } else if (raising && raised === 1 && uart.includes('waiting for PA0 2nd edge')) {
                bindings.gpio_set_input(0, 0, false); emu.step(1000);
                bindings.gpio_set_input(0, 0, true); raised++;
            }
        } else if (script === 'timcap') {
            if (i % 2 === 0) emu.timInjectCapture('TIM3', 0);
        } else if (script === 'canhost') {
            if (uart.includes('CAN RX ready')) emu.canInject(0x123, 8, new Uint8Array([72, 69, 76, 76, 79, 33, 33, 33]));
        } else if (script === 'echo') {
            if (!echoed && uart.includes('Echo ready')) { echoed = true; emu.sendUart('hello'); }
        } else if (script === 'rxhost') {
            if (!echoed && (uart.includes('Sending') || uart.includes('RX-INT-TEST'))) { echoed = true; emu.sendUart('Hello\n'); }
        } else if (script === 'dcmi') {
            if (uart.includes('PHASE2') && !fed2) { fed2 = true; bindings.dcmi_feed_frame(8, 4, DCMI_BIG); }
            if (uart.includes('DCMI ovr OK') && !fed3) { fed3 = true; bindings.dcmi_feed_frame(8, 4, DCMI_BIG); }
        }
        const fault = emu.faultInfo();
        if (fault) { uart += `\n[FAULT ${JSON.stringify(fault)}]`; break; }
        if (markers.every((mk) => uart.includes(mk))) { ok = true; break; }
    }
    if (ok && anti && anti.some((a) => uart.includes(a))) ok = false;
    if (ok && script === 'exti' && raised !== 2) ok = false;
    // Optional post hook: observe model-side state the guest cannot see
    // (e.g. a pin counter), like a scope probe. Runs before close().
    // Returns an error string, or null when satisfied.
    if (ok && typeof post === 'function') {
        const err = post(bindings, uart);
        if (err) { ok = false; uart += '\n[POST] ' + err; }
    }
    if (opts.expectFail) {
        if (!ok) { pass++; return 'pass'; }
        fail++; failed.push(label + ' UNEXPECTEDLY-PASSED');
        console.log(`[${label}] UNEXPECTEDLY PASSED (expected-fail, update compat!)`);
        emu.close();
        return 'fail';
    }
    emu.close();
    if (ok) { pass++; }
    else {
        fail++; failed.push(label);
        console.log(`[${label}] FAIL`);
        console.log('--- tail ---\n' + uart.replace(/\r/g, '').split('\n').filter(Boolean).slice(-6).join('\n'));
    }
    return ok ? 'pass' : 'fail';
};

for (const [demo, boardKey, bin, markers, anti, opts, script, iters, step, post] of E) {
    await runOne(demo, boardKey, bin, markers, anti, opts, script, iters, step);
}
console.log(`\nMATRIX pass=${pass} fail=${fail}`);
if (failed.length) console.log('failed: ' + failed.join(' '));
process.exit(fail ? 1 : 0);
