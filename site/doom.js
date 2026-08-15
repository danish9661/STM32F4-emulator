// DOOM browser demo — UI shell.
//
// The emulator itself lives in site/doom-worker.js. Stepping the guest is a
// long synchronous WASM call; running it here blocked input, layout and paint
// for the whole burst. This file now owns only what a worker cannot touch:
// the canvas, the keyboard, the AudioContext/worklet and localStorage. Per
// frame it does one putImageData of an RGBA buffer the worker transferred.
//
// It is a responsiveness win, not a throughput one — there is still one
// emulation thread, and the guest still runs at whatever the Unicorn WASM
// core manages (~20-24 MIPS, so ~25 of the target 35 fps).
//
// Serve from site/ (python3 -m http.server 8123 --directory site) — the page
// fetches the WAD + SVD + wasm at runtime (file:// won't work).
import { FIRMWARES } from './firmware.js';

const $ = (id) => document.getElementById(id);
const canvas = $('screen');
const ctx = canvas.getContext('2d');
const img = ctx.createImageData(320, 200);

// Low-detail render (guest ABI slot, 0=high 1=low): the engine renders every
// other column -> measured 1145k -> 918k guest instructions per frame (-20%),
// i.e. ~19 -> ~24 fps. Default ON for speed; toggled from the topbar.
//
// This MUST go through the ABI slot: the guest applies it via the engine's
// R_SetViewSize()+R_ExecuteSetViewSize(), the only place detailshift and the
// colfunc/spanfunc render pointers are recomputed. Writing the engine's
// `detailLevel` global directly is a NO-OP — measured bit-identical
// inst/frame with detailshift stuck at 0.
let lowDetail = localStorage.getItem('doomDetail') !== '0';
const detailToggle = $('detail');
detailToggle.checked = lowDetail;
detailToggle.addEventListener('change', () => {
    lowDetail = detailToggle.checked;
    localStorage.setItem('doomDetail', lowDetail ? '1' : '0');
    send({ t: 'detail', lowDetail });
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

const SAVE_SLOTS = 2;

let worker = null;
let paused = false;
let booted = false;
let uartBuf = '';
const held = new Set();          // doom keycodes currently held (for the label)

// Debug handles for the CDP smoke tests. window.__emu is gone — the emulator
// no longer lives on this thread — so anything that drove it directly must go
// through window.__doom instead.
window.__audioTotal = 0;
window.__audioNode = null;
window.__doom = {
    get booted() { return booted; },
    get paused() { return paused; },
    get stats() { return lastStats; },
    key(code, pressed) { send({ t: 'key', code, pressed }); },
    send: (m) => send(m),
};
let lastStats = null;

function send(msg, transfer) {
    if (worker) worker.postMessage(msg, transfer || []);
}

// ── audio: the worker hands us I2S samples, the worklet plays them ──
// The worklet (audio-worklet.js) resamples 11025 Hz -> context rate and plays
// continuously on the audio thread — no BufferSource scheduling jitter,
// latency bounded by its MAX_QUEUE drop guard. The guest is realtime-locked,
// so production is ~11025 samples/s wall; the worklet only underruns on slow
// machines.
let audioCtx = null, audioNode = null;

async function initAudio() {
    if (audioCtx) return;
    try {
        audioCtx = new (window.AudioContext || window.webkitAudioContext)();
        // NOTE: bump this ?v= whenever audio-worklet.js changes — the module
        // is cached hard, and a stale worklet is indistinguishable from an
        // audio bug (v20 = dynamic rate control, replacing silence+flush).
        await audioCtx.audioWorklet.addModule('audio-worklet.js?v=23');
        audioNode = new AudioWorkletNode(audioCtx, 'doom-audio');
        window.__audioNode = audioNode;
        audioNode.connect(audioCtx.destination);
        audioNode.port.onmessage = (e) => {
            // starved = output samples the worklet could not fill (the
            // audible crackle); rate = playback speed vs correct pitch.
            if (e.data && e.data.stat) window.__audioStat = e.data;
            // 'need' (queue running low) needs no action now: the worker
            // steps on its own timer and is not throttled by this thread's
            // rAF, so there is no starvation path left for it to rescue.
        };
    } catch (e) {
        audioCtx = null;
        audioNode = null;
    }
}

function setStatus(s, cls) {
    const el = $('status');
    el.textContent = s;
    el.className = cls || '';
}

function updateKeysLabel() {
    const names = [];
    for (const k of held) {
        names.push(Object.keys(DOM_TO_DOOM).find((d) => DOM_TO_DOOM[d] === k && d.length === 1) ||
                    (k === KEY.STRAFE_L ? 'Shift' : k === KEY.FIRE ? 'Ctrl' : k === KEY.USE ? 'Space' : 'k' + k.toString(16)));
    }
    $('keys').textContent = names.length ? 'held: ' + names.join(' ') : '';
}

// The keyup-ordering dance (holding a U back until the guest consumed the D)
// lives in the worker, next to the ring it writes. Here we only track what is
// held so the topbar label stays honest.
document.addEventListener('keydown', (e) => {
    let code = DOM_TO_DOOM[e.key];
    if (code === undefined && e.key.length === 1 && e.key >= 'a' && e.key <= 'z') {
        code = e.key.charCodeAt(0);    // raw ASCII letters reach the menu
    }                                  // string-entry + 'y' confirm prompts
    if (code === undefined) return;
    e.preventDefault();
    initAudio();                       // user gesture: unlock WebAudio
    if (!held.has(code)) {
        held.add(code);
        updateKeysLabel();
        send({ t: 'key', code, pressed: true });
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
        updateKeysLabel();
        send({ t: 'key', code, pressed: false });
    }
});
canvas.addEventListener('mousedown', () => {
    if (document.pointerLockElement !== canvas) {
        canvas.requestPointerLock();
        return;
    }
    initAudio();
    held.add(KEY.FIRE);
    updateKeysLabel();
    send({ t: 'key', code: KEY.FIRE, pressed: true });
});
document.addEventListener('mouseup', () => {
    if (held.delete(KEY.FIRE)) {
        updateKeysLabel();
        send({ t: 'key', code: KEY.FIRE, pressed: false });
    }
});
document.addEventListener('visibilitychange', () => {
    send({ t: 'hidden', hidden: document.hidden });
});

$('btnPause').addEventListener('click', () => {
    paused = !paused;
    $('btnPause').textContent = paused ? 'Resume' : 'Pause';
    send({ t: 'pause', paused });
});
$('btnReset').addEventListener('click', boot);
$('btnFull').addEventListener('click', () => {
    const el = document.fullscreenElement ? document : $('screenWrap');
    (document.fullscreenElement ? document.exitFullscreen() : el.requestFullscreen()).catch(() => {});
});
$('btnClear').addEventListener('click', () => { $('uart').textContent = uartBuf = ''; });

function appendUart(chunk) {
    if (!chunk || window.__noUart) return;
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
// Re-fit whenever the wrap itself changes size, not just on window resize.
// Calling fitCanvas() only at boot meant that if it measured before layout
// settled (fonts/log/guide still reflowing) the canvas stayed stuck at that
// wrong size forever — the page rendered a small game floating in a large
// black void. A ResizeObserver removes that whole class of race, and also
// handles fullscreen and the log/guide toggles for free.
if (typeof ResizeObserver !== 'undefined') {
    new ResizeObserver(() => fitCanvas()).observe($('screenWrap'));
}

// ── log / guide visibility: hiding them hands the space to the game ──
let logHidden = localStorage.getItem('doomHideLog') === '1';
function applyLogVisibility() {
    document.body.classList.toggle('nolog', logHidden);
    document.body.classList.toggle('noguide', logHidden);
    const b = $('btnLog');
    if (b) { b.textContent = logHidden ? 'Show log' : 'Hide log'; b.setAttribute('aria-pressed', String(logHidden)); }
    fitCanvas();
}
$('btnLog').addEventListener('click', () => {
    logHidden = !logHidden;
    localStorage.setItem('doomHideLog', logHidden ? '1' : '0');
    applyLogVisibility();
});
applyLogVisibility();

function onWorkerMessage(e) {
    const m = e.data;
    switch (m.t) {
        case 'frame':
            img.data.set(m.rgba);
            ctx.putImageData(img, 0, 0);
            break;
        case 'uart':
            appendUart(m.text);
            break;
        case 'audio':
            window.__audioTotal += m.samples.length;
            if (window.__noDrain) break;
            if (audioNode) audioNode.port.postMessage(m.samples, [m.samples.buffer]);
            break;
        case 'status':
            setStatus(m.text, m.cls);
            break;
        case 'booted':
            booted = true;
            fitCanvas();
            break;
        case 'stats':
            lastStats = m;
            // Keep the status honest: rendering frames = actually playing.
            // Also surface the two states a user would otherwise misread as a
            // hang — the pre-first-frame boot, and audio still locked by the
            // browser's autoplay policy until the first keypress.
            if (m.guestFrames === 0) {
                setStatus('booting — loading WAD…');
            } else if (!audioCtx || audioCtx.state !== 'running') {
                setStatus('running — press any key to enable sound', 'warn');
            } else {
                setStatus('running');
            }
            // Audio health: playback rate vs correct pitch (1.00 = in tune).
            // Below 1.0 means the guest is producing audio slower than
            // realtime and the worklet is stretching to stay continuous.
            const a = window.__audioStat;
            const audioTxt = a && a.rate ? ` · audio ${a.rate.toFixed(2)}x` : '';
            $('stats').textContent =
                `MIPS: ${m.mips.toFixed(1)} · FPS: ${m.fps.toFixed(0)}/35 · drawn ${m.drawn.toFixed(0)}${audioTxt} · ${m.instM.toFixed(1)}M inst`;
            break;
        case 'save':
            try {
                let s = '';
                for (let i = 0; i < m.bytes.length; i += 0x8000) {
                    s += String.fromCharCode.apply(null, m.bytes.subarray(i, i + 0x8000));
                }
                localStorage.setItem('doom-save-' + m.slot, btoa(s));
            } catch (err) { /* quota */ }
            break;
        case 'loadReq': {
            // The guest busy-waits on this, so answer immediately.
            const raw = (m.slot >= 0 && m.slot < SAVE_SLOTS)
                ? localStorage.getItem('doom-save-' + m.slot) : null;
            let bytes = null;
            if (raw) {
                const bin = atob(raw);
                bytes = new Uint8Array(bin.length);
                for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
            }
            send({ t: 'loadResp', slot: m.slot, bytes }, bytes ? [bytes.buffer] : []);
            break;
        }
        case 'error':
            booted = false;
            console.error('doom worker:', m.message);
            break;
    }
}

async function boot() {
    setStatus('booting…');
    fitCanvas();
    paused = false;
    booted = false;
    $('btnPause').textContent = 'Pause';
    held.clear();
    updateKeysLabel();
    uartBuf = '';
    $('uart').textContent = '';

    if (worker) { worker.terminate(); worker = null; }

    try {
        const [svdXml, wad] = await Promise.all([
            fetch('vendor/stm32f407.svd').then((r) => r.text()),
            fetch('doom1.wad').then((r) => r.arrayBuffer()),
        ]);
        const firmware = new Uint8Array(atob(FIRMWARES.doom.bytes).split('').map((c) => c.charCodeAt(0)));

        // Restore the save slot map (which slots have a saved game) so the
        // load menu shows exactly the localStorage-backed slots.
        let saveMap = 0;
        for (let i = 0; i < SAVE_SLOTS; i++) {
            if (localStorage.getItem('doom-save-' + i)) saveMap |= 1 << i;
        }

        // Bump ?v= on every doom-worker.js edit — worker scripts cache as hard
        // as module scripts, and a stale copy looks exactly like a bug.
        worker = new Worker('doom-worker.js?v=6', { type: 'module' });
        worker.onmessage = onWorkerMessage;
        worker.onerror = (e) => {
            setStatus('worker failed: ' + (e.message || 'load error'), 'error');
            console.error(e);
        };
        send({ t: 'boot', svdXml, wad, firmware, lowDetail, saveMap }, [wad]);
        send({ t: 'hidden', hidden: document.hidden });
    } catch (e) {
        setStatus('boot failed: ' + e.message, 'error');
        console.error(e);
    }
}

boot();
