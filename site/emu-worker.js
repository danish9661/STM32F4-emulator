// Console emulation worker: owns the emulator, netsim peer, scripted USB
// host, and the per-burst state gathering. site/app.js is the UI shell —
// DOM, canvas, WebSocket gateway socket, file parsing, and localStorage
// stay on the page; the worker never touches them.
//
// Why: stepping the guest is a long synchronous WASM call. On the main
// thread every burst blocked input, layout and paint (the Step-1 burst
// fix raised live speed 6 -> 36 MIPS but put ~15 ms of emulation on the
// main thread per frame). The worker frees the main thread at no
// throughput cost, mirroring site/doom-worker.js:
//
// - Burst cadence is driven by the PAGE's rAF: app.js posts {t:'tick'}
//   every animation frame; the worker runs one burst per tick. Its rAF
//   callback only posts a message. Receiving it is an event-loop turn,
//   which is the TCI gap the core needs (AGENTS.md §7/§16: gap-free
//   stepping wedges Chrome's WASM interpreter — never batch bursts
//   back-to-back, never "optimize" the yield away).
// - Stale-tick fallback: when rAF dies (hidden tab) the worker drives
//   itself on a clamped setTimeout (bigger wall budget — the clamp is a
//   pure duty-cycle tax, no correctness impact).
// - No SharedArrayBuffer (Pages cannot set COOP/COEP; separate WASM
//   linear memories anyway). All transfer is postMessage, zero-copy
//   where marked.
//
// Bump the ?v= on the Worker() URL in app.js whenever this file changes.
//
// Message protocol (structured clone; `transfer` where noted):
//   page -> worker:
//     boot {cfg} ......... create emulator + netsim/usbhost + seeds; see
//                          BOOT_CFG below. Transfer: fw/extraMem/data bufs.
//     tick ............... run one burst (page rAF driven)
//     uartRx {bytes} ..... guest RX input (string or byte array)
//     uartRxTo {addr,bytes} explicit-port variant (serial target dropdown)
//     can {id,dlc,data} .. host CAN injection
//     timCap {name,ch} ... host TIM capture edge
//     gpioIn {bank,pin,high} clickable GPIO inputs (model-level)
//     watch {addrs} ...... memory-watch address list (full replace)
//     poke {addr,value} .. memory-watch poke (write32)
//     reset {mode} ....... mode 0=resetCpu 1=setNrst pulse (Reset button)
//     nrst {asserted} .... hold/release NRST
//     gwrx {frame} ....... gateway RX frame (page relays its WebSocket)
//     measure {id,steps,size} timed burst for the MIPS probe (async reply)
//     stop ............... pause stepping (page Run/Stop); start on tick
//   worker -> page:
//     booted {uart} ...... emulator ready (boot banner chunk included)
//     burst {state} ...... per-burst state; see BURST_STATE below. In
//                          gateway mode the page also relays burst.tx
//                          over its WebSocket (worker has no socket).
//     status {text,cls} .. status line updates (same text as page setStatus)
//     measureResult {id,mips,runs} reply to measure
//     fault {info} ....... guest fault (faultInfo, for the fault panel)
import * as bindings from './vendor/stm32_periph_wasm.js?v=48';
import { createEmulator } from './emulator.js?v=3';
import { createNetSim } from './netsim.js';
import { createUsbHost } from './usbhost.js';

// Burst sizing mirrors the Step-1 page loop (6 x 100k, fine-step firmware
// 6 x 20k; bridge mode never reaches the worker). Wall budget caps slow
// hosts: the worker thread being busy does NOT jank the page, but message
// round trips (UI state, pokes) queue behind the burst — 20 ms bounds
// interactive latency like doom-worker's 28 ms.
const BURST_STEPS = 6;
const BURST_FINE_STEPS = 6;
const WALL_BUDGET_MS = 20;
const TICK_STALE_MS = 200;
const YIELD_MS = 4;

// FSMC RDDID sink (ILI9341 ID 0x9341) for fsmc_test + family builds.
// Duplicated from app.js: ext_devices crosses postMessage, so the page's
// function-valued handler cannot travel. Keep byte-identical with the
// app.js copy.
const fsmcRddidSink = (events, pushData) => {
    for (let i = 0; i + 1 < events.length; i += 2) {
        const hdr = events[i] >>> 0;
        if (hdr & 0x80000000) {
            if ((events[i + 1] & 0xFF) === 0x04) pushData([0x9341]);
        }
    }
};

let emu = null, netsim = null, usbhost = null;
let cfg = null;             // last boot cfg (image opts + names)
let uartBuf = '';           // worker-side terminal buffer (usbhost + feats)
let uartLen = 0;
let dcmiFed = null;
let featHooks = null;
let watchAddrs = [];
let paused = false;
let booted = false;
let lastTickAt = 0, timer = null;
let bootedUart = '';

const post = (msg, transfer) => self.postMessage(msg, transfer || []);
const status = (text, cls) => post({ t: 'status', text, cls });

function pushUart(chunk) {
    if (!chunk) return '';
    uartBuf += chunk; uartLen += chunk.length;
    if (uartLen > 200000) { uartBuf = uartBuf.slice(-200000); uartLen = 200000; }
    return chunk;
}

function scheduleLoop(delay) {
    if (timer === null) timer = setTimeout(selfTick, delay === undefined ? YIELD_MS : delay);
}
function selfTick() {
    timer = null;
    if (!booted || paused) return;
    if (performance.now() - lastTickAt > TICK_STALE_MS) {
        burst();
        scheduleLoop(YIELD_MS);
    } else {
        scheduleLoop(TICK_STALE_MS);
    }
}

// ── boot ──
// BOOT_CFG (all cloneable; big buffers transferred):
// { fw, svdXml, svdFile, flash_size, ram_size, extraMem[{addr,data}],
//   uartAddr, name, enable_irqs, irq_eth, freertos, lowpower, eth,
//   ext_devices (minus function handlers; fsmcDevices rebuilt here),
//   gateway (bool), audioWav, dcmiBig, camPrefeed {w,h,pixels} | null,
//   useNetsim (bool: false in gateway mode) }
async function boot(c) {
    cfg = c;
    uartBuf = ''; uartLen = 0; bootedUart = '';
    dcmiFed = { big2: false, big3: false };
    watchAddrs = [];
    paused = false;
    try { if (emu) emu.close(); } catch {}
    emu = null; netsim = null; usbhost = null; featHooks = null;
    const ext = c.ext_devices || {};
    if (ext.fsmc) ext.fsmcDevices = [{ bank: 0, handler: fsmcRddidSink }];
    if (Array.isArray(c.probes)) watchAddrs = c.probes.map((a) => a >>> 0);
    emu = await createEmulator({
        firmware: c.fw,
        bindings,
        svdXml: c.svdXml,
        svdFile: c.svdFile,
        flash_size: c.flash_size,
        ram_size: c.ram_size,
        wasmUrl: 'vendor/stm32_periph_wasm_bg.wasm?v=48',
        extra_mem: c.extraMem || [],
        uart_addr: c.uartAddr,
        enable_irqs: !!c.enable_irqs,
        irq_eth: !!c.irq_eth,
        freertos: !!c.freertos,
        lowpower: !!c.lowpower,
        eth: c.eth,
        ext_devices: ext,
        onTx: (pkt) => {
            txFrames.push(pkt);
            if (netsim) {
                try {
                    for (const reply of netsim.onTx(pkt)) emu.injectFrame(reply);
                } catch {}
            }
        },
    });
    if (!c.gateway && c.useNetsim !== false) {
        try { netsim = createNetSim(); } catch { netsim = null; }
    }
    if (c.usbhost) {
        try { usbhost = createUsbHost(bindings); } catch { usbhost = null; }
    }
    if (c.featHooks && typeof bindings.eth_arm_collision === 'function') {
        featHooks = { collideDone: 0, linkDown: false, linkUp: false };
    }
    // Boot seeds (mirror app.js boot order): audio WAV, DCMI phase-1,
    // camera pre-feed for the polled-capture demos.
    try { if (c.audioWav && bindings.audio_load_wav) bindings.audio_load_wav(c.audioWav); } catch {}
    try { if (c.dcmiPhase1 && bindings.dcmi_feed_frame) bindings.dcmi_feed_frame(2, 2, new Uint8Array([0x11, 0x22, 0x33, 0x44])); } catch {}
    try {
        if (c.camPrefeed && emu.camera) {
            emu.camera.feed(c.camPrefeed.w, c.camPrefeed.h, c.camPrefeed.pixels);
        }
    } catch {}
    booted = true;
    lastTickAt = performance.now();
    bootedUart = pushUart(emu.drainUart());
    scheduleLoop(YIELD_MS);
    post({ t: 'booted', uart: bootedUart });
}

let txFrames = [];

// Scripted DCMI camera (dcmi_test phases 2-3), same markers as app.js.
function driveDcmi() {
    if (!bindings.dcmi_feed_frame || !cfg.dcmiPhases) return;
    if (uartBuf.includes('PHASE2') && !dcmiFed.big2) {
        dcmiFed.big2 = true;
        try { bindings.dcmi_feed_frame(8, 4, cfg.dcmiBig); } catch {}
    }
    if (uartBuf.includes('DCMI ovr OK') && !dcmiFed.big3) {
        dcmiFed.big3 = true;
        try { bindings.dcmi_feed_frame(8, 4, cfg.dcmiBig); } catch {}
    }
}

function driveFeatHooks() {
    if (!featHooks) return;
    const arms = uartBuf.split('COLLIDE ARM').length - 1;
    while (featHooks.collideDone < arms) {
        featHooks.collideDone++;
        try { bindings.eth_arm_collision(); } catch {}
    }
    if (!featHooks.linkDown && uartBuf.includes('LINK DOWN ARM')) {
        featHooks.linkDown = true;
        try { bindings.eth_set_link(false); } catch {}
    }
    if (!featHooks.linkUp && uartBuf.includes('LINK UP ARM')) {
        featHooks.linkUp = true;
        try { bindings.eth_set_link(true); } catch {}
    }
}

// Per-burst state for the page (BURST_STATE). Small scalar reads inline;
// framebuffers only on frame change (transfer = zero-copy); speaker
// samples drained every burst (transfer).
let lastOledFrame = -1, lastTftFrame = '', lastLtdcFrame = -1;
function gatherState() {
    const st = { inst: 0, pc: 0, sp: 0, xpsr: 0, stopped: false, gpio: null, watch: {}, regs: null };
    try {
        const r = emu.getRegisters();
        st.pc = r.PC >>> 0; st.sp = r.SP >>> 0; st.xpsr = r.XPSR >>> 0;
        st.regs = [r.R0, r.R1, r.R2, r.R3, r.R4, r.R5, r.R6, r.R7, r.R8, r.R9, r.R10, r.R11, r.R12] .map((v) => v >>> 0);
    } catch {}
    st.gpio = [];
    for (let b = 0; b < 5; b++) {
        try {
            const base = 0x40020000 + b * 0x400;
            st.gpio.push([emu.read32(base) >>> 0, emu.read32(base + 0x14) >>> 0, emu.read32(base + 0x10) >>> 0]);
        } catch { st.gpio.push([0, 0, 0]); }
    }
    for (const a of watchAddrs) {
        try { st.watch[a] = emu.read32(a) >>> 0; } catch { st.watch[a] = 0; }
    }
    return st;
}

function burst() {
    if (!emu || !booted || paused) return;
    const t0 = performance.now();
    const fine = !!cfg.fineSteps;
    const maxSteps = fine ? BURST_FINE_STEPS : BURST_STEPS;
    let res = null, steps = 0;
    txFrames = [];
    for (let s = 0; s < maxSteps; s++) {
        try {
            res = emu.step(fine ? 20000 : 100000);
        } catch (e) {
            status('error: ' + e.message, 'err');
            paused = true;
            post({ t: 'burst', state: null, error: String(e && e.message || e) });
            return;
        }
        steps++;
        if (res.stopped) break;
        if (performance.now() - t0 > WALL_BUDGET_MS) break;
    }
    const chunk = pushUart(emu.drainUart());
    try { if (usbhost) usbhost.frame(uartBuf); } catch {}
    try { driveDcmi(); } catch {}
    try { driveFeatHooks(); } catch {}
    const st = gatherState();
    st.inst = res ? res.instCount : 0;    st.pc = res ? res.pc >>> 0 : st.pc;
    st.stopped = res ? !!res.stopped : false;
    st.steps = steps;
    // Device snapshots (change-gated; buffers transferred).
    const transfers = [];
    try {
        if (emu.oled) {
            const f = emu.oled.frame();
            st.oledFrame = f;
            if (f !== lastOledFrame) {
                lastOledFrame = f;
                const fb = Uint8Array.from(emu.oled.fb);
                st.oledFb = fb; transfers.push(fb.buffer);
            }
        }
    } catch {}
    try {
        if (emu.tft) {
            const key = emu.tft.frame() + ':' + emu.tft.w + 'x' + emu.tft.h;
            st.tftFrame = emu.tft.frame(); st.tftW = emu.tft.w; st.tftH = emu.tft.h;
            if (key !== lastTftFrame) {
                lastTftFrame = key;
                const fb = Uint8Array.from(emu.tft.fb);
                st.tftFb = fb; transfers.push(fb.buffer);
            }
        }
    } catch {}
    try {
        const fc = (typeof bindings.ltdc_get_frame_count === 'function') ? bindings.ltdc_get_frame_count() : -1;
        st.ltdcFrame = fc;
        if (fc >= 0 && fc !== lastLtdcFrame && emu.uc && typeof emu.uc.mem_read === 'function') {
            lastLtdcFrame = fc;
            const info = [
                emu.read32(0x40016818), emu.read32(0x40016884), emu.read32(0x40016894),
                emu.read32(0x400168AC), emu.read32(0x400168B0), emu.read32(0x400168B4),
                emu.read32(0x40016888), emu.read32(0x4001688C),
            ].map((v) => v >>> 0);
            st.ltdcInfo = info;
            // Pixel rows from guest RAM (same geometry math as app.js
            // renderLtdc; rows copied here because the page cannot mem_read).
            try {
                const cfbar = info[3], pitch = (info[4] >>> 16) || 0;
                const lineBytes = (info[4] & 0xFFFF) || 0;
                const h = info[5] || 0;
                if (cfbar && pitch && lineBytes && h && h <= 1024) {
                    const px = new Uint8Array(lineBytes * h);
                    for (let y = 0; y < h; y++) {
                        px.set(new Uint8Array(emu.uc.mem_read(BigInt((cfbar + y * pitch) >>> 0), lineBytes)), y * lineBytes);
                    }
                    st.ltdcPx = px; transfers.push(px.buffer);
                }
            } catch {}
        }
    } catch {}
    try {
        if (emu.buzzer) st.buzzer = { f: emu.buzzer.freq, duty: emu.buzzer.duty, ch: emu.buzzer.change };
    } catch {}
    try {
        if (typeof bindings.dma_get_pending_count === 'function') st.dma = bindings.dma_get_pending_count() >>> 0;
    } catch {}
    try {
        if (emu.rtc && emu.rtc.time) st.rtc = { t: emu.rtc.time, temp: emu.rtc.temp, ch: emu.rtc.change };
    } catch {}
    try {
        if (typeof emu.takeSpeakerSamples === 'function') {
            const s = emu.takeSpeakerSamples();
            if (s && s.length) { const arr = Float32Array.from(s); st.spk = arr; transfers.push(arr.buffer); }
        }
    } catch {}
    try {
        if (typeof bindings.eth_pps_count === 'function') {
            st.pps = bindings.eth_pps_count() >>> 0;
            try { st.ppsLevel = !!bindings.eth_pps_level(); } catch { st.ppsLevel = null; }
        }
    } catch {}
    const tx = txFrames; txFrames = [];
    // TX frames are small (a few per burst) — plain clone, no transfer
    // (the page needs the bytes twice in gateway mode: viewer + ws.send,
    // and a transfer would neuter the second use).
    const txBytes = [];
    for (const f of tx) txBytes.push(f instanceof Uint8Array ? Uint8Array.from(f) : new Uint8Array(f));
    let fault = null;
    if (st.stopped && typeof emu.faultInfo === 'function') {
        try { fault = emu.faultInfo(); } catch {}
    }
    post({ t: 'burst', state: st, uart: chunk, tx: txBytes, fault }, transfers);
}

self.onmessage = async (e) => {
    const m = e.data;
    try {
        switch (m.t) {
            case 'boot':
                lastOledFrame = -1; lastTftFrame = ''; lastLtdcFrame = -1;
                await boot(m.cfg);
                break;
            case 'tick':
                lastTickAt = performance.now();
                burst();
                break;
            case 'stop':
                paused = !!m.paused;
                break;
            case 'uartRx': {
                const b = m.bytes;
                try {
                    if (typeof emu.sendUart === 'function') emu.sendUart(b);
                } catch {}
                break;
            }
            case 'uartRxTo':
                try {
                    if (typeof emu.sendUartTo === 'function') emu.sendUartTo(m.addr >>> 0, m.bytes);
                    else if (typeof emu.sendUart === 'function') emu.sendUart(m.bytes);
                } catch {}
                break;
            case 'can':
                try { emu.canInject(m.id, m.dlc, m.data); } catch {}
                break;
            case 'timCap':
                try { emu.timInjectCapture(m.name, m.ch); } catch {}
                break;
            case 'gpioIn':
                try { if (bindings.gpio_set_input) bindings.gpio_set_input(m.bank, m.pin, !!m.high); } catch {}
                break;
            case 'watch':
                watchAddrs = (m.addrs || []).map((a) => a >>> 0);
                break;
            case 'usbOut':
                try { if (usbhost && typeof usbhost.bulkOut === 'function') usbhost.bulkOut(m.bytes); } catch {}
                break;
            case 'poke':
                try { emu.write32(m.addr >>> 0, m.value >>> 0); } catch {}
                break;
            case 'reset':
                try {
                    if (m.mode === 1 && typeof emu.setNrst === 'function') {
                        emu.setNrst(true); emu.setNrst(false);
                    }
                    if (typeof emu.resetCpu === 'function') emu.resetCpu();
                    else emu.reset();
                    uartBuf = ''; uartLen = 0;
                    post({ t: 'burst', state: gatherState(), uart: pushUart(emu.drainUart()), tx: [], fault: null });
                } catch (err) { status('reset failed: ' + err.message, 'err'); }
                break;
            case 'nrst':
                try { if (typeof emu.setNrst === 'function') emu.setNrst(!!m.asserted); } catch {}
                break;
            case 'gwrx':
                try { emu.injectFrame(m.frame); } catch {}
                break;
            case 'measure': {
                // Timed burst for the MIPS probe (async reply — the page
                // awaits measureResult; no CDP round trip per step).
                // Warmup first: a freshly booted worker tiers up over the
                // first runs (cold spreads rise run-over-run and never
                // plateau); time only the last 5 like the Node pill does
                // after its boot loop.
                try { for (let w = 0; w < 3; w++) emu.step(m.size || 500000); } catch {}
                const runs = [];
                for (let k = 0; k < 5; k++) {
                    let i0 = 0, i1 = 0;
                    try {
                        const r0 = emu.step(0); i0 = r0.instCount;
                        const t0 = performance.now();
                        for (let i = 0; i < 10; i++) { i1 = emu.step(m.size || 500000).instCount; }
                        runs.push((i1 - i0) / (performance.now() - t0) * 1000 / 1e6);
                    } catch (err) { post({ t: 'measureResult', id: m.id, error: String(err && err.message || err) }); break; }
                }
                runs.sort((x, y) => x - y);
                post({ t: 'measureResult', id: m.id, best: runs[4], median: runs[2], runs });
                break;
            }
            default:
                break;
        }
    } catch (err) {
        status('worker: ' + (err && err.message || err), 'err');
    }
};
