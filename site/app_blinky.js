// In-browser driver for the blinky (non-ethernet) demo.
// Boots the bundled blinky firmware, drives the emulator with
// requestAnimationFrame, renders UART output + a live LED wired to the
// emulated GPIOA ODR bit 5, and stats. No network anywhere.
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { FIRMWARES } from './firmware.js';

const $ = (id) => document.getElementById(id);
const uartEl = $('uart'), statusEl = $('statusText'), dotEl = $('dot');
const ledEl = $('led');

const decodeB64 = (b64) => {
    const bin = atob(b64);
    const u = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) u[i] = bin.charCodeAt(i);
    return u;
};

let session = 0;
let emu = null, running = false, paused = false;
let uartBuf = '', totalInst = 0, t0 = performance.now(), lastInst = 0, lastT = t0;
let ledOn = false, toggles = 0;

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
    $('stPc').textContent = regs ? '0x' + (regs.PC >>> 0).toString(16) : '—';
    $('stToggles').textContent = toggles;
};

const refreshLed = () => {
    if (!emu) return;
    const odr = emu.read32(0x40020014) & 0x20; // GPIOA ODR bit 5 (PA5)
    const on = odr !== 0;
    if (on !== ledOn) { toggles++; ledOn = on; }
    ledEl.className = 'led ' + (on ? 'on' : 'off');
    $('stLedHex').textContent = 'ODR bit5 = ' + (on ? '1' : '0');
};

let stepsDone = 0;
const raf = () => new Promise((r) => requestAnimationFrame(r));

const boot = async (fw, uploadedBytes) => {
    const id = ++session;
    running = true;
    setStatus('booting…', 'stop');
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }

    const firmware = uploadedBytes || decodeB64(FIRMWARES[fw].bytes);
    uartEl.textContent = uartBuf = '';
    totalInst = 0; stepsDone = 0; toggles = 0; ledOn = false;
    ledEl.className = 'led off';
    t0 = lastT = performance.now(); lastInst = 0;

    const svdXml = await fetch('vendor/stm32f407.svd').then((r) => r.text());
    if (id !== session) return;

    emu = await createEmulator({ firmware, bindings, unicorn: MUnicorn, svdXml });
    if (id !== session) { emu.close(); return; }

    appendUart(`── booted ${uploadedBytes ? 'custom firmware' : FIRMWARES[fw].name} ──\r\n`);
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
        refreshLed();
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

boot('blinky');
