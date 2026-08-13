// Node harness for the doomgeneric F407 port.
// Boots the engine, waits for the title screen, then injects a New Game
// (Y + Enter + skill key) via the SRAM key ring and checks the framebuffer
// changes across frames, that the guest consumes keys, and that the run
// completes without a TCI/guest crash.
// Usage: node site/test_doom.mjs  (exit 0 = PASS)
import { readFileSync } from 'fs';
import { createRequire } from 'module';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const firmware = new Uint8Array(readFileSync(new URL('../doom/doom.bin', import.meta.url)));
const wad = readFileSync('/tmp/opencode/wad/doom1.wad');

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

const emu = await createEmulator({
    firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
    extra_ram: [
        { addr: 0xC0000000, size: 16 * 1024 * 1024 },   // .data/.bss + zone + heap
        { addr: 0xB8000000, size: 8 * 1024 * 1024 },    // WAD image
    ],
    extra_mem: [{ addr: 0xB8000000, data: wad }],
    minimalPolls: true, blockCounting: true,
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

const uart = [];
let uartText = '';
let fbAddr = 0n;
let prevHash = -1, changes = 0;
let phase = 'boot';      // boot -> title -> wait1/2/3 (change-gated keys) -> play
let gate = 0;            // change-count snapshot between key sends
let maxSteps = 0;
let crashed = false;
let audioPeak = 0, audioSamples = 0;   // drained incrementally (ring windows)

try {
    for (let i = 0; i < 400; i++) {
        emu.step(200000);
        const chunk = emu.drainUart();
        uart.push(chunk);
        uartText += chunk;
        maxSteps += 200000;
        if (maxSteps > 80000000) break;

        const a = emu.takeSpeakerSamples();
        audioSamples += a.length;
        for (let j = 0; j < a.length; j++) { const v = Math.abs(a[j]); if (v > audioPeak) audioPeak = v; }

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
            sendKey(KEY_W, true);               // held forward
            if (i % 10 === 0) {                  // and turn occasionally
                sendKey(0xAC, true); sendKey(0xAC, false);   // KEY_LEFTARROW
            }
            if (i % 5 === 0) {                   // and fire: guarantees loud audio
                sendKey(0xA3, true); sendKey(0xA3, false);   // KEY_FIRE
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
for (let i = 0; i < audio.length; i++) { const v = Math.abs(audio[i]); if (v > audioPeak) audioPeak = v; }
console.log();
console.log('uart tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-8).join(' | '));
console.log(`fbAddr=0x${fbAddr.toString(16)} frame_changes=${changes} keyRd=${keyRd} inst=${maxSteps} crashed=${crashed} phase=${phase} menuActive=${read32(0xC00166F8)} gamestate=${read32(0xC00153AC)}`);
console.log(`audio: ${audioSamples} samples drained, peak amplitude ${audioPeak.toFixed(0)}`);

const pass =
    !crashed &&
    all.includes('Z_Init') &&
    all.includes('adding doom1.wad') &&
    all.includes('I_InitGraphics') &&
    fbAddr !== 0n &&
    changes >= 20 &&
    keyRd > 0 &&          // guest consumed at least one injected event
    palOk &&              // guest exported the palette
    audioSamples > 0 &&   // I2S mixer is streaming audio frames
    audioPeak > 0.005 &&  // and the weapon/menu sounds actually produced signal
    read32(0xC00166F8) === 0 &&   // menu closed = game actually started
    phase === 'play';
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
