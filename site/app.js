// In-browser driver for the STM32F407 demo.
// Boots a bundled firmware, drives the emulator with requestAnimationFrame,
// and renders UART output + stats + simulated network packets.
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { createNetSim } from './netsim.js';
import { FIRMWARES } from './firmware.js';

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

let session = 0;           // bumped on every reset; stale loops exit
let emu = null, netsim = null, running = false, paused = false;
let uartBuf = '', totalInst = 0, t0 = performance.now(), lastInst = 0, lastT = t0;

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

const addFrame = (dir, pkt) => {
    const meta = describeFrame(dir, pkt);
    const div = document.createElement('div');
    div.className = 'frame ' + dir;
    div.innerHTML = `<span class="tag">${dir === 'tx' ? 'TX' : 'RX'}</span> ${pkt.length}B ${meta}<pre>${hex(pkt, 48)}</pre>`;
    framesEl.prepend(div);
    while (framesEl.children.length > 60) framesEl.lastChild.remove();
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
    const rounds = (uartBuf.match(/=== HTTP \d+b ===/g) || []).length;
    $('stRounds').textContent = rounds;
    $('stPc').textContent = regs ? '0x' + (regs.PC >>> 0).toString(16) : '—';
    const s = netsim ? netsim.stats : null;
    $('stNet').textContent = s ? `${s.httpResponses} HTTP / ${s.dhcpAcks} DHCP / ${s.tx} TX` : '—';
};

let stepsDone = 0;
const raf = () => new Promise((r) => requestAnimationFrame(r));

const boot = async (fw, uploadedBytes) => {
    const id = ++session;
    paused = false;
    setStatus('booting…', 'stop');
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }

    const firmware = uploadedBytes || decodeB64(FIRMWARES[fw].bytes);
    uartEl.textContent = uartBuf = '';
    framesEl.textContent = '';
    totalInst = 0; stepsDone = 0;
    t0 = lastT = performance.now(); lastInst = 0;

    const svdXml = await fetch('vendor/stm32f407.svd').then((r) => r.text());
    if (id !== session) return; // reset while loading

    netsim = uploadedBytes ? null : createNetSim();
    emu = await createEmulator({
        firmware,
        bindings,
        unicorn: MUnicorn,
        svdXml,
        onTx: (pkt) => {
            addFrame('tx', pkt);
            if (netsim) {
                for (const reply of netsim.onTx(pkt)) {
                    addFrame('rx', reply);
                    emu.injectFrame(reply);
                }
            }
        },
    });
    if (id !== session) { emu.close(); return; }

    appendUart(`── booted ${uploadedBytes ? 'custom firmware' : FIRMWARES[fw].name} ──\r\n`);
    running = true;
    $('btnRun').textContent = 'Pause';
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
            $('btnRun').textContent = 'Resume';
            return;
        }
        appendUart(emu.drainUart());
        refreshStats();
        await raf();
    }
};

$('btnRun').addEventListener('click', () => {
    if (!emu) return;
    running = !running;
    $('btnRun').textContent = running ? 'Pause' : 'Resume';
    setStatus(running ? 'running' : 'paused', running ? 'run' : 'stop');
});
$('btnReset').addEventListener('click', () => boot($('fwSelect').value));
$('btnClear').addEventListener('click', () => { uartEl.textContent = uartBuf = ''; });
$('btnLoad').addEventListener('click', () => boot($('fwSelect').value));
$('btnUpload').addEventListener('click', async () => {
    const f = $('fwFile').files[0];
    if (!f) return;
    const bytes = new Uint8Array(await f.arrayBuffer());
    await boot(null, bytes);
});
window.addEventListener('error', (e) => setStatus('error: ' + e.message, 'err'));

boot('eth_http');
