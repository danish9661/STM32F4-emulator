// In-browser driver for the STM32F407 single-page console.
// Preset + custom (.bin/.hex/.elf/.map) firmware loading, Run/Stop/Reset,
// an optional WebSocket gateway (real network stack) with a netsim fallback,
// live UART terminal, GPIO/peripheral register readout, and packet viewer.
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';
import { FIRMWARES } from './firmware.js';
import { parseIntelHex, parseElf, parseMap } from './loaders.js';
import { createRemoteEmulator } from './remote-emu.js';

const $ = (id) => document.getElementById(id);
const uartEl = $('uart'), statusEl = $('statusText'), dotEl = $('dot');
const framesEl = $('frames');

const decodeB64 = (b64) => {
    const bin = atob(b64);
    const u = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) u[i] = bin.charCodeAt(i);
    return u;
};
const hex = (arr, n = 32) => {
    let s = '';
    for (let i = 0; i < Math.min(arr.length, n); i++) s += arr[i].toString(16).padStart(2, '0') + ' ';
    if (arr.length > n) s += '…';
    return s.trim();
};
const hex32 = (v) => '0x' + (v >>> 0).toString(16).padStart(8, '0');

let session = 0;
let emu = null, netsim = null, running = false;
let uartBuf = '', totalInst = 0, t0 = performance.now(), lastInst = 0, lastT = t0;
let stepsDone = 0;
let image = null;          // { flash, ram, extraMem, entry, symbols, name, uartAddr }

// Firmwares on UART4 (0x40004C00) instead of USART1 (0x40011000).
const UART4_FIRMWARES = new Set(['echo_test', 'blink_serial']);
const uartAddrFor = (name) => UART4_FIRMWARES.has(name) ? 0x40004C00 : 0x40011000;

// Per-firmware ETH RX descriptor/buffer SRAM addresses (driver injects frames
// directly into guest memory). Defaults match eth_http (emulator.js E).
const ETH_RX_MAP = {
    eth_irq_test: { rxDesc: 0x20000050, rxBuf: 0x2000005c },
};

// Interrupt-driven firmware: the emulator pumps guest IRQ handlers (USART RXNE
// etc.). OFF for ETH firmware — the driver signals completion via SRAM
// irq_flag and the guest ETH_IRQHandler would double-process DMASR/rx_desc.
const IRQ_FIRMWARES = new Set(['rx_interrupt_test', 'rx_crypto_test', 'comprehensive_test', 'eth_irq_test']);

// Interrupt-driven ETH firmware: the guest ETH_IRQHandler (run by the pump)
// reads DMASR and scans rx_desc itself, so the driver must not write the
// SRAM irq_flag/rx_frame_idx globals (irq_eth mode in emulator.js).
const IRQ_ETH_FIRMWARES = new Set(['eth_irq_test']);

// Firmwares that exercise the WFI/STOP low-power path: the emulator halts the
// core on WFI and advances the virtual RTC until an alarm/interrupt wakes it.
const LOWPOWER_FIRMWARES = new Set(['deep_sleep_demo']);

// Virtual hardware attached to the emulator per firmware: the JS device
// layer parses the peripheral traffic and renders it (OLED fb, TFT fb,
// buzzer freq, speaker samples). Matches emulator.js ext_devices.
// RTC register-file seed: BCD time 0x00-0x06 (10:45:30 dow3 15/07/26),
// temp MSB/LSB 0x11/0x12 = 27.50 C. The guest overwrites the time regs.
const RTC_INIT = (() => {
    const b = new Uint8Array(20);
    b.set([0x30, 0x45, 0x10, 0x03, 0x15, 0x07, 0x26]);
    b[0x11] = 0x1B; b[0x12] = 0x80;
    return b;
})();
const DEVICE_FIRMWARES = {
    oled_test: { oled: { i2c: 'I2C1', addr: 0x3C } },
    tft_test: { tft: { spi: 'SPI2', cs: 'PB12', dc: 'PB11' } },
    buzzer_test: { buzzer: { tim: 'TIM2' } },
    audio_play_test: { speaker: true },
    rtc_test: { rtc: { i2c: 'I2C1', addr: 0x68, init: RTC_INIT } },
    qspi_test: { qspi: [{ peripheral: 'QUADSPI', size: 256 }] },
};

// ── status + UART ──────────────────────────────────────────────────────────
const setStatus = (text, cls) => {
    statusEl.textContent = text;
    dotEl.className = 'dot ' + cls;
};

const appendUart = (chunk) => {
    if (!chunk) return;
    uartBuf += chunk;
    if (uartBuf.length > 200000) uartBuf = uartBuf.slice(-200000);
    const wasAtBottom = uartEl.scrollTop + uartEl.clientHeight >= uartEl.scrollHeight - 8;
    uartEl.textContent = uartBuf;
    if (wasAtBottom && $('chkAuto').checked) uartEl.scrollTop = uartEl.scrollHeight;
};

// ── packets ────────────────────────────────────────────────────────────────
const addFrame = (dir, pkt) => {
    const meta = describeFrame(dir, pkt);
    const div = document.createElement('div');
    div.className = 'frame ' + dir;
    div.innerHTML = `<span class="tag">${dir === 'tx' ? 'TX' : 'RX'}</span> ${pkt.length}B ${meta}<pre style="margin:2px 0 0;font-size:11px;color:var(--dim);white-space:pre-wrap;word-break:break-all;">${hex(pkt, 32)}</pre>`;
    framesEl.prepend(div);
    while (framesEl.children.length > 40) framesEl.lastChild.remove();
};

const describeFrame = (dir, pkt) => {
    if (pkt.length < 42) return '';
    const et = (pkt[12] << 8) | pkt[13];
    if (et === 0x0806) return 'ARP';
    if (et !== 0x0800) return '';
    const proto = pkt[23];
    if (proto === 17) {
        const dp = (pkt[36] << 8) | pkt[37];
        if (dp === 67) return 'DHCP';
        if (dp === 68) return 'DHCP → server';
        return 'UDP';
    }
    if (proto === 6) {
        const sp = (pkt[34] << 8) | pkt[35], dp = (pkt[36] << 8) | pkt[37];
        const fl = pkt[47];
        const flS = ((fl & 0x2 ? 'S' : '') + (fl & 0x10 ? 'A' : '') + (fl & 0x08 ? 'P' : '') + (fl & 0x01 ? 'F' : '')).padEnd(3, '·');
        const seq = ((pkt[38] << 24) | (pkt[39] << 16) | (pkt[40] << 8) | pkt[41]) >>> 0;
        const ack = ((pkt[42] << 24) | (pkt[43] << 16) | (pkt[44] << 8) | pkt[45]) >>> 0;
        return `TCP :${sp}→:${dp} fl=0x${fl.toString(16)} ${flS} seq=${seq} ack=${ack}`;
    }
    return `IP proto ${proto}`;
};

// ── gateway (real network stack, optional) ─────────────────────────────────
const gw = { ws: null, connected: false, tx: 0, rx: 0 };
const setGwStatus = (connected, text) => {
    gw.connected = connected;
    $('gwDot').className = 'dot ' + (connected ? 'run' : 'stop');
    $('gwStatus').textContent = text;
    $('btnGw').textContent = connected ? 'Disconnect' : 'Connect';
};

const gwLabel = () => `${gw.connected ? 'connected — ' + gw.tx + ' TX / ' + gw.rx + ' RX frames to the real stack' : 'offline — using the scripted network (netsim)'}`;
const refreshGwLabel = () => { if (gw.connected) $('gwStatus').textContent = gwLabel(); };

const connectGateway = () => {
    const url = $('gwUrl').value.trim() || 'ws://127.0.0.1:5099/api/network-gateway';
    if (!/^wss?:\/\//.test(url)) return setGwStatus(false, 'bad URL — expected ws:// or wss://');
    if (gw.ws) {
        try { gw.ws.close(); } catch (e) {}
        gw.ws = null;
        setGwStatus(false, gwLabel());
        return;
    }
    let ws;
    try { ws = new WebSocket(url); } catch (e) { return setGwStatus(false, 'connect failed: ' + e.message); }
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => {
        gw.ws = ws;
        setGwStatus(true, gwLabel());
        appendUart(`── gateway connected (${url}) ──\r\n`);
    };
    ws.onmessage = (ev) => {
        if (typeof ev.data === 'string') return; // control message (e.g. RESET)
        const buf = new Uint8Array(ev.data);
        gw.rx++;
        refreshGwLabel();
        addFrame('rx', buf);
        if (emu) emu.injectFrame(buf);
    };
    ws.onclose = () => {
        if (gw.ws !== ws) return;
        gw.ws = null;
        setGwStatus(false, gwLabel());
    };
    ws.onerror = () => setGwStatus(false, gwLabel().replace('connected', 'connection error'));
};

// ── firmware loading ───────────────────────────────────────────────────────
const loadFirmwareBytes = (bytes, name) => {
    image = { flash: new Uint8Array(bytes), ram: null, extraMem: [], entry: null, symbols: null, name, uartAddr: uartAddrFor(name) };
    boot();
};

const loadHex = (text, name) => {
    const img = parseIntelHex(text);
    if (!img.flash) throw new Error('no flash data found in HEX file');
    image = { flash: img.flash, ram: img.ram, extraMem: [], entry: img.entry, symbols: null, name, uartAddr: uartAddrFor(name) };
    boot();
};

const loadElf = (bytes, name) => {
    const img = parseElf(bytes);
    if (!img.flash) throw new Error('ELF has no FLASH loadable segment');
    image = { flash: img.flash, ram: img.ram, extraMem: img.extraMem, entry: img.entry, symbols: img.symbols, name, uartAddr: uartAddrFor(name) };
    renderSymbols(img.symbols);
    boot();
};

$('btnBoot').addEventListener('click', () => {
    const fw = $('fwSelect').value;
    image = { flash: decodeB64(FIRMWARES[fw].bytes), ram: null, extraMem: [], entry: null, symbols: null, name: fw, uartAddr: uartAddrFor(fw) };
    renderSymbols(null);
    boot();
});

$('fwFile').addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (!file) return;
    const ext = file.name.slice(file.name.lastIndexOf('.')).toLowerCase();
    try {
        if (ext === '.bin') {
            loadFirmwareBytes(new Uint8Array(await file.arrayBuffer()), file.name);
        } else if (ext === '.hex') {
            loadHex(await file.text(), file.name);
        } else if (ext === '.elf') {
            loadElf(new Uint8Array(await file.arrayBuffer()), file.name);
        } else if (ext === '.map') {
            const symbols = parseMap(await file.text());
            if (!symbols.length) throw new Error('no symbols found in map file');
            if (image) image.symbols = symbols;
            renderSymbols(symbols);
            appendUart(`── loaded ${symbols.length} symbols from ${file.name} ──\r\n`);
        } else {
            throw new Error('unsupported file type: ' + ext);
        }
    } catch (err) {
        setStatus('load error: ' + err.message, 'err');
    }
    e.target.value = '';
});

// ── boot / run / stop / reset ──────────────────────────────────────────────
const raf = () => new Promise((r) => requestAnimationFrame(r));

const setBusy = (busy) => {
    for (const id of ['btnRun', 'btnStop', 'btnReset']) $(id).disabled = !busy;
};

// Remote bridge mode: when ?bridge=ws://… is set, the emulator runs in Node
// and the browser is a thin UI over WebSocket.  bridgeUrl is null in local
// mode (the default).
const params = new URLSearchParams(location.search);
const bridgeUrl = params.get('bridge');

const boot = async () => {
    const id = ++session;
    running = false;
    setBusy(false);
    $('btnRun').textContent = 'Run';
    setStatus('booting…', 'stop');
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }
    oledCacheKey = ''; tftCacheKey = ''; buzzerCacheKey = ''; rtcCacheKey = '';
    if (audioCtx) { try { audioCtx.close(); } catch (e) {} audioCtx = null; audioQueued = 0; }

    const fw = image.flash;
    uartEl.textContent = uartBuf = '';
    framesEl.textContent = '';
    totalInst = 0; stepsDone = 0;
    t0 = lastT = performance.now(); lastInst = 0;
    $('stFw').textContent = image.name;

    if (bridgeUrl) {
        // ── remote mode: Node runs the emulator, browser is a thin client ──
        netsim = null;
        gw.tx = 0; gw.rx = 0;
        try {
            emu = await createRemoteEmulator(bridgeUrl, {
                onTx: (pkt) => {
                    addFrame('tx', pkt);
                    if (gw.connected && gw.ws) {
                        gw.tx++;
                        refreshGwLabel();
                        try { gw.ws.send(pkt); } catch (e) {}
                    } else if (netsim) {
                        for (const reply of netsim.onTx(pkt)) {
                            addFrame('rx', reply);
                            emu.injectFrame(reply);
                        }
                    }
                },
                onStopped: () => {
                    running = false;
                    $('btnRun').textContent = 'Run';
                    setStatus('stopped', 'stop');
                },
            });
            if (id !== session) { emu.close(); return; }
            // Send the firmware image to the Node bridge.
            await emu.loadImage(fw);
        } catch (e) {
            setStatus('bridge error: ' + e.message, 'err');
            return;
        }
    } else {
        // ── local mode: WASM runs in the browser (default) ──
        const svdXml = await fetch('vendor/stm32f407.svd').then((r) => r.text());
        if (id !== session) return;

        netsim = gw.connected ? null : createNetSim();
        gw.tx = 0; gw.rx = 0;
        if (gw.connected) setGwStatus(true, gwLabel());
        emu = await createEmulator({
            firmware: fw,
            bindings,
            unicorn: MUnicorn,
            svdXml,
            extra_mem: image.extraMem,
            uart_addr: image.uartAddr,
            enable_irqs: IRQ_FIRMWARES.has(image.name),
            irq_eth: IRQ_ETH_FIRMWARES.has(image.name),
            lowpower: LOWPOWER_FIRMWARES.has(image.name),
            eth: ETH_RX_MAP[image.name],
            ext_devices: DEVICE_FIRMWARES[image.name],
            onTx: (pkt) => {
                addFrame('tx', pkt);
                if (gw.connected && gw.ws) {
                    gw.tx++;
                    refreshGwLabel();
                    try { gw.ws.send(pkt); } catch (e) {}
                } else if (netsim) {
                    for (const reply of netsim.onTx(pkt)) {
                        addFrame('rx', reply);
                        emu.injectFrame(reply);
                    }
                }
            },
        });
        if (id !== session) { emu.close(); return; }
    }

    appendUart(`── booted ${image.name} ${bridgeUrl ? '(bridge ' + bridgeUrl + ')' : gw.connected ? '(gateway)' : '(netsim)'} ──\r\n`);
    window.__emu = emu;          // debug handle (CDP smoke tests)
    window.__bindings = bindings;
    running = true;
    setBusy(true);
    $('btnRun').textContent = 'Stop';
    setStatus('running', 'run');
    loop(id);
};

const loop = async (id) => {
    while (session === id) {
        if (!running) { await raf(); continue; }
        try {
            const res = await emu.step();
            totalInst = res.instCount;
            stepsDone++;
        } catch (e) {
            setStatus('error: ' + e.message, 'err');
            running = false;
            $('btnRun').textContent = 'Run';
            return;
        }
        appendUart(emu.drainUart());
        refreshStats();
        renderLtdc();
        renderDevices();
        await raf();
    }
};

$('btnRun').addEventListener('click', () => {
    if (!emu) return;
    if (running) {
        running = false;
        $('btnRun').textContent = 'Run';
        setStatus('stopped', 'stop');
    } else {
        running = true;
        $('btnRun').textContent = 'Stop';
        setStatus('running', 'run');
        loop(session);
    }
});
$('btnStop').addEventListener('click', () => {
    if (!emu) return;
    running = false;
    $('btnRun').textContent = 'Run';
    setStatus('stopped', 'stop');
});
$('btnReset').addEventListener('click', () => {
    if (gw.connected && gw.ws) {
        try { gw.ws.send('RESET'); } catch (e) {}
    }
    if (image) boot();
});
$('btnClear').addEventListener('click', () => { uartEl.textContent = uartBuf = ''; });
$('btnGw').addEventListener('click', connectGateway);

const sendRx = (term) => {
    const input = $('rxInput');
    const text = input.value;
    const bytes = new Uint8Array(text.length + (term ? 1 : 0));
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i) & 0xFF;
    if (term) bytes[text.length] = term;
    if (!bytes.length || !emu) return;
    appendUart('> ' + text + (term === 0x0D ? '\r' : term === 0x0A ? '\n' : '') + '\r\n');
    emu.sendUart(bytes);
    input.value = '';
};
$('btnSend').addEventListener('click', () => sendRx(0x0D));
$('rxInput').addEventListener('keydown', (e) => {
    if (e.key !== 'Enter') return;
    e.preventDefault();
    sendRx(e.shiftKey ? 0x0A : 0x0D);
});

// ── CAN injection (host-injected frames via emu.canInject) ─────────────────
const setCanStatus = (msg, err) => {
    const el = $('canStatus');
    if (el) { el.textContent = msg; el.style.color = err ? 'var(--red)' : 'var(--dim)'; }
};
const injectCan = () => {
    if (!emu) { setCanStatus('boot a CAN firmware first', true); return; }
    const idStr = $('canId').value.trim().replace(/^0x/i, '');
    const id = parseInt(idStr, 16);
    if (!Number.isFinite(id) || id < 0 || id > 0x7FF) {
        setCanStatus('CAN ID must be an 11-bit hex value (0..7FF)', true); return;
    }
    const toks = $('canData').value.trim().split(/\s+/).filter(Boolean);
    if (toks.length > 8) { setCanStatus('at most 8 data bytes', true); return; }
    const data = new Uint8Array(toks.length);
    for (let i = 0; i < toks.length; i++) {
        const b = parseInt(toks[i], 16);
        if (!Number.isFinite(b) || b < 0 || b > 0xFF) {
            setCanStatus('data bytes must be hex 00..FF', true); return;
        }
        data[i] = b;
    }
    emu.canInject(id, toks.length, data);
    setCanStatus(`injected id=0x${id.toString(16)} dlc=${toks.length}`);
};
$('btnCanInject').addEventListener('click', injectCan);
$('btnCanDemo').addEventListener('click', () => {
    $('canId').value = '123';
    $('canData').value = '48 45 4C 4C 4F 21 21 21';
    injectCan();
});

// ── memory watch ────────────────────────────────────────────────────────────
const WATCH = [];
const watchListEl = $('watchList');
watchListEl.innerHTML = '<div class="empty">No watches yet — add an address above.</div>';

const parseHex = (s) => {
    const t = String(s).trim().replace(/^0x/i, '').replace(/[^0-9a-fA-F]/g, '');
    if (!t) return null;
    const v = parseInt(t, 16);
    return Number.isFinite(v) ? v >>> 0 : null;
};

const refreshWatch = async () => {
    if (!emu) return;
    for (const w of WATCH) {
        try {
            const v = await emu.read32(w.addr);
            w.valEl.textContent = hex32(v);
        } catch { w.valEl.textContent = '—'; }
    }
};

const addWatch = () => {
    const addr = parseHex($('watchAddr').value);
    if (addr === null) { setStatus('watch: invalid address', 'err'); return; }
    const label = $('watchLabel').value.trim() || ('0x' + addr.toString(16));
    const w = { addr, label, valEl: null };
    const row = document.createElement('div');
    row.className = 'row';
    const lbl = document.createElement('span');
    lbl.className = 'lbl';
    lbl.innerHTML = `<b>${label}</b> <span style="color:var(--dim)">(${hex32(addr)})</span>`;
    const val = document.createElement('span');
    val.className = 'val';
    val.textContent = '—';
    w.valEl = val;
    const poke = document.createElement('input');
    poke.type = 'text'; poke.placeholder = 'poke'; poke.spellcheck = false;
    const setBtn = document.createElement('button');
    setBtn.textContent = 'Set';
    setBtn.onclick = () => {
        const v = parseHex(poke.value);
        if (v === null) { setStatus('watch: invalid poke value', 'err'); return; }
        try { emu.write32(addr, v); } catch { setStatus('watch: write failed', 'err'); }
    };
    const rm = document.createElement('button');
    rm.className = 'rm'; rm.textContent = '×'; rm.title = 'remove';
    rm.onclick = () => {
        const i = WATCH.indexOf(w);
        if (i >= 0) WATCH.splice(i, 1);
        row.remove();
        if (!WATCH.length) watchListEl.innerHTML = '<div class="empty">No watches yet — add an address above.</div>';
    };
    row.append(lbl, val, poke, setBtn, rm);
    WATCH.push(w);
    if (watchListEl.querySelector('.empty')) watchListEl.innerHTML = '';
    watchListEl.appendChild(row);
    $('watchAddr').value = ''; $('watchLabel').value = '';
};

const clearWatch = () => {
    WATCH.length = 0;
    watchListEl.innerHTML = '<div class="empty">No watches yet — add an address above.</div>';
};

$('btnWatchAdd').addEventListener('click', addWatch);
$('watchAddr').addEventListener('keydown', (e) => { if (e.key === 'Enter') addWatch(); });
$('btnWatchClear').addEventListener('click', clearWatch);

$('btnSaveLog').addEventListener('click', () => {
    if (!uartBuf) return;
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([uartBuf], { type: 'text/plain' }));
    a.download = 'uart-' + (image ? image.name : 'log') + '.txt';
    a.click();
    URL.revokeObjectURL(a.href);
});
window.addEventListener('error', (e) => setStatus('error: ' + e.message, 'err'));
window.addEventListener('unhandledrejection', (e) => setStatus('boot error: ' + String(e.reason?.message || e.reason).replace(/\s+/g, ' ').slice(0, 300), 'err'));

// ── stats ──────────────────────────────────────────────────────────────────
const refreshStats = async () => {
    if (!emu) return;
    let regs = null;
    try {
        const r = emu.getRegisters();
        regs = r instanceof Promise ? await r : r;
    } catch {}
    const now = performance.now();
    const dt = (now - lastT) / 1000;
    if (dt > 0.5) {
        const mips = ((totalInst - lastInst) / dt / 1e6).toFixed(2);
        lastInst = totalInst; lastT = now;
        $('stMips').textContent = mips;
    }
    $('stInst').textContent = totalInst.toLocaleString();
    $('stSteps').textContent = stepsDone.toLocaleString();
    $('stRounds').textContent = (uartBuf.match(/=== HTTP \d+b ===/g) || []).length;
    $('stPc').textContent = regs ? hex32(regs.PC) : '—';
    $('stSp').textContent = regs ? hex32(regs.SP) : '—';
    $('stXpsr').textContent = regs ? hex32(regs.XPSR) : '—';
    await refreshGpio();
    await refreshPeriph();
    await refreshWatch();
};

// ── LTDC display sink ──────────────────────────────────────────────────────
// Renders layer-0's framebuffer (ARGB8888 / RGB565) into the aside canvas
// each rAF once the guest enables the controller + layer. Cache-keyed so we
// only repaint when the framebuffer content actually changes (the model
// doesn't signal frame boundaries through a JS-readable counter change alone).
const LTDC = 0x40016800;
const ltdcCanvas = $('ltdcCanvas'), ltdcInfo = $('ltdcInfo');
let ltdcCtx = ltdcCanvas.getContext('2d');
let ltdcCacheKey = '';
let ltdcOff = document.createElement('canvas');

const renderLtdc = () => {
    if (!emu || !ltdcCanvas.width) return;
    try {
        const gcr = emu.read32(LTDC + 0x18);
        const l1cr = emu.read32(LTDC + 0x84);
        if (!(gcr & 1) || !(l1cr & 1)) { ltdcInfo.textContent = 'scanout idle'; return; }
        const pf = emu.read32(LTDC + 0x94) & 7;
        const cfbar = emu.read32(LTDC + 0xAC) >>> 0;
        const cfblr = emu.read32(LTDC + 0xB0) >>> 0;
        const cfblnr = emu.read32(LTDC + 0xB4) & 0x7FF;
        const whpcr = emu.read32(LTDC + 0x88) >>> 0;
        const wvpcr = emu.read32(LTDC + 0x8C) >>> 0;
        const bpp = pf === 2 ? 2 : 4;
        const lineBytes = Math.min(cfblr & 0x1FFF, 640 * bpp);
        const w = Math.min((((whpcr & 0xFFF) + 1) || (lineBytes / bpp)) | 0, 640);
        const h = Math.min((((whpcr >> 16) & 0xFFF) + 1) | 0, 640);
        const lines = Math.min(cfblnr || h, 640);
        const pitch = Math.min((cfblr >>> 16) || lineBytes, w * bpp + 8);
        if (!w || !lines || cfbar < 0x20000000 || cfbar >= 0x20040000) {
            ltdcInfo.textContent = 'layer enabled, waiting for framebuffer…';
            return;
        }
        const key = pf + ':' + w + 'x' + lines + ':' + cfbar + ':' + pitch + ':' + (bindings.ltdc_get_frame_count ? bindings.ltdc_get_frame_count() : 0);
        if (key === ltdcCacheKey) return;
        ltdcCacheKey = key;
        // Read (only the used width bytes per line — the model guest RAM is
        // byte-exact, so no opacity/color-key handling beyond the format).
        const rowBytes = Math.min(w * bpp, pitch || w * bpp);
        const bytes = new Uint8Array(w * lines * bpp);
        for (let y = 0; y < lines; y++) {
            const row = emu.uc.mem_read(BigInt(cfbar + y * pitch), rowBytes);
            bytes.set(new Uint8Array(row.buffer, row.byteOffset, rowBytes), y * w * bpp);
        }
        ltdcOff.width = w; ltdcOff.height = lines;
        const octx = ltdcOff.getContext('2d');
        const img = octx.createImageData(w, lines);
        const d = img.data;
        for (let i = 0, p = 0; i < w * lines; i++) {
            if (pf === 2) {
                const v = bytes[p] | (bytes[p + 1] << 8); p += 2;
                d[i * 4] = ((v >> 11) & 0x1F) << 3;
                d[i * 4 + 1] = ((v >> 5) & 0x3F) << 2;
                d[i * 4 + 2] = (v & 0x1F) << 3;
                d[i * 4 + 3] = 255;
            } else {
                d[i * 4] = bytes[p + 2]; d[i * 4 + 1] = bytes[p + 1];
                d[i * 4 + 2] = bytes[p]; d[i * 4 + 3] = bytes[p + 3];
                p += 4;
            }
        }
        octx.putImageData(img, 0, 0);
        ltdcCtx.imageSmoothingEnabled = false;
        ltdcCtx.drawImage(ltdcOff, 0, 0, ltdcCanvas.width, ltdcCanvas.height);
        ltdcInfo.textContent = `layer0 ${w}×${lines} ${pf === 2 ? 'RGB565' : 'ARGB8888'} @ ${hex32(cfbar)}`;
    } catch (e) { /* device disabled mid-read — repaint next rAF */ }
};

// ── virtual devices (OLED / TFT / buzzer / speaker) ────────────────────────
const oledCanvas = $('oledCanvas'), oledInfo = $('oledInfo');
const oledCtx = oledCanvas.getContext('2d');
let oledCacheKey = '';
const renderOled = () => {
    if (!emu || !emu.oled) { oledInfo.textContent = 'no OLED firmware'; return; }
    const key = emu.oled.frame();
    if (key === oledCacheKey) return;
    oledCacheKey = key;
    const fb = emu.oled.fb, img = oledCtx.createImageData(128, 64), d = img.data;
    for (let i = 0; i < 128 * 64; i++) {
        const v = fb[i] ? 255 : 0;
        d[i * 4] = v; d[i * 4 + 1] = v; d[i * 4 + 2] = v; d[i * 4 + 3] = 255;
    }
    oledCtx.imageSmoothingEnabled = false;
    oledCtx.putImageData(img, 0, 0);
    oledInfo.textContent = `SSD1306 128×64 framebuffer — frame ${key}`;
};

const tftCanvas = $('tftCanvas'), tftInfo = $('tftInfo');
const tftCtx = tftCanvas.getContext('2d');
let tftCacheKey = '';
const renderTft = () => {
    if (!emu || !emu.tft) { tftInfo.textContent = 'no TFT firmware'; return; }
    const { w, h, fb } = emu.tft;
    const key = emu.tft.frame() + ':' + w + 'x' + h;
    if (key === tftCacheKey) return;
    tftCacheKey = key;
    const img = tftCtx.createImageData(w, h), d = img.data;
    for (let i = 0, p = 0; i < w * h; i++, p += 2) {
        const v = (fb[p] << 8) | fb[p + 1];   // parser stores RGB565 big-endian
        d[i * 4] = ((v >> 11) & 0x1F) << 3;
        d[i * 4 + 1] = ((v >> 5) & 0x3F) << 2;
        d[i * 4 + 2] = (v & 0x1F) << 3;
        d[i * 4 + 3] = 255;
    }
    tftCtx.imageSmoothingEnabled = false;
    tftCtx.putImageData(img, 0, 0);
    tftInfo.textContent = `ILI9341 ${w}×${h} RGB565 — frame ${emu.tft.frame()}`;
};

const buzzerInfo = $('buzzerInfo');
let buzzerCacheKey = '';
const renderBuzzer = () => {
    if (!emu || !emu.buzzer) { buzzerInfo.textContent = 'no buzzer firmware'; return; }
    const f = emu.buzzer.freq, duty = emu.buzzer.duty, ch = emu.buzzer.change;
    const key = f.toFixed(1) + '/' + duty.toFixed(3);
    if (key === buzzerCacheKey) return;
    buzzerCacheKey = key;
    buzzerInfo.textContent = f > 1
        ? `TIM2 CH1 PWM ${f.toFixed(0)} Hz, duty ${(duty * 100).toFixed(0)}% — ${ch} note changes`
        : `TIM2 idle — ${ch} note changes`;
};

const speakerInfo = $('speakerInfo');
let audioCtx = null, audioNextTime = 0, audioQueued = 0;
const renderSpeaker = () => {
    if (!emu || !emu.takeSpeakerSamples) { speakerInfo.textContent = 'no audio firmware'; return; }
    const samples = emu.takeSpeakerSamples();
    if (!samples.length) return;
    if (!audioCtx) {
        try { audioCtx = new (window.AudioContext || window.webkitAudioContext)(); } catch (e) {}
        if (!audioCtx) { speakerInfo.textContent = 'WebAudio unavailable'; return; }
        audioNextTime = audioCtx.currentTime + 0.05;
    }
    if (audioCtx.state === 'suspended') audioCtx.resume();
    let off = 0;
    while (off < samples.length) {
        const n = Math.min(4096, samples.length - off);
        const buf = audioCtx.createBuffer(1, n, audioCtx.sampleRate);
        const ch = buf.getChannelData(0);
        for (let i = 0; i < n; i++) ch[i] = samples[off + i];
        const src = audioCtx.createBufferSource();
        src.buffer = buf;
        src.connect(audioCtx.destination);
        src.start(audioNextTime);
        audioNextTime += n / audioCtx.sampleRate;
        off += n;
        audioQueued += n;
    }
    speakerInfo.textContent = `I2S capture playing — ${(audioQueued / audioCtx.sampleRate).toFixed(1)} s queued`;
};

let rtcCacheKey = '';
const renderRtc = () => {
    if (!emu || !emu.rtc || !emu.rtc.time) {
        if (!emu || !emu.rtc) $('rtcInfo').textContent = 'no RTC firmware';
        return;
    }
    const t = emu.rtc.time, temp = emu.rtc.temp;
    const key = `${t.sec}:${t.min}:${t.hour}:${t.dow}:${t.day}:${t.mon}:${t.year}:${temp}`;
    if (key === rtcCacheKey) return;
    rtcCacheKey = key;
    const pad = (n) => String(n).padStart(2, '0');
    $('rtcInfo').textContent =
        `time ${pad(t.hour)}:${pad(t.min)}:${pad(t.sec)} DOW=${t.dow} ` +
        `${pad(t.day)}/${pad(t.mon)}/${pad(t.year)} temp=${temp.toFixed(2)} C`;
};

const renderDevices = () => { renderOled(); renderTft(); renderBuzzer(); renderSpeaker(); renderRtc(); };

// ── GPIO banks A–E ─────────────────────────────────────────────────────────
const GPIO_BASE = 0x40020000, GPIO_STRIDE = 0x400;
const BANKS = ['A', 'B', 'C', 'D', 'E'];
let gpioBuilt = false;

const buildGpio = () => {
    if (gpioBuilt) return;
    gpioBuilt = true;
    const el = $('gpio');
    for (const b of BANKS) {
        const div = document.createElement('div');
        div.className = 'bank';
        div.innerHTML = `<span class="bname">P${b}</span><span class="pins">${Array.from({ length: 16 }, (_, i) => `<span class="pin" id="pin${b}${i}"></span>`).join('')}</span>`;
        const regs = document.createElement('div');
        regs.className = 'regs';
        regs.id = 'regs' + b;
        div.appendChild(regs);
        el.appendChild(div);
    }
};

const refreshGpio = async () => {
    if (!emu) return;
    buildGpio();
    for (const b of BANKS) {
        const idx = BANKS.indexOf(b);
        const base = GPIO_BASE + idx * GPIO_STRIDE;
        const [moder, odr, idr] = await Promise.all([
            emu.read32(base + 0x00), emu.read32(base + 0x14), emu.read32(base + 0x10),
        ]);
        for (let p = 0; p < 16; p++) {
            const mode = (moder >> (p * 2)) & 0x3;
            const out = mode === 1;
            const on = out ? ((odr >> p) & 1) !== 0 : ((idr >> p) & 1) !== 0;
            const pin = $('pin' + b + p);
            pin.className = 'pin ' + (out ? (on ? 'out-on' : 'out-off') : on ? 'in-on' : '');
        }
        $('regs' + b).textContent = `MODER=${hex32(moder)} ODR=${hex32(odr)} IDR=${hex32(idr)}`;
    }
};

// ── peripherals ────────────────────────────────────────────────────────────
const PERIPH_REGS = [
    ['ETH DMASR', 0x40029014], ['ETH MACCR', 0x40028000],
    ['USART1 SR', 0x40011000], ['USART1 BRR', 0x40011008],
    ['RCC AHB1ENR', 0x40023830], ['GPIOA ODR', 0x40020014],
];
const refreshPeriph = async () => {
    if (!emu) return;
    const el = $('periph');
    el.innerHTML = '';
    for (const [name, addr] of PERIPH_REGS) {
        const div = document.createElement('div');
        div.className = 'line';
        const v = await emu.read32(addr);
        div.innerHTML = `<span>${name} (${hex32(addr)})</span><b>${hex32(v)}</b>`;
        el.appendChild(div);
    }
};

// ── symbols ────────────────────────────────────────────────────────────────
const renderSymbols = (symbols) => {
    const el = $('symbols');
    el.textContent = '';
    if (!symbols || !symbols.length) {
        const p = document.createElement('p');
        p.className = 'sec';
        p.style.padding = '8px 12px';
        p.textContent = 'Load a .map file or an .elf to list symbols here.';
        el.appendChild(p);
        return;
    }
    const max = 200;
    for (const s of symbols.slice(0, max)) {
        const div = document.createElement('div');
        div.className = 'sym';
        div.innerHTML = `<span>${s.name}</span><span>${hex32(s.addr)}${s.size ? ' (' + s.size + ')' : ''}</span>`;
        el.appendChild(div);
    }
    if (symbols.length > max) {
        const p = document.createElement('p');
        p.className = 'sec';
        p.style.padding = '4px 12px';
        p.textContent = `… ${symbols.length - max} more`;
        el.appendChild(p);
    }
};

// ── idle until the user picks a firmware (auto-boot only via ?fw=) ─────────
setBusy(false);
$('btnRun').textContent = 'Run';
setStatus('idle — select a firmware and press Boot', 'stop');
// "below" was wrong — the Firmware panel is in the sidebar to the right
// (and the panel names now match what the sidebar actually says).
appendUart('STM32F407 console ready. Nothing is running yet.\r\n'
    + 'Pick a firmware under FIRMWARE (right) and press "Boot preset",\r\n'
    + 'or upload your own .bin/.hex/.elf/.map. Firmware output appears here.\r\n');

const preset = params.get('fw');
if (preset && FIRMWARES[preset]) {
    $('fwSelect').value = preset;
    $('btnBoot').click();
}
