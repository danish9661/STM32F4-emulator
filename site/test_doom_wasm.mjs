// Node harness for the doomgeneric F407 port on the wasm CPU backend.
// DOOM boot -> title -> menu -> New Game ->
// E1M1 play with W + turns, quick-save, framebuffer/audio/save assertions.
// Usage: node site/test_doom_wasm.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../firmware/doom/doom.bin', import.meta.url)));
let wad;
try { wad = readFileSync('/tmp/opencode/wad/doom1.wad'); }
catch { wad = readFileSync(new URL('./doom1.wad', import.meta.url)); }

// Doom keycodes (engine/doomkeys.h; TranslateKey is identity)
const KEY_Y = 0x79;
const KEY_ENTER = 0x0D;
const KEY_3 = 0x33;
const KEY_W = 0x77;

// Emulator-visible ABI (doom/f407/doomplatform.h)
const ABASE = 0x20002000n;
const KEYWR = ABASE;                 // u32 write index (JS side)
const KEYRD = ABASE + 0x04n;         // u32 read index (guest side)
const RING = ABASE + 0x08n;          // 256-byte ring, 2 bytes/event
const DGSB = ABASE + 0x510n;         // u32 DG_ScreenBuffer value
const PALETTE = ABASE + 0x110n;      // 1024 B (b,g,r,a per entry)
// Savegame ABI: guest stages blobs in EXTRAM, driver mirrors them.
// SAVEFLAG 1 = guest wrote a slot (driver stores + sets SAVEMAP bit);
// SAVEFLAG 2 = guest wants to load a slot (driver restores + SAVEREADY).
const SAVEFLAG = ABASE + 0x51Cn;
const SAVESIZE = ABASE + 0x520n;
const SAVEREADY = ABASE + 0x524n;
const SAVESLOT = ABASE + 0x528n;
const SAVEMAP = ABASE + 0x52Cn;
const SAVEADDR = 0xC0080000n;
const SAVESLOTSIZE = 0x40000n;

const emu = await createEmulator({
    firmware, bindings, svdXml, wasmInit: wasmBytes,
    extra_ram: [
        { addr: 0xC0000000, size: 16 * 1024 * 1024 },   // .data/.bss + zone + heap
        { addr: 0xB8000000, size: 8 * 1024 * 1024 },    // WAD image
    ],
    extra_mem: [{ addr: 0xB8000000, data: wad }],
    ext_devices: { speaker: true },   // I2S capture drain (audio test)
});

const uc = emu.uc;
function read32(addr) { return emu.read32(addr); }
function memRead(addr, len) { return new Uint8Array(uc.mem_read(addr, len)); }

let keyWr = 0;
function sendKey(code, pressed) {
    const off = keyWr % 256;
    uc.mem_write(RING + BigInt(off), Uint8Array.of(code));
    uc.mem_write(RING + BigInt((off + 1) % 256), Uint8Array.of(pressed ? 0x80 : 0x00));
    keyWr = (keyWr + 2) % 256;
    emu.write32(KEYWR, keyWr);
}

// Savegame mirror (doom-worker.js processSaves/completeLoad parity, inline:
// the guest busy-waits on SAVEREADY, so the answer must land between steps
// — never mid-step). Returns true when a load completed this call.
const savedSlots = new Map(); // slot -> Uint8Array blob
function processSaves() {
    const flag = read32(SAVEFLAG);
    if (flag === 1) {
        const slot = read32(SAVESLOT), size = read32(SAVESIZE);
        if (slot >= 0 && slot < 2 && size > 0) {
            savedSlots.set(slot, memRead(SAVEADDR + BigInt(slot) * SAVESLOTSIZE, size));
            emu.write32(SAVEMAP, read32(SAVEMAP) | (1 << slot));
        }
        emu.write32(SAVEFLAG, 0);
    } else if (flag === 2) {
        const slot = read32(SAVESLOT);
        const blob = savedSlots.get(slot);
        if (blob && blob.length) {
            uc.mem_write(SAVEADDR + BigInt(slot) * SAVESLOTSIZE, blob);
            emu.write32(SAVESIZE, blob.length);
        } else {
            emu.write32(SAVESIZE, 0);
        }
        emu.write32(SAVEREADY, 1);
        emu.write32(SAVEFLAG, 0);
        return true;
    }
    return false;
}

const uart = [];
let uartText = '';
let fbAddr = 0n;
let prevHash = -1, changes = 0;
let phase = 'boot';      // boot -> title -> wait1/2/3 (change-gated keys) -> play
let saveTapped = false;  // F6 quick-save menu opened once mid-game
let saveSlotKey = false, saveNameKey = false;
let loadTapped = false;  // F9 quick-load after the save committed
let loadConfirmKey = false;
let loadDone = false;    // LOAD ok observed (the reverse handshake)
let gate = 0;            // change-count snapshot between key sends
let maxSteps = 0;
let crashed = false;
// Samples are floats in -1..1 (emulator.js normalizes the I2S u16 capture).
// `audioClipped` guards the mixer's gain staging: the scale constant in
// doom/f407/i_sound_f407.c has been wrong in BOTH directions historically
// (8.47x too quiet, then 8.47x too loud — which hard-clipped ~45% of all
// nonzero samples), and neither shows up in a "did any sample arrive" check.
let audioPeak = 0, audioSamples = 0;   // drained incrementally (ring windows)
let audioNonzero = 0, audioClipped = 0;
const tallyAudio = (a) => {
    for (let i = 0; i < a.length; i++) {
        const v = Math.abs(a[i]);
        if (v > audioPeak) audioPeak = v;
        if (a[i] !== 0) audioNonzero++;
        if (v >= 0.999) audioClipped++;
    }
};

try {
    for (let i = 0; i < 400; i++) {
        emu.step(200000);
        // Save/load handshake first (mirrors doom-worker.js: the guest
        // busy-waits on SAVEREADY, so the answer lands between steps).
        // Check BEFORE draining UART so the LOAD ok line is attributed to
        // the right iteration.
        if (processSaves()) console.log('[save] load handshake completed');
        const chunk = emu.drainUart();
        uart.push(chunk);
        uartText += chunk;
        maxSteps += 200000;
        if (maxSteps > 80000000 && !loadTapped) break;
        if (loadDone && maxSteps > 80000000) break;

        const a = emu.takeSpeakerSamples();
        audioSamples += a.length;
        tallyAudio(a);

        const sb = read32(DGSB);
        if (sb && BigInt(sb) !== fbAddr) { fbAddr = BigInt(sb); console.log(`[fb] DG_ScreenBuffer = 0x${sb.toString(16)}`); }

        const frame = fbAddr ? memRead(fbAddr, 320 * 200) : null;
        if (frame) {
            let h = 0;
            for (let j = 0; j < frame.length; j += 997) h = (h * 31 + frame[j]) | 0;
            if (h !== prevHash) { prevHash = h; changes++; }
        }

        if (phase === 'boot' && uartText.includes('I_InitGraphics')) {
            phase = 'title';
            changes = 0;          // count only post-boot frames
            console.log('[boot] title screen');
        } else if (phase === 'title' && changes >= 2) {
            phase = 'waitMenu';
            console.log('[keys] Enter (open menu)');
            sendKey(KEY_ENTER, true); sendKey(KEY_ENTER, false);
        } else if (phase === 'waitMenu' && read32(0xC00166F8) === 1) {
            // main menu is up (Enter #1 opened it).  doom1.wad = retail so
            // the flow is: New Game -> EPISODE select -> skill menu ->
            // Down x2 (skill 3) -> Enter.  The skill menu selects by cursor,
            // not number keys.  The guest consumes one key-pair per frame
            // (key-UP breaks the drain), so pace sends ~5 batches apart.
            phase = 'keys';
            console.log('[keys] Enter, Enter, Down, Down, Enter (new game, ep1, skill 3)');
            sendKey(KEY_ENTER, true); sendKey(KEY_ENTER, false);
            gate = 0;
        } else if (phase === 'keys') {
            gate++;
            if (gate === 5) {
                sendKey(KEY_ENTER, true); sendKey(KEY_ENTER, false);   // New Game
            } else if (gate === 10) {
                sendKey(KEY_ENTER, true); sendKey(KEY_ENTER, false);   // episode 1
            } else if (gate === 15) {
                sendKey(0xAF, true); sendKey(0xAF, false);             // KEY_DOWNARROW
            } else if (gate === 20) {
                sendKey(0xAF, true); sendKey(0xAF, false);
            } else if (gate === 25) {
                sendKey(KEY_ENTER, true); sendKey(KEY_ENTER, false);   // skill 3 -> start
                phase = 'play';
                console.log('[keys] in game (W + turn)');
            }
        } else if (phase === 'play') {
            if (i % 25 === 0) sendKey(KEY_W, true);          // re-assert held W (sparse:
            if (i % 40 === 0) {                              // the guest drains ~1 pair
                sendKey(0xAC, true); sendKey(0xAC, false);   // per frame; spamming over-
                sendKey(0xA3, true); sendKey(0xA3, false);   // flows the 256B ring)
            }
            if (i >= 200 && !saveTapped) {        // quick-save (F6): menu -> Enter
                saveTapped = true;                // -> name char -> Enter
                sendKey(0xC0, true); sendKey(0xC0, false);
                console.log('[keys] F6 quick-save');
            }
            if (saveTapped && !saveSlotKey && read32(0xC00166F8) === 1) {
                saveSlotKey = true;
                sendKey(0x0D, true); sendKey(0x0D, false);   // Enter: select slot 0
                console.log('[keys] slot 0 (Enter)');
            }
            if (saveSlotKey && !saveNameKey) {
                saveNameKey = true;
                sendKey(0x61, true); sendKey(0x61, false);   // name 'a' (-> 'A')
                sendKey(0x0D, true); sendKey(0x0D, false);   // Enter -> M_DoSave(0)
                console.log('[keys] name+enter');
            }
            // LOAD handshake (reverse direction): after the save committed
            // (SAVEMAP bit 0 set by processSaves), press F9 quick-load.
            // NOTE: no 'y' confirm gate here — the confirm prompt prints
            // via the menu drawer (framebuffer), NOT UART, so gating on
            // UART text waits forever (observed: F9 sent, prompt up, 'y'
            // never sent). Instead hold 'y' down from the start: the
            // M_QuickLoadResponse consumes it when the prompt runs. The
            // key ring is drained pair-per-frame, so re-assert sparingly.
            if (saveNameKey && !loadTapped && (read32(SAVEMAP) & 1) !== 0) {
                loadTapped = true;
                sendKey(0xC3, true); sendKey(0xC3, false);   // F9 quick-load
                sendKey(0x79, true); sendKey(0x79, false);   // 'y' confirm (held via re-assert below)
                loadConfirmKey = true;
                console.log('[keys] F9 quick-load + y');
            }
            if (loadConfirmKey && !loadDone) {
                if (uartText.includes('LOAD ok slot=0')) {
                    loadDone = true;
                    console.log('[save] LOAD ok observed');
                } else if (i % 10 === 0) {
                    // Re-assert 'y' until the guest drains it (menu tick
                    // consumes one pair per frame; a single tap can land
                    // in a drain the prompt hasn't run yet).
                    sendKey(0x79, true); sendKey(0x79, false);
                }
            }
        }

        if (i % 50 === 0 && phase !== 'boot') {
            console.log(`[trace] ${i * 200000} inst phase=${phase} changes=${changes} keyRd=${read32(KEYRD)} pc=${emu.getRegisters().PC.toString(16)}`);
        }
        if (changes >= 300 && phase === 'play') break;    }
} catch (e) {
    crashed = true;
    console.log('[CRASH pc=' + emu.getRegisters().PC.toString(16) + '] ' + String(e).split('\n')[0]);
}

const all = uart.join('');
const keyRd = read32(KEYRD);
const pal = fbAddr ? memRead(PALETTE, 256 * 4) : null;
const palOk = pal && pal[3] + pal[4] + pal[5] + pal[7] > 0;   // non-zero entries exist
const audio = emu.takeSpeakerSamples();   // final drain
audioSamples += audio.length;
tallyAudio(audio);
const clipPct = audioNonzero ? (100 * audioClipped / audioNonzero) : 0;
console.log();
console.log('uart tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | '));
console.log(`fbAddr=0x${fbAddr.toString(16)} frame_changes=${changes} keyRd=${keyRd} inst=${maxSteps} crashed=${crashed} phase=${phase} menuActive=${read32(0xC00166F8)} gamestate=${read32(0xC00153AC)}`);
console.log(`audio: ${audioSamples} samples drained, peak ${audioPeak.toFixed(3)} (of 1.0), nonzero ${audioNonzero}, clipped ${audioClipped} (${clipPct.toFixed(1)}% of nonzero)`);
console.log(`save: flag=${read32(0x2000251C)} size=${read32(0x20002520)} ready=${read32(0x20002524)} slot=${read32(0x20002528)} map=${read32(0x2000252C)} sendsave=${read32(0xC0015848)} sse=${read32(0xC00166FC)} saveSlot=${read32(0xC0016700)} qss=${read32(0xC0016818)}`);

const pass =
    !crashed &&
    all.includes('Z_Init') &&
    all.includes('adding doom1.wad') &&
    all.includes('I_InitGraphics') &&
    all.includes('SAVE ok slot=0') &&   // F6 quick-save committed via the ABI
    all.includes('LOAD ok slot=0') &&   // F9 quick-load restored via the ABI
    loadDone &&                          // handshake observed live, not just in UART
    fbAddr !== 0n &&
    changes >= 20 &&
    keyRd > 0 &&          // guest consumed at least one injected event
    palOk &&              // guest exported the palette
    audioSamples > 0 &&   // I2S mixer is streaming audio frames
    audioPeak > 0.005 &&  // and the weapon/menu sounds actually produced signal
    clipPct < 5 &&        // mixer gain staging sane (was ~45% when scaled 8.47x too hot)
    read32(0xC00166F8) === 0 &&   // menu closed = game actually started
    phase === 'play';
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
