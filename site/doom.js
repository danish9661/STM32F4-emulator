// DOOM browser demo: boots the doomgeneric F407 port on the emulator,
// feeds keys through the SRAM ABI ring, renders the 320x200 framebuffer
// through the guest-exported palette.
// Serve from site/ (python3 -m http.server 8123 --directory site) — the
// page fetches the WAD + SVD + wasm at runtime (file:// won't work).
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { FIRMWARES } from './firmware.js';

const $ = (id) => document.getElementById(id);
const canvas = $('screen');
const ctx = canvas.getContext('2d');
const img = ctx.createImageData(320, 200);

// Pacing: the guest's DG_SleepMs(15) is now a no-op (no busy-wait — that
// used to waste ~360k instructions per frame).  The guest advances its own
// frame counter (ABI +0x514) once per rendered frame at 35 tics/s of game
// time, so the driver paces by wall clock: run steps until the guest's frame
// count catches up to realtime 35 fps.  Any machine above ~3.5 MIPS holds
// exactly 35 fps; slower machines degrade gracefully (as fast as they can).
const FRAME_MS = 1000 / 35;      // one game frame (tic) per 28.57ms realtime
const STEP_BUDGET = 12;          // max steps per rAF
const MS_BUDGET = 16;            // max wall time per rAF

// Doom keycodes (engine/doomkeys.h; TranslateKey is identity)
const KEY = {
    ESC: 0x1B, ENTER: 0x0D,
    LEFTARROW: 0xAC, RIGHTARROW: 0xAE, UPARROW: 0xAD, DOWNARROW: 0xAF,
    STRAFE_L: 0xA0, STRAFE_R: 0xA1, USE: 0xA2, FIRE: 0xA3,
    F1: 0x80, F2: 0x81, F3: 0x82, F6: 0x85, F9: 0x88,
    F10: 0x89, F11: 0x8A, F12: 0x8B,
};
const DOM_TO_DOOM = {
    'Enter': KEY.ENTER, 'Escape': KEY.ESC,
    'ArrowLeft': KEY.LEFTARROW, 'ArrowRight': KEY.RIGHTARROW,
    'ArrowUp': KEY.UPARROW, 'ArrowDown': KEY.DOWNARROW,
    'w': 0x77, 'a': 0x61, 's': 0x73, 'd': 0x64,
    'ShiftLeft': KEY.STRAFE_L, 'ShiftRight': KEY.STRAFE_R,
    ' ': KEY.USE, 'ControlLeft': KEY.FIRE, 'ControlRight': KEY.FIRE,
    'F1': KEY.F1, 'F2': KEY.F2, 'F3': KEY.F3, 'F6': KEY.F6,
    'F9': KEY.F9, 'F10': KEY.F10, 'F11': KEY.F11, 'F12': KEY.F12,
};

const ABI = 0x20002000n;
const KEYWR = ABI;                 // u32 write index (JS)
const KEYRD = ABI + 0x04n;         // u32 read index (guest)
const RING = ABI + 0x08n;          // 256-byte ring, 2 bytes/event
const DGSB = ABI + 0x510n;         // u32 DG_ScreenBuffer
const FRAMECOUNT = ABI + 0x514n;   // u32 rendered-frame counter (guest)
const PALETTE = ABI + 0x110n;      // 1024 B BGRA

let emu = null, uc = null;
let keyWr = 0;
const held = new Set();            // doom keycodes currently held down
const sentPos = new Map();         // code -> ring byte position of its keydown
const upPending = new Map();       // code -> D ring position; U waits until the guest consumed the D
let fbAddr = 0n;
let fbHash = 0, framesShown = 0;
let uartBuf = '';
let paused = false;
let booted = false;
let instTotal = 0;
let statLast = performance.now(), statInst = 0;
let activeMs = 0;                    // wall time spent inside emu.step() (MIPS meter)
let paceWall = 0, paceFrames = -1;  // realtime pacing anchor (wall ms, frame#)

function setStatus(s, cls) {
    const el = $('status');
    el.textContent = s;
    el.className = cls || '';
}

function sendKey(code, pressed) {
    if (!uc) return 0;
    const off = keyWr % 256;
    uc.mem_write(RING + BigInt(off), Uint8Array.of(code));
    uc.mem_write(RING + BigInt((off + 1) % 256), Uint8Array.of(pressed ? 0x80 : 0));
    keyWr = (keyWr + 2) % 256;
    emu.write32(KEYWR, keyWr);
    return off;
}

// momentary press (down + up pair)
function tap(code) { sendKey(code, true); sendKey(code, false); }

function updateKeysLabel() {
    const names = [];
    for (const k of held) {
        names.push(Object.keys(DOM_TO_DOOM).find((d) => DOM_TO_DOOM[d] === k && d.length === 1) ||
                    (k === KEY.STRAFE_L ? 'Shift' : k === KEY.FIRE ? 'Ctrl' : k === KEY.USE ? 'Space' : 'k' + k.toString(16)));
    }
    $('keys').textContent = names.length ? 'held: ' + names.join(' ') : '';
}

// The guest's I_GetEvent drains the key ring and BREAKS on the first keyup
// (engine/i_input.c), and the ring is only drained in bursts (once per guest
// tic-batch, when NetUpdate's wall-clock delta allows a new ticcmd). If a
// (D,U) pair sits in the ring when a drain runs, gamekeydown[] is set and
// cleared within one tic — in-game turn/move keys would never register
// (menus are unaffected since they act on single events). So a keyup is held
// back until the guest's ring read cursor (keyRd) proves it consumed the
// keydown AND the user has actually released the key; the U then lands in a
// LATER drain. Held keys need no re-assert: the engine's gamekeydown[]
// persists from the single keydown until the U.
function flushUpPending() {
    if (!upPending.size) return;
    const keyRd = emu.read32(KEYRD);
    for (const [code, dPos] of upPending) {
        if (held.has(code)) continue;        // still held: keep gamekeydown[] true
        if (keyRd !== dPos) {                // guest cursor moved past our keydown
            sendKey(code, false);
            upPending.delete(code);
        }
    }
}

document.addEventListener('keydown', (e) => {
    const code = DOM_TO_DOOM[e.key];
    if (code === undefined) return;
    e.preventDefault();
    if (!held.has(code)) {
        sentPos.set(code, sendKey(code, true));
        held.add(code);
        updateKeysLabel();
    }
});
document.addEventListener('keyup', (e) => {
    const code = DOM_TO_DOOM[e.key];
    if (code === undefined) return;
    e.preventDefault();
    if (held.delete(code)) {
        const dPos = sentPos.get(code);
        sentPos.delete(code);
        upPending.set(code, dPos);
        updateKeysLabel();
    }
});
canvas.addEventListener('mousedown', () => {
    if (document.pointerLockElement !== canvas) {
        canvas.requestPointerLock();
        return;
    }
    sentPos.set(KEY.FIRE, sendKey(KEY.FIRE, true));
    held.add(KEY.FIRE);
    updateKeysLabel();
});
document.addEventListener('mouseup', () => {
    if (held.delete(KEY.FIRE)) {
        const dPos = sentPos.get(KEY.FIRE);
        sentPos.delete(KEY.FIRE);
        upPending.set(KEY.FIRE, dPos);
        updateKeysLabel();
    }
});

$('btnPause').addEventListener('click', () => {
    paused = !paused;
    $('btnPause').textContent = paused ? 'Resume' : 'Pause';
});
$('btnReset').addEventListener('click', boot);
$('btnFull').addEventListener('click', () => {
    const el = document.fullscreenElement ? document : $('screenWrap');
    (document.fullscreenElement ? document.exitFullscreen() : el.requestFullscreen()).catch(() => {});
});
$('btnClear').addEventListener('click', () => { $('uart').textContent = uartBuf = ''; });

function appendUart(chunk) {
    if (!chunk) return;
    uartBuf += chunk.replace(/\r/g, '');
    const el = $('uart');
    el.textContent = uartBuf;
    el.scrollTop = el.scrollHeight;
}

function fitCanvas() {
    // contain-fit: largest 16:10 box inside the wrap, centered by flex
    const wrap = $('screenWrap').getBoundingClientRect();
    const scale = Math.min(wrap.width / 640, wrap.height / 400);
    canvas.style.width = Math.max(2, Math.floor(640 * scale)) + 'px';
    canvas.style.height = Math.max(2, Math.floor(400 * scale)) + 'px';
}
window.addEventListener('resize', fitCanvas);

async function boot() {
    setStatus('booting…');
    fitCanvas();                 // layout first; re-fit when the emulator is up
    paused = false;
    $('btnPause').textContent = 'Pause';
    held.clear();
    updateKeysLabel();
    keyWr = 0;
    uartBuf = '';
    $('uart').textContent = '';
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }

    try {
        const [svdXml, wad] = await Promise.all([
            fetch('vendor/stm32f407.svd').then((r) => r.text()),
            fetch('doom1.wad').then((r) => r.arrayBuffer()),
        ]);
        const firmware = new Uint8Array(atob(FIRMWARES.doom.bytes).split('').map((c) => c.charCodeAt(0)));

        emu = await createEmulator({
            firmware,
            bindings,
            unicorn: MUnicorn,
            svdXml,
            extra_ram: [
                { addr: 0xC0000000, size: 16 * 1024 * 1024 },   // .data/.bss + zone + heap
                { addr: 0xB8000000, size: 8 * 1024 * 1024 },    // WAD image
            ],
            extra_mem: [{ addr: 0xB8000000, data: new Uint8Array(wad) }],
            minimalPolls: true, blockCounting: true,
        });
        uc = emu.uc;
        window.__emu = emu;
        fbAddr = 0n;
        fitCanvas();
        booted = true;
        setStatus('running — DOOM booting (WAD at 0xB8000000)');
        requestAnimationFrame(loop);
    } catch (e) {
        setStatus('boot failed: ' + e.message, 'error');
        console.error(e);
    }
}

function fnv1a(data) {
    let h = 0x811c9dc5;
    for (let i = 0; i < data.length; i++) {
        h ^= data[i];
        h = Math.imul(h, 0x01000193);
    }
    return h >>> 0;
}

// renders one game frame; returns true if the framebuffer changed
function renderFb() {
    if (!emu) return false;
    const dgsb = emu.read32(DGSB);
    if (dgsb !== 0 && dgsb !== Number(fbAddr)) fbAddr = BigInt(dgsb >>> 0);
    if (fbAddr === 0n) return false;
    let fb;
    try {
        fb = new Uint8Array(uc.mem_read(fbAddr, 320 * 200));
    } catch (e) {
        return false;
    }
    const h = fnv1a(fb);
    if (h === fbHash) return false;
    fbHash = h;
    const pal = new Uint8Array(uc.mem_read(PALETTE, 256 * 4));
    const d = img.data;
    for (let i = 0; i < 64000; i++) {
        const p = fb[i] * 4;
        d[i * 4] = pal[p + 2];      // BGRA -> RGBA
        d[i * 4 + 1] = pal[p + 1];
        d[i * 4 + 2] = pal[p];
        d[i * 4 + 3] = 255;
    }
    ctx.putImageData(img, 0, 0);
    return true;
}

async function loop() {
    if (!booted || !emu) return;
    if (!paused) {
        try {
            flushUpPending();        // keyups land on the NEXT rAF, after the guest consumed the down
            const t0 = performance.now();
            const stepT0 = performance.now();
            let steps = 0;
            let frames = emu.read32(FRAMECOUNT);
            // anchor pacing at the FIRST rendered frame (before that, boot runs
            // at full speed); re-anchor after a pause
            if (frames > 0 && paceFrames < 0) {
                paceFrames = frames;
                paceWall = performance.now();
            }
            // realtime pacing: run until the guest's frame counter catches the
            // wall clock at 35 fps (28.57ms per frame)
            let target = Infinity;
            if (paceFrames >= 0) target = paceFrames + Math.floor((performance.now() - paceWall) / FRAME_MS);
            while (steps < STEP_BUDGET && performance.now() - t0 < MS_BUDGET && frames < target) {
                const res = emu.step();
                instTotal = res.instCount;   // cumulative counter
                steps++;
                frames = emu.read32(FRAMECOUNT);
            }
            activeMs += performance.now() - stepT0;
        } catch (e) {
            setStatus('emulator error: ' + e.message, 'error');
            return;
        }
        appendUart(emu.drainUart());
        if (renderFb()) framesShown++;

        const now = performance.now();
        if (now - statLast >= 500) {
            const dt = (now - statLast) / 1000;
            const mips = activeMs > 0 ? (instTotal - statInst) / (activeMs / 1000) / 1e6 : 0;
            $('stats').textContent =
                `MIPS: ${mips.toFixed(1)} · FPS: ${(framesShown / dt).toFixed(0)} · ${(instTotal / 1e6).toFixed(1)}M inst`;
            statLast = now; statInst = instTotal;
            activeMs = 0;
            framesShown = 0;
        }
    } else {
        paceFrames = -1;   // re-anchor on resume so no catch-up burst
    }
    requestAnimationFrame(loop);
}

boot();
