// In-browser driver for the STM32F407 single-page console.
// Preset + custom (.bin/.hex/.elf/.map) firmware loading, Run/Stop/Reset,
// an optional WebSocket gateway (real network stack) with a netsim fallback,
// live UART terminal, GPIO/peripheral register readout, and packet viewer.
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';
import { FIRMWARES } from './firmware.js';
import { parseIntelHex, parseElf, parseMap } from './loaders.js';

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

// Interrupt-driven firmware: the emulator pumps guest IRQ handlers (USART RXNE
// etc.). OFF for ETH firmware — the driver signals completion via SRAM
// irq_flag and the guest ETH_IRQHandler would double-process DMASR/rx_desc.
const IRQ_FIRMWARES = new Set(['rx_interrupt_test', 'rx_crypto_test']);

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
const gw = { ws: null, connected: false };
const setGwStatus = (connected, text) => {
    gw.connected = connected;
    $('gwDot').className = 'dot ' + (connected ? 'run' : 'stop');
    $('gwStatus').textContent = text;
    $('btnGw').textContent = connected ? 'Disconnect' : 'Connect';
};

const connectGateway = () => {
    const url = $('gwUrl').value.trim() || 'ws://127.0.0.1:5099/api/network-gateway';
    if (!/^wss?:\/\//.test(url)) return setGwStatus(false, 'bad URL — expected ws:// or wss://');
    if (gw.ws) {
        try { gw.ws.close(); } catch (e) {}
        gw.ws = null;
        setGwStatus(false, 'offline — using the scripted network (netsim)');
        return;
    }
    let ws;
    try { ws = new WebSocket(url); } catch (e) { return setGwStatus(false, 'connect failed: ' + e.message); }
    ws.binaryType = 'arraybuffer';
    ws.onopen = () => {
        gw.ws = ws;
        setGwStatus(true, 'connected — Ethernet frames go to the real stack');
        appendUart(`── gateway connected (${url}) ──\r\n`);
    };
    ws.onmessage = (ev) => {
        if (typeof ev.data === 'string') return; // control message (e.g. RESET)
        const buf = new Uint8Array(ev.data);
        if (emu) emu.injectFrame(buf);
    };
    ws.onclose = () => {
        if (gw.ws !== ws) return;
        gw.ws = null;
        setGwStatus(false, 'disconnected — using the scripted network (netsim)');
    };
    ws.onerror = () => setGwStatus(false, 'connection error — using the scripted network (netsim)');
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

const boot = async () => {
    const id = ++session;
    running = false;
    setBusy(false);
    $('btnRun').textContent = 'Run';
    setStatus('booting…', 'stop');
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }

    const fw = image.flash;
    uartEl.textContent = uartBuf = '';
    framesEl.textContent = '';
    totalInst = 0; stepsDone = 0;
    t0 = lastT = performance.now(); lastInst = 0;
    $('stFw').textContent = image.name;

    const svdXml = await fetch('vendor/stm32f407.svd').then((r) => r.text());
    if (id !== session) return;

    netsim = gw.connected ? null : createNetSim();
    emu = await createEmulator({
        firmware: fw,
        bindings,
        unicorn: MUnicorn,
        svdXml,
        extra_mem: image.extraMem,
        uart_addr: image.uartAddr,
        enable_irqs: IRQ_FIRMWARES.has(image.name),
        onTx: (pkt) => {
            addFrame('tx', pkt);
            if (gw.connected && gw.ws) {
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

    appendUart(`── booted ${image.name} ${gw.connected ? '(gateway)' : '(netsim)'} ──\r\n`);
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
            const res = emu.step();
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

const sendRx = () => {
    const input = $('rxInput');
    const text = input.value;
    if (!text || !emu) return;
    appendUart('> ' + text + '\r\n');
    const bytes = new Uint8Array(text.length);
    for (let i = 0; i < text.length; i++) bytes[i] = text.charCodeAt(i) & 0xFF;
    emu.sendUart(bytes);
    input.value = '';
};
$('btnSend').addEventListener('click', sendRx);
$('rxInput').addEventListener('keydown', (e) => {
    if (e.key === 'Enter') sendRx();
});
window.addEventListener('error', (e) => setStatus('error: ' + e.message, 'err'));

// ── stats ──────────────────────────────────────────────────────────────────
const refreshStats = () => {
    const regs = emu && emu.getRegisters ? emu.getRegisters() : null;
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
    refreshGpio();
    refreshPeriph();
};

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

const refreshGpio = () => {
    if (!emu) return;
    buildGpio();
    for (const b of BANKS) {
        const idx = BANKS.indexOf(b);
        const base = GPIO_BASE + idx * GPIO_STRIDE;
        const moder = emu.read32(base + 0x00), odr = emu.read32(base + 0x14), idr = emu.read32(base + 0x10);
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
const refreshPeriph = () => {
    if (!emu) return;
    const el = $('periph');
    el.innerHTML = '';
    for (const [name, addr] of PERIPH_REGS) {
        const div = document.createElement('div');
        div.className = 'line';
        div.innerHTML = `<span>${name} (${hex32(addr)})</span><b>${hex32(emu.read32(addr))}</b>`;
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
appendUart('STM32F407 console ready.\r\nSelect a preset below or upload .bin/.hex/.elf/.map, then press Boot.\r\n');

const params = new URLSearchParams(location.search);
const preset = params.get('fw');
if (preset && FIRMWARES[preset]) {
    $('fwSelect').value = preset;
    $('btnBoot').click();
}
