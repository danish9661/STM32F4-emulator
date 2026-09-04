// DOOM emulation worker: owns the emulator, the stepping loop, the guest ABI
// and the framebuffer->RGBA conversion. site/doom.js is the UI shell — it
// keeps the canvas, keyboard, AudioContext and localStorage, which a worker
// cannot touch, and does nothing per frame but putImageData.
//
// Why a worker at all: stepping the guest is a long synchronous WASM call, so
// on the main thread every burst blocked input, layout and paint. It costs no
// throughput (page-driven cadence, see below) but there is still exactly one
// emulation thread — this makes the page smooth, not the guest faster. SharedArrayBuffer is deliberately NOT used: it needs
// COOP/COEP headers GitHub Pages cannot set, and it would buy nothing here
// (the two WASM modules have separate linear memories regardless, and
// Unicorn's build is single-threaded — see AGENTS.md §7).
//
// Bump the ?v= on the Worker() URL in doom.js whenever this file changes:
// worker scripts are cached exactly as hard as module scripts, and a stale
// copy looks precisely like the bug you thought you just fixed.
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

// ── pacing (unchanged from the pre-worker main-thread loop) ──
// The guest's DG_SleepMs(15) is a no-op, and the guest advances its own frame
// counter (ABI +0x514) once per rendered frame at 35 tics/s of game time, so
// the driver paces by wall clock: run steps until the guest's frame count
// catches up to realtime 35 fps.
//
// Measured cost (2026-08-14, E1M1, low detail ON): ~918k guest instructions
// per frame, so a full 35 fps needs ~32 MIPS. The Unicorn WASM core tops out
// near 20-24 MIPS, so the page runs ~25 fps and degrades gracefully.
//
// Audio consequence: the guest mixer emits exactly one frame's worth of
// samples (11025/35 = 315) per RENDERED frame, so production scales with fps.
// audio-worklet.js handles that with rate control, NOT silence insertion. Do
// not "fix" it by making the guest mix more per frame: sample position would
// then advance faster than game logic and pitch-shift every sound.
const FRAME_MS = 1000 / 35;      // one game frame (tic) per 28.57ms realtime
const STEP_BUDGET = 32;          // max steps per burst
const STEP_INST = 60000;         // instructions per emu.step()
// Between bursts the loop must RETURN TO THE EVENT LOOP: stepping the guest
// gap-free wedges Chrome's TCI interpreter (AGENTS §7/§16). The old
// main-thread loop got that for free by living on rAF.
//
// A worker has no rAF, and setTimeout is clamped to ~4ms once nested, which
// is a straight duty-cycle tax: measured against the old build, a self-timed
// 28ms burst (87% duty) ran 12% fewer guest instructions and a 44ms burst
// (92%) ran ~6% fewer.
//
// A MessageChannel yield (~0.05ms, ~99% duty) was tried to close that gap and
// is NOT viable: over a 60s run it degraded to 760M instructions at MIPS 1.1,
// i.e. it hit exactly the stall a real gap avoids. Do not "optimize" it back.
// So the burst cadence is driven by the PAGE's rAF: doom.js posts a 'tick'
// every animation frame and the worker runs one burst per tick. That
// reproduces the old main-thread build's 16ms-per-16.7ms duty (96%) without
// putting the work on the main thread — its rAF callback only posts a
// message. Receiving that message is itself an event-loop turn, so the TCI
// still gets its gap.
//
// A self-timer remains as the fallback for when rAF is dead (hidden tab,
// which Chrome throttles to nothing). It runs a bigger burst because its
// gap is the clamped 4ms and there is no frame to pace to.
const YIELD_MS = 4;
const RAF_MS_BUDGET = 16;       // page-driven: one animation frame of work
const SELF_MS_BUDGET = 44;      // self-driven: amortize the 4ms clamp
const TICK_STALE_MS = 200;      // no tick for this long => rAF is dead
let timer = null;
let lastTickAt = 0;
function scheduleLoop(delay) {
    if (timer === null) timer = setTimeout(selfTick, delay === undefined ? YIELD_MS : delay);
}
// Fallback driver: only actually steps when the page's ticks have stopped.
function selfTick() {
    timer = null;
    if (!booted) return;
    if (performance.now() - lastTickAt > TICK_STALE_MS) {
        loop(SELF_MS_BUDGET);
        scheduleLoop(YIELD_MS);
    } else {
        scheduleLoop(TICK_STALE_MS);   // page is driving; just watch
    }
}

const ABI = 0x20002000n;
const KEYWR = ABI;                 // u32 write index (driver)
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
const DETAIL_ADDR = 0x20002530;

let emu = null, uc = null;
let keyWr = 0;
let fbAddr = 0n;
let paused = false, booted = false, hidden = false;
let lowDetail = true;
let instTotal = 0, framesShown = 0;
let statLast = 0, statInst = 0, statFrames = 0, activeMs = 0;
let paceWall = 0, paceFrames = -1;
let bootClock = 0, clockMs = 0;
// Pending load handshake: the guest busy-waits on SAVEREADY, so the answer
// from the main thread's localStorage has to come back before we step again.
let loadPending = false;

const held = new Set();          // doom keycodes currently held down
const sentPos = new Map();       // code -> ring byte position of its keydown
const upPending = new Map();     // code -> D ring position (see flushUpPending)

const post = (msg, transfer) => self.postMessage(msg, transfer || []);
const status = (text, cls) => post({ t: 'status', text, cls });

// Unicorn ships as a classic emscripten script that assigns a global, so it
// cannot be `import`ed. Fetch + indirect-eval it, and hand it an explicit
// locateFile: emscripten derives the .wasm path from document.currentScript,
// which does not exist here, and would otherwise look for the wasm next to
// this worker instead of in vendor/.
// SECURITY: vendor/unicorn_arm.js is a vendored, version-pinned artifact
// (alexaltea/unicorn.js 2.1.4). If you update it, bump ?v= and verify SRI:
//   openssl dgst -sha384 -binary vendor/unicorn_arm.js | openssl base64 -A
// and add <meta integrity> if served with CSP.
async function loadUnicorn() {
    const url = new URL('vendor/unicorn_arm.js?v=20', self.location.href);
    const src = await (await fetch(url)).text();
    const factory = (0, eval)(src + '\n;MUnicorn');
    return () => factory({
        locateFile: (p) => new URL('vendor/' + p, self.location.href).href,
    });
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

// The guest's I_GetEvent drains the key ring and BREAKS on the first keyup
// (engine/i_input.c), and the ring is only drained in bursts. If a (D,U) pair
// sits in the ring when a drain runs, gamekeydown[] is set and cleared within
// one tic and in-game turn/move keys never register (menus are unaffected,
// they act on single events). So a keyup is held back until the guest's read
// cursor proves it consumed the keydown AND the user has actually released
// the key; the U then lands in a LATER drain. Held keys need no re-assert:
// gamekeydown[] persists from the single keydown until the U.
function flushUpPending() {
    if (!upPending.size) return;
    const keyRd = emu.read32(KEYRD);
    for (const [code, dPos] of upPending) {
        if (held.has(code)) continue;
        if (keyRd !== dPos) {
            sendKey(code, false);
            upPending.delete(code);
        }
    }
}

// ── savegames: the blobs live in guest EXTRAM, localStorage lives on the
// main thread, so both directions are a message round-trip.
function processSaves() {
    if (!emu || loadPending) return;
    const flag = emu.read32(SAVEFLAG);
    if (flag === 1) {
        const slot = emu.read32(SAVESLOT);
        const size = emu.read32(SAVESIZE);
        if (slot >= 0 && slot < SAVE_SLOTS && size > 0) {
            const b = new Uint8Array(uc.mem_read(SAVEADDR + BigInt(slot * Number(SAVESLOTSIZE)), size));
            post({ t: 'save', slot, bytes: b }, [b.buffer]);
            emu.write32(SAVEMAP, emu.read32(SAVEMAP) | (1 << slot));
        }
        emu.write32(SAVEFLAG, 0);
    } else if (flag === 2) {
        // Ask the main thread for the blob and stop stepping until it lands:
        // the guest spins on SAVEREADY, so there is nothing to run meanwhile.
        loadPending = true;
        post({ t: 'loadReq', slot: emu.read32(SAVESLOT) });
    }
}

function completeLoad(slot, bytes) {
    if (!emu) return;
    if (bytes && bytes.length) {
        uc.mem_write(SAVEADDR + BigInt(slot * Number(SAVESLOTSIZE)), bytes);
        emu.write32(SAVESIZE, bytes.length);
    } else {
        emu.write32(SAVESIZE, 0);
    }
    emu.write32(SAVEREADY, 1);
    emu.write32(SAVEFLAG, 0);
    loadPending = false;
}

function fnv1a(data) {
    let h = 0x811c9dc5;
    for (let i = 0; i < data.length; i++) {
        h ^= data[i];
        h = Math.imul(h, 0x01000193);
    }
    return h >>> 0;
}

// Palette-expands the guest framebuffer and transfers the RGBA bytes to the
// main thread, which only has to putImageData them. Returns true if the
// framebuffer changed.
//
// Hash-only gating: the guest frame counter can stall (the level-start melt
// wipe spins on I_GetTime), so it must never gate the repaint.
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
    const rgba = new Uint8ClampedArray(320 * 200 * 4);
    for (let i = 0; i < 64000; i++) {
        const p = fb[i] * 4;
        rgba[i * 4] = pal[p + 2];      // BGRA -> RGBA
        rgba[i * 4 + 1] = pal[p + 1];
        rgba[i * 4 + 2] = pal[p];
        rgba[i * 4 + 3] = 255;
    }
    post({ t: 'frame', rgba }, [rgba.buffer]);
    return true;
}

function drainAudio() {
    const s = emu.takeSpeakerSamples();
    if (!s || !s.length) return;
    post({ t: 'audio', samples: s }, [s.buffer]);
}

function loop(msBudget) {
    if (!booted || !emu) return;
    const t0 = performance.now();
    if (!paused && !loadPending) {
        try {
            flushUpPending();   // keyups land on the NEXT burst, after the guest consumed the down
            let steps = 0;
            let frames = emu.read32(FRAMECOUNT);
            if (paceFrames < 0) {
                paceFrames = frames;     // re-anchor after a pause: no catch-up burst
                paceWall = t0;
            }
            // Realtime lock: the guest may never produce frames faster than
            // wall time. While the page is HIDDEN there is nothing to render
            // and no rAF pacing to respect, so run flat out and let the
            // worklet's bounded queue keep audio continuous.
            const target = hidden ? Infinity : paceFrames + Math.floor((t0 - paceWall) / FRAME_MS);
            while (steps < STEP_BUDGET && performance.now() - t0 < msBudget && frames < target) {
                // Drive the guest clock BEFORE every step: DG_GetTicksMs reads
                // g_abi.clockMs, so I_GetTime advances even inside a single
                // doomgeneric_Tick (the melt wipe spins on I_GetTime). The
                // clock is DERIVED FROM FRAMECOUNT, not wall time, so guest
                // time can never run ahead of the guest's own execution and
                // audio production stays locked to wall 11025 samples/s.
                clockMs = Math.max(clockMs, Math.floor(bootClock + frames * FRAME_MS) & 0xffffffff);
                emu.write32(CLOCKMS, clockMs);
                const res = emu.step();
                instTotal = res.instCount;
                steps++;
                frames = emu.read32(FRAMECOUNT);
            }
            activeMs += performance.now() - t0;
        } catch (e) {
            status('emulator error: ' + e.message, 'error');
            post({ t: 'error', message: String(e && e.message || e) });
            return;
        }
        const uart = emu.drainUart();
        if (uart) post({ t: 'uart', text: uart });
        drainAudio();
        processSaves();
        if (renderFb()) framesShown++;
        reportStats();
    } else {
        paceFrames = -1;    // re-anchor on resume
    }
}

function reportStats() {
    const now = performance.now();
    if (now - statLast < 500) return;
    const dt = (now - statLast) / 1000;
    const mips = activeMs > 0 ? (instTotal - statInst) / (activeMs / 1000) / 1e6 : 0;
    // FPS is the guest's OWN rendered-frame counter, not the count of frames
    // whose pixels changed: the old meter counted framebuffer changes and so
    // read ~0 whenever the view was static (menus, standing still), badly
    // understating the real rate. `drawn` keeps the change-gated repaint
    // observable as a second number.
    const guestFrames = Number(emu.read32(FRAMECOUNT));
    post({
        t: 'stats',
        mips,
        fps: (guestFrames - statFrames) / dt,
        drawn: framesShown / dt,
        instM: instTotal / 1e6,
        guestFrames,
    });
    statLast = now; statInst = instTotal; statFrames = guestFrames;
    activeMs = 0; framesShown = 0;
}

async function boot(msg) {
    status('booting…');
    paused = false;
    booted = false;
    keyWr = 0;
    held.clear(); sentPos.clear(); upPending.clear();
    loadPending = false;
    lastHash = -1;
    instTotal = 0; framesShown = 0; statInst = 0; statFrames = 0; activeMs = 0;
    if (timer !== null) { clearTimeout(timer); timer = null; }
    if (emu) { try { emu.close(); } catch (e) {} emu = null; }

    try {
        // Unicorn is opt-in only (msg.cpuBackend === 'unicorn'); the default
        // wasm backend never touches it, so skip the ~800KB fetch + compile.
        const unicorn = msg.cpuBackend === 'unicorn' ? await loadUnicorn() : null;
        emu = await createEmulator({
            firmware: msg.firmware,
            bindings,
            unicorn,
            cpu_backend: msg.cpuBackend === 'unicorn' ? 'unicorn' : 'wasm',
            cpu_backend: msg.cpuBackend || 'wasm',
            svdXml: msg.svdXml,
            extra_ram: [
                { addr: 0xC0000000, size: 16 * 1024 * 1024 },   // .data/.bss + zone + heap
                { addr: 0xB8000000, size: 8 * 1024 * 1024 },    // WAD image
            ],
            extra_mem: [{ addr: 0xB8000000, data: new Uint8Array(msg.wad) }],
            // noCountHook: no per-block JS callback at all (measured ~6%
            // faster than blockCounting, which crossed the WASM->JS boundary
            // on every basic block just to bump a counter). Safe here because
            // doom paces off the guest's FRAME counter, not instCount —
            // instCount then tracks the emu_start budget, which is what the
            // MIPS readout wants anyway (block counting over-reported ~1.39x).
            minimalPolls: true, noCountHook: true,
            maxBatch: STEP_INST,
            ext_devices: { speaker: true },   // enable the I2S capture drain
        });
        uc = emu.uc;
        lowDetail = msg.lowDetail;
        emu.write32(DETAIL_ADDR, lowDetail ? 1 : 0);
        emu.write32(SAVEMAP, msg.saveMap | 0);
        bootClock = performance.now();
        clockMs = 0;
        paceFrames = 0;              // realtime lock from boot
        paceWall = bootClock;
        fbAddr = 0n;
        statLast = performance.now();
        booted = true;
        status('loading WAD…');
        post({ t: 'booted' });
        scheduleLoop(TICK_STALE_MS);   // the page's ticks take over from here
    } catch (e) {
        status('boot failed: ' + e.message, 'error');
        post({ t: 'error', message: String(e && e.message || e) });
    }
}

self.onmessage = (e) => {
    const m = e.data;
    switch (m.t) {
        case 'boot':
            boot(m);
            break;
        case 'key':
            if (!booted) break;
            if (m.pressed) {
                if (!held.has(m.code)) {
                    sentPos.set(m.code, sendKey(m.code, true));
                    held.add(m.code);
                }
            } else if (held.delete(m.code)) {
                upPending.set(m.code, sentPos.get(m.code));
                sentPos.delete(m.code);
            }
            break;
        case 'pause':
            paused = m.paused;
            break;
        case 'detail':
            lowDetail = m.lowDetail;
            if (emu) emu.write32(DETAIL_ADDR, lowDetail ? 1 : 0);
            break;
        case 'tick':
            // One burst per page animation frame. The gap between bursts is
            // the message dispatch plus whatever is left of the frame — the
            // same ~0.7ms the old main-thread rAF loop ran with, which is a
            // real event-loop turn and is what the TCI needs. Do NOT add a
            // minimum-gap guard that SKIPS ticks: dropping a tick that queued
            // behind a burst costs a whole frame and measured 413M vs 720M
            // guest instructions.
            lastTickAt = performance.now();
            if (!booted) break;
            loop(RAF_MS_BUDGET);
            break;
        case 'hidden':
            hidden = m.hidden;
            break;
        case 'loadResp':
            completeLoad(m.slot, m.bytes);
            break;
        case 'close':
            booted = false;
            if (timer !== null) { clearTimeout(timer); timer = null; }
            if (emu) { try { emu.close(); } catch (err) {} emu = null; }
            break;
    }
};
