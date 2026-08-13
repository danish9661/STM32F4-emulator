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
const STEP_INST = 60000;         // instructions per emu.step() (~0.66 frames;
                                 // small steps = <1-frame pacing overshoot +
                                 // tight clock updates for time-based waits)

// Low-detail render (guest global detailLevel @ 0xC0016814, 0=high 1=low):
// the engine renders every other column -> ~30-40% fewer guest instructions
// per frame.  Default ON for speed; toggled from the topbar (persisted).
const DETAIL_LEVEL = 0xC0016814;
let lowDetail = localStorage.getItem('doomDetail') !== '0';
const detailToggle = $('detail');
detailToggle.checked = lowDetail;
detailToggle.addEventListener('change', () => {
    lowDetail = detailToggle.checked;
    localStorage.setItem('doomDetail', lowDetail ? '1' : '0');
    if (emu) emu.write32(DETAIL_LEVEL, lowDetail ? 1 : 0);
});

// Doom keycodes (engine/doomkeys.h; TranslateKey is identity)
const KEY = {
    ESC: 0x1B, ENTER: 0x0D,
    LEFTARROW: 0xAC, RIGHTARROW: 0xAE, UPARROW: 0xAD, DOWNARROW: 0xAF,
    STRAFE_L: 0xA0, STRAFE_R: 0xA1, USE: 0xA2, FIRE: 0xA3,
    F1: 0xBB, F2: 0xBC, F3: 0xBD, F6: 0xC0, F9: 0xC3,
    F10: 0xC4, F11: 0xC5, F12: 0xC6,
};
const DOM_TO_DOOM = {
    'Enter': KEY.ENTER, 'Escape': KEY.ESC,
    'ArrowLeft': KEY.LEFTARROW, 'ArrowRight': KEY.RIGHTARROW,
    'ArrowUp': KEY.UPARROW, 'ArrowDown': KEY.DOWNARROW,
    // WASD -> movement keys: the engine's default bindings (m_controls.c)
    // are arrow-only, so translate here instead of rebinding in the guest.
    'w': KEY.UPARROW, 's': KEY.DOWNARROW, 'a': KEY.LEFTARROW, 'd': KEY.RIGHTARROW,
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
const CLOCKMS = ABI + 0x518n;      // u32 ms clock (guest reads via DG_GetTicksMs)
const PALETTE = ABI + 0x110n;      // 1024 B BGRA
const SAVEFLAG = ABI + 0x51Cn;     // u32 guest->driver: 1=save written, 2=load req
const SAVESIZE = ABI + 0x520n;     // u32 blob bytes
const SAVEREADY = ABI + 0x524n;    // u32 driver->guest: requested slot restored
const SAVESLOT = ABI + 0x528n;     // u32 guest->driver: slot index
const SAVEMAP = ABI + 0x52Cn;      // u32 driver->guest: bit N = slot N saved
const SAVEADDR = 0xC0080000n;      // EXTRAM save area (2 slots x 256 KB)
const SAVESLOTSIZE = 0x40000n;
const SAVE_SLOTS = 2;

let emu = null, uc = null;
let keyWr = 0;
const held = new Set();            // doom keycodes currently held down
const sentPos = new Map();         // code -> ring byte position of its keydown
const upPending = new Map();       // code -> D ring position; U waits until the guest consumed the D
let fbAddr = 0n;
let framesShown = 0;
let uartBuf = '';
let paused = false;
let booted = false;
window.__pump = () => pumpAudio(); // debug handle for hidden-tab tests
window.__audioNode = null; // set by initAudio for tests
let instTotal = 0;
let lastRAF = 0;                     // last rAF timestamp — when stale (>200ms)
let lastPump = 0;                    // last pump burst — rAF must not start
                                     // back-to-back (gap-free melt-crossing
                                     // stepping wedges the TCI interpreter)
                                     // rAF is dead (hidden tab) and the
                                     // worklet hunger pump takes over driving
let statLast = performance.now(), statInst = 0;
let activeMs = 0;                    // wall time spent inside emu.step() (MIPS meter)
let paceWall = 0, paceFrames = -1;  // realtime pacing anchor (wall ms, frame#)
let bootClock = 0;
let clockMs = 0;                    // monotonic guest clock: never written
                                    // below the previous value (the melt wipe
                                    // waits on tics = now - start and would
                                    // spin forever on a backwards clock)                  // wall time at boot; guest clock is derived
                                    // from FRAMECOUNT so guest time NEVER runs
                                    // ahead of its own execution (realtime lock)

// ── audio: drain the model's I2S1 capture FIFO into an AudioWorklet ──
// The worklet (audio-worklet.js) resamples 11025 Hz -> context rate and
// plays continuously on the audio thread — no BufferSource scheduling
// jitter, latency bounded by its MAX_QUEUE drop guard.  The guest is
// realtime-locked (CLOCKMS = bootClock + frames*FRAME_MS), so production is
// ~11025 samples/s wall; the worklet only underruns on slow machines.
let audioCtx = null, audioNode = null;
window.__audioTotal = 0;            // samples drained (smoke assertions)

async function initAudio() {
    if (audioCtx) return;
    try {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)();
        await audioCtx.audioWorklet.addModule('audio-worklet.js?v=19');
        audioNode = new AudioWorkletNode(audioCtx, 'doom-audio');
        window.__audioNode = audioNode;
        audioNode.connect(audioCtx.destination);
        // audio-driven stepping (fallback): when the worklet queue runs low
        // it posts 'need' — pump emulation steps to refill.  Only engages
        // when rAF is dead (hidden tab / throttled to <5 Hz) — while visible
        // the rAF loop is the sole driver (no double-driving, no CPU waste).
        audioNode.port.onmessage = (e) => {
            if (e.data === 'need') pumpAudio();
        };
        // self-timed safety net: pump on a timer too, so production never
        // depends on the worklet's message cadence (headless/weird audio
        // clocks deliver 'need' sparsely).  Chrome throttles hidden-tab
        // intervals to ~1 s for audible tabs — each pump then runs the full
        // 1 s of target delta (~35 frames, well under the 200-step cap).
        if (!window.__audioPumpTimer) {
            // 40ms cadence = the rAF loop's proven-safe stepping pattern
            // (12-step bursts, ~40ms apart — 200M+ inst clean in the smoke;
            // sustained gap-free stepping through a melt wipe wedges Chrome's
            // TCI, see AGENTS §7/§16).  Hidden-tab intervals are throttled
            // further by Chrome (~1s for audible tabs) — production then
            // drops to ~1 burst/s, which the worklet queue (0.37s) cannot
            // cover: fully-hidden tabs get partial audio, by browser design.
            window.__audioPumpTimer = setInterval(pumpAudio, 40);
        }
    } catch (e) {
        audioCtx = null;
        audioNode = null;
    }
}

function pumpAudio() {
    if (!booted || paused || !emu) return;
    const now = performance.now();
    if (now - lastRAF <= 200) return;   // rAF alive — it drives
    let frames = emu.read32(FRAMECOUNT);
    const target = paceFrames + Math.floor((now - paceWall) / FRAME_MS);
    let steps = 0;
    // 20k-inst step cap: the §7 TCI wedge is batch-size + env dependent (it
    // only reproduces in Chrome, never Node — see AGENTS §7); 20k is the
    // proven-safe budget there.  The rAF loop (visible tab) keeps 60k —
    // verified clean over 200M+ instructions; the pump path (rAF dead =
    // hidden/throttled) is the risky one, so it gets the conservative cap.
    // 12-step burst (melt-crossing spins wedge Chrome's TCI when stepped
    // gap-free; the 40ms interval cadence is the proven-safe rAF pattern).
    while (steps < 12 && frames < target) {
        clockMs = Math.max(clockMs, Math.floor(bootClock + frames * FRAME_MS) & 0xffffffff);
        emu.write32(CLOCKMS, clockMs);
        const res = emu.step(20000);
        instTotal = res.instCount;   // cumulative counter
        steps++;
        frames = emu.read32(FRAMECOUNT);
    }
    lastPump = performance.now();
    if (!window.__noUart) appendUart(emu.drainUart());
    drainAudio();
    processSaves();
}

function drainAudio() {
    const s = emu.takeSpeakerSamples();
    if (!s || !s.length) return;
    window.__audioTotal += s.length;
    if (window.__noDrain) return;
    if (audioNode) audioNode.port.postMessage(s, [s.buffer]);
}

// ── savegames: mirror the guest's save ABI to localStorage ──
// The guest stages "doomsavN.dsg" blobs in EXTRAM (SAVEADDR) and flags us
// (SAVEFLAG=1 + SAVESLOT + SAVESIZE) when the engine rename-commits a save.
// Loads set SAVEFLAG=2 + SAVESLOT and busy-wait on SAVEREADY until we restore
// the blob (or clear SAVESIZE when the slot is empty).  saveMap (bit N) tells
// the load menu which slots exist.  Polled from the rAF loop and the audio
// pump, so saves/loads resolve within one step burst.
function processSaves() {
    if (!emu) return;
    const flag = emu.read32(SAVEFLAG);
    if (flag === 1) {
        const slot = emu.read32(SAVESLOT);
        const size = emu.read32(SAVESIZE);
        if (slot >= 0 && slot < SAVE_SLOTS && size > 0) {
            const b = new Uint8Array(uc.mem_read(SAVEADDR + BigInt(slot * Number(SAVESLOTSIZE)), size));
            let s = '';
            for (let i = 0; i < b.length; i += 0x8000) {
                s += String.fromCharCode.apply(null, b.subarray(i, i + 0x8000));
            }
            try { localStorage.setItem('doom-save-' + slot, btoa(s)); } catch (e) { /* quota */ }
            emu.write32(SAVEMAP, emu.read32(SAVEMAP) | (1 << slot));
        }
        emu.write32(SAVEFLAG, 0);
    } else if (flag === 2) {
        const slot = emu.read32(SAVESLOT);
        const raw = (slot >= 0 && slot < SAVE_SLOTS) ? localStorage.getItem('doom-save-' + slot) : null;
        if (raw) {
            const bin = atob(raw);
            const b = new Uint8Array(bin.length);
            for (let i = 0; i < bin.length; i++) b[i] = bin.charCodeAt(i);
            uc.mem_write(SAVEADDR + BigInt(slot * Number(SAVESLOTSIZE)), b);
            emu.write32(SAVESIZE, b.length);
        } else {
            emu.write32(SAVESIZE, 0);
        }
        emu.write32(SAVEREADY, 1);
        emu.write32(SAVEFLAG, 0);
    }
}

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
    let code = DOM_TO_DOOM[e.key];
    if (code === undefined && e.key.length === 1 && e.key >= 'a' && e.key <= 'z') {
        code = e.key.charCodeAt(0);    // raw ASCII letters reach the menu
    }                                  // string-entry + 'y' confirm prompts
    if (code === undefined) return;
    e.preventDefault();
    initAudio();                       // user gesture: unlock WebAudio
    if (!held.has(code)) {
        sentPos.set(code, sendKey(code, true));
        held.add(code);
        updateKeysLabel();
    }
});
document.addEventListener('keyup', (e) => {
    let code = DOM_TO_DOOM[e.key];
    if (code === undefined && e.key.length === 1 && e.key >= 'a' && e.key <= 'z') {
        code = e.key.charCodeAt(0);
    }
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
            maxBatch: STEP_INST,
            ext_devices: { speaker: true },   // enable the I2S capture drain
        });
        emu.write32(DETAIL_LEVEL, lowDetail ? 1 : 0);
        uc = emu.uc;
        bootClock = performance.now();
        clockMs = 0;
        paceFrames = 0;               // realtime lock from boot: the guest may
        paceWall = bootClock;         // never produce frames ahead of wall time
        window.__emu = emu;
        fbAddr = 0n;
        // restore the save slot map (which slots have a saved game) so the
        // load menu shows exactly the localStorage-backed slots
        let saveMap = 0;
        for (let i = 0; i < SAVE_SLOTS; i++) {
            if (localStorage.getItem('doom-save-' + i)) saveMap |= 1 << i;
        }
        emu.write32(SAVEMAP, saveMap);
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

// renders one game frame; returns true if the framebuffer changed.
// hash-only gating: the guest frame counter can stall (e.g. the level-start
// melt wipe spins on I_GetTime), so it must never gate the repaint.
let lastHash = -1;
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
    if (h === lastHash) return false;
    lastHash = h;
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
    lastRAF = performance.now();
    if (!paused) {
        try {
            flushUpPending();        // keyups land on the NEXT rAF, after the guest consumed the down
            const t0 = performance.now();
            const stepT0 = performance.now();
            let steps = 0;
            let frames = emu.read32(FRAMECOUNT);
            if (paceFrames < 0) {
                // re-anchor after a pause so no catch-up burst
                paceFrames = frames;
                paceWall = performance.now();
            }
            // realtime lock: paceFrames/paceWall anchor at boot (0, bootClock).
            // The guest may never produce frames faster than wall time; the
            // target counts 35 frames per second.  In a HIDDEN tab rAF stops
            // firing (Chrome pauses it entirely), so pacing would starve the
            // guest and the audio worklet would underrun: run flat-out and let
            // the worklet's bounded queue (drop-oldest) keep audio continuous.
            let target = document.hidden ? Infinity : paceFrames + Math.floor((performance.now() - paceWall) / FRAME_MS);
            if (performance.now() - lastPump >= 60) {   // gap from the pump's burst
            while (steps < STEP_BUDGET && performance.now() - t0 < MS_BUDGET && frames < target) {
                // drive the guest clock BEFORE every step: DG_GetTicksMs reads
                // g_abi.clockMs, so I_GetTime advances even inside a single
                // doomgeneric_Tick (the melt wipe spins on I_GetTime).  The
                // clock is DERIVED FROM FRAMECOUNT (not wall time), so guest
                // time can never run ahead of the guest's own execution —
                // audio production stays locked to wall 11025 samples/s.
                clockMs = Math.max(clockMs, Math.floor(bootClock + frames * FRAME_MS) & 0xffffffff);
                emu.write32(CLOCKMS, clockMs);
                const res = emu.step();
                instTotal = res.instCount;   // cumulative counter
                steps++;
                frames = emu.read32(FRAMECOUNT);
            }
            }   // end pump-gap guard
            activeMs += performance.now() - stepT0;
        } catch (e) {
            setStatus('emulator error: ' + e.message, 'error');
            return;
        }
        appendUart(emu.drainUart());
        drainAudio();
        processSaves();          // save/load handshake (respond within a step burst)
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
