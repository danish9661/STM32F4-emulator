#!/usr/bin/env node
// ws-bridge.mjs — WebSocket bridge for the STM32F407 emulator.
//
// Runs the emulator headlessly in Node and exposes it over a binary
// WebSocket protocol so the browser demo (remote-emu.js) can drive it
// with zero WASM on the client side.  The browser is a thin UI; all
// execution stays in Node.
//
// Usage:
//   node site/ws-bridge.mjs [--port 8234] [--firmware path] [--verbose]
//                            [--lowpower] [--inst N]
//
// Protocol (binary WebSocket, little-endian):
//
//   Browser → Node (requests):
//     0x01 STEP        [id:u32] [max_inst:u32]
//     0x02 STOP        (no id — fire-and-forget)
//     0x03 RESET       (no id)
//     0x04 LOAD_IMAGE  [id:u32] [flash_len:u32 LE] [flash bytes…]
//     0x10 READ32      [id:u32] [addr:u32]
//     0x11 WRITE32     [id:u32] [addr:u32] [value:u32]
//     0x12 GET_REGS    [id:u32]
//     0x20 ETH_RX      [len:u32] [frame bytes…]
//     0x21 CAN_RX      [id:u16] [dlc:u8] [8 bytes data]
//     0x22 UART_TX     [len:u16] [bytes…]
//     0x30 SPI_MISO    [periph_len:u8] [periph str] [len:u16] [bytes]
//     0x31 I2C_RX      [periph_len:u8] [periph str] [len:u16] [bytes]
//     0x40 SET_INPUT   [pin_len:u8] [pin str] [level:u8]
//
//   Node → Browser (responses + pushes):
//     0x80 UART_TX     [len:u16] [bytes…]
//     0x81 ETH_TX      [len:u32] [frame bytes…]
//     0x82 GPIO_STATE  [bank:u8] [idr:u32] [odr:u32] [moder:u32]
//     0x8A STOPPED     (unsolicited)
//     0x90 STEP_RESP   [id:u32] [inst_count:u32] [stopped:u8]
//     0x91 READ32_RESP [id:u32] [value:u32]
//     0x92 WRITE32_OK  [id:u32]
//     0x93 LOAD_OK     [id:u32]
//     0x94 REGS_RESP   [id:u32] [36 × u32 LE]
//     0xA0 ERROR       [id:u32] [len:u16] [msg bytes…]

import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import { WebSocketServer } from 'ws';
import * as bindings from './vendor/stm32_periph_wasm.js';
import { createEmulator } from './emulator.js';
import { parseIntelHex, parseElf } from './loaders.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./vendor/unicorn_arm.cjs');
const __dirname = dirname(fileURLToPath(import.meta.url));
const svdXml = readFileSync(resolve(__dirname, 'vendor/stm32f407.svd'), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(resolve(__dirname, 'vendor/stm32_periph_wasm_bg.wasm')));

// ── CLI args ────────────────────────────────────────────────────────────────
const args = process.argv.slice(2);
let port = 8234;
let firmwarePath = null;
let verbose = false;
let lowpower = false;
let maxInst = 0; // 0 = run until client stops
for (let i = 0; i < args.length; i++) {
    if (args[i] === '--port') port = Number(args[++i]) || 8234;
    else if (args[i] === '--firmware' || args[i] === '-f') firmwarePath = args[++i];
    else if (args[i] === '--verbose' || args[i] === '-v') verbose = true;
    else if (args[i] === '--lowpower' || args[i] === '-l') lowpower = true;
    else if (args[i] === '--inst' || args[i] === '-n') maxInst = Number(args[++i]) || 0;
    else if (!firmwarePath && !args[i].startsWith('-')) firmwarePath = args[i];
}

// ── firmware loading helpers ────────────────────────────────────────────────
function detectFormat(buf) {
    if (buf[0] === 0x7F && buf[1] === 0x45 && buf[2] === 0x4C && buf[3] === 0x46) return 'elf';
    let i = 0;
    while (i < buf.length && (buf[i] === 0x20 || buf[i] === 0x09 || buf[i] === 0x0D || buf[i] === 0x0A)) i++;
    if (buf[i] === 0x3A) return 'hex';
    return 'bin';
}

function loadFirmware(path) {
    const raw = new Uint8Array(readFileSync(path));
    const fmt = detectFormat(raw);
    let firmware, extra_mem = [];
    if (fmt === 'elf') {
        const elf = parseElf(raw);
        firmware = elf.flash;
        extra_mem = elf.extraMem || [];
    } else if (fmt === 'hex') {
        const hex = parseIntelHex(Buffer.from(raw).toString('latin1'));
        firmware = hex.flash;
    } else {
        firmware = raw;
    }
    return { firmware, extra_mem, fmt };
}

// ── message encoding helpers ────────────────────────────────────────────────
const MSG = {
    // Browser → Node
    STEP: 0x01, STOP: 0x02, RESET: 0x03, LOAD_IMAGE: 0x04,
    READ32: 0x10, WRITE32: 0x11, GET_REGS: 0x12,
    ETH_RX: 0x20, CAN_RX: 0x21, UART_TX: 0x22,
    SPI_MISO: 0x30, I2C_RX: 0x31, SET_INPUT: 0x40,
    // Node → Browser
    PUSH_UART: 0x80, PUSH_ETH: 0x81, PUSH_GPIO: 0x82,
    STOPPED: 0x8A,
    STEP_RESP: 0x90, READ32_RESP: 0x91, WRITE32_OK: 0x92,
    LOAD_OK: 0x93, REGS_RESP: 0x94, ERROR: 0xA0,
};

function u8(v) { return v & 0xFF; }
function u16LE(buf, off) { return buf[off] | (buf[off + 1] << 8); }
function u32LE(buf, off) { return buf[off] | (buf[off + 1] << 8) | (buf[off + 2] << 16) | (buf[off + 3] << 24); }
function s16LE(buf, off) { const v = u16LE(buf, off); return v > 0x7FFF ? v - 0x10000 : v; }

function packU32LE(v) {
    return new Uint8Array([v & 0xFF, (v >>> 8) & 0xFF, (v >>> 16) & 0xFF, (v >>> 24) & 0xFF]);
}

function encodeResp(type, id, ...payloads) {
    const parts = [new Uint8Array([type]), packU32LE(id), ...payloads];
    const len = parts.reduce((s, p) => s + p.length, 0);
    const out = new Uint8Array(len);
    let off = 0;
    for (const p of parts) { out.set(p, off); off += p.length; }
    return out;
}

function encodeError(id, msg) {
    const enc = new TextEncoder().encode(msg);
    return encodeResp(MSG.ERROR, id, new Uint8Array([enc.length & 0xFF, (enc.length >> 8) & 0xFF]), enc);
}

// ── UART drain → push to browser ───────────────────────────────────────────
let uartDrainTimer = null;
function scheduleUartDrain(ws, emu) {
    if (uartDrainTimer) return;
    uartDrainTimer = setTimeout(() => {
        uartDrainTimer = null;
        if (!emu || ws.readyState !== 1) return;
        const u = emu.drainUart();
        if (u && u.length) {
            const enc = new TextEncoder().encode(u);
            const msg = new Uint8Array(3 + enc.length);
            msg[0] = MSG.PUSH_UART;
            msg[1] = enc.length & 0xFF;
            msg[2] = (enc.length >> 8) & 0xFF;
            msg.set(enc, 3);
            try { ws.send(msg); } catch {}
        }
    }, 5);
}

// ── per-connection state ───────────────────────────────────────────────────
async function handleConnection(ws) {
    let emu = null;
    let stepping = false;
    let drainTimer = null;
    let gpioTimer = null;
    let firmwareLoaded = false;

    const cleanup = () => {
        if (uartDrainTimer) { clearTimeout(uartDrainTimer); uartDrainTimer = null; }
        if (drainTimer) { clearInterval(drainTimer); drainTimer = null; }
        if (gpioTimer) { clearInterval(gpioTimer); gpioTimer = null; }
        if (emu) { try { emu.close(); } catch {} emu = null; }
    };

    ws.on('close', cleanup);
    ws.on('error', cleanup);

    // If a firmware was specified on the CLI, load it immediately.
    if (firmwarePath) {
        try {
            const { firmware, extra_mem } = loadFirmware(firmwarePath);
            emu = await createEmulator({
                firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
                extra_mem, verbose, lowpower,
                onTx: (pkt) => {
                    if (ws.readyState !== 1) return;
                    const msg = new Uint8Array(5 + pkt.length);
                    msg[0] = MSG.PUSH_ETH;
                    new DataView(msg.buffer).setUint32(1, pkt.length, true);
                    msg.set(pkt, 5);
                    try { ws.send(msg); } catch {}
                },
            });
            firmwareLoaded = true;
            startPeriodicDrain();
        } catch (e) {
            console.error('ws-bridge: failed to load firmware:', e.message);
        }
    }

    function startPeriodicDrain() {
        // Drain UART every 16ms (≈1 rAF) and push to browser.
        drainTimer = setInterval(() => {
            if (!emu || ws.readyState !== 1) return;
            const u = emu.drainUart();
            if (u && u.length) {
                const enc = new TextEncoder().encode(u);
                const msg = new Uint8Array(3 + enc.length);
                msg[0] = MSG.PUSH_UART;
                msg[1] = enc.length & 0xFF;
                msg[2] = (enc.length >> 8) & 0xFF;
                msg.set(enc, 3);
                try { ws.send(msg); } catch {}
            }
        }, 16);
        // GPIO state push every 250ms (lightweight — 5 reads).
        gpioTimer = setInterval(() => {
            if (!emu || ws.readyState !== 1) return;
            for (let bank = 0; bank < 5; bank++) {
                const base = 0x40020000 + bank * 0x400;
                try {
                    const moder = emu.read32(base);
                    const idr = emu.read32(base + 0x10);
                    const odr = emu.read32(base + 0x14);
                    const msg = new Uint8Array(14);
                    msg[0] = MSG.PUSH_GPIO;
                    msg[1] = bank;
                    new DataView(msg.buffer).setUint32(2, idr, true);
                    new DataView(msg.buffer).setUint32(6, odr, true);
                    new DataView(msg.buffer).setUint32(10, moder, true);
                    try { ws.send(msg); } catch {}
                } catch {}
            }
        }, 250);
    }

    ws.on('message', async (data) => {
        const buf = Buffer.isBuffer(data) ? data : Buffer.from(data);
        if (buf.length < 1) return;
        const type = buf[0];

        try {
            switch (type) {
                case MSG.STEP: {
                    if (!emu) { ws.send(encodeError(u32LE(buf, 1), 'no firmware loaded')); break; }
                    const id = u32LE(buf, 1);
                    const maxInst = buf.length >= 9 ? u32LE(buf, 5) : 100000;
                    const r = emu.step(maxInst);
                    // Send STEP_RESP
                    const resp = new Uint8Array(10);
                    resp[0] = MSG.STEP_RESP;
                    new DataView(resp.buffer).setUint32(1, id, true);
                    new DataView(resp.buffer).setUint32(5, r.instCount, true);
                    resp[9] = r.stopped ? 1 : 0;
                    try { ws.send(resp); } catch {}
                    // Drain any pending UART
                    const u = emu.drainUart();
                    if (u && u.length) {
                        const enc = new TextEncoder().encode(u);
                        const msg = new Uint8Array(3 + enc.length);
                        msg[0] = MSG.PUSH_UART;
                        msg[1] = enc.length & 0xFF;
                        msg[2] = (enc.length >> 8) & 0xFF;
                        msg.set(enc, 3);
                        try { ws.send(msg); } catch {}
                    }
                    if (r.stopped) {
                        try { ws.send(new Uint8Array([MSG.STOPPED])); } catch {}
                    }
                    break;
                }
                case MSG.STOP: {
                    if (emu) emu.stop();
                    break;
                }
                case MSG.RESET: {
                    if (emu) emu.reset();
                    break;
                }
                case MSG.LOAD_IMAGE: {
                    const id = u32LE(buf, 1);
                    const flashLen = u32LE(buf, 5);
                    const flash = new Uint8Array(buf.buffer, buf.byteOffset + 9, flashLen);
                    // Close old emulator
                    if (emu) { try { emu.close(); } catch {} emu = null; }
                    if (drainTimer) { clearInterval(drainTimer); drainTimer = null; }
                    if (gpioTimer) { clearInterval(gpioTimer); gpioTimer = null; }
                    try {
                        emu = await createEmulator({
                            firmware: new Uint8Array(flash), bindings, unicorn: unicornFactory,
                            svdXml, wasmInit: wasmBytes, verbose, lowpower,
                            onTx: (pkt) => {
                                if (ws.readyState !== 1) return;
                                const msg = new Uint8Array(5 + pkt.length);
                                msg[0] = MSG.PUSH_ETH;
                                new DataView(msg.buffer).setUint32(1, pkt.length, true);
                                msg.set(pkt, 5);
                                try { ws.send(msg); } catch {}
                            },
                        });
                        firmwareLoaded = true;
                        startPeriodicDrain();
                        try { ws.send(encodeResp(MSG.LOAD_OK, id, new Uint8Array(4))); } catch {}
                    } catch (e) {
                        try { ws.send(encodeError(id, e.message)); } catch {}
                    }
                    break;
                }
                case MSG.READ32: {
                    if (!emu) { ws.send(encodeError(u32LE(buf, 1), 'no firmware loaded')); break; }
                    const id = u32LE(buf, 1);
                    const addr = u32LE(buf, 5);
                    try {
                        const v = emu.read32(addr);
                        try { ws.send(encodeResp(MSG.READ32_RESP, id, packU32LE(v))); } catch {}
                    } catch (e) {
                        try { ws.send(encodeError(id, e.message)); } catch {}
                    }
                    break;
                }
                case MSG.WRITE32: {
                    if (!emu) { ws.send(encodeError(u32LE(buf, 1), 'no firmware loaded')); break; }
                    const id = u32LE(buf, 1);
                    const addr = u32LE(buf, 5);
                    const value = u32LE(buf, 9);
                    try {
                        emu.write32(addr, value);
                        try { ws.send(encodeResp(MSG.WRITE32_OK, id, new Uint8Array(4))); } catch {}
                    } catch (e) {
                        try { ws.send(encodeError(id, e.message)); } catch {}
                    }
                    break;
                }
                case MSG.GET_REGS: {
                    if (!emu) { ws.send(encodeError(u32LE(buf, 1), 'no firmware loaded')); break; }
                    const id = u32LE(buf, 1);
                    try {
                        const regs = emu.getRegisters();
                        const out = new Uint8Array(1 + 4 + 36 * 4);
                        out[0] = MSG.REGS_RESP;
                        new DataView(out.buffer).setUint32(1, id, true);
                        let off = 5;
                        for (const n of ['R0','R1','R2','R3','R4','R5','R6','R7','R8','R9','R10','R11','R12','SP','LR','PC']) {
                            new DataView(out.buffer).setUint32(off, regs[n] >>> 0, true); off += 4;
                        }
                        try { ws.send(out); } catch {}
                    } catch (e) {
                        try { ws.send(encodeError(id, e.message)); } catch {}
                    }
                    break;
                }
                case MSG.ETH_RX: {
                    if (!emu) break;
                    const len = u32LE(buf, 1);
                    const frame = new Uint8Array(buf.buffer, buf.byteOffset + 5, len);
                    emu.injectFrame(frame);
                    break;
                }
                case MSG.CAN_RX: {
                    if (!emu) break;
                    const canId = u16LE(buf, 1);
                    const dlc = buf[3];
                    const data = buf.slice(4, 4 + 8);
                    emu.canInject(canId, dlc, data);
                    break;
                }
                case MSG.UART_TX: {
                    if (!emu) break;
                    const len = u16LE(buf, 1);
                    const bytes = buf.slice(3, 3 + len);
                    emu.sendUart(bytes);
                    break;
                }
                case MSG.SET_INPUT: {
                    if (!emu) break;
                    const pinLen = buf[1];
                    const pinName = buf.slice(2, 2 + pinLen).toString('ascii');
                    const level = buf[2 + pinLen] ? 1 : 0;
                    const pin = emu.pin(pinName);
                    if (pin) pin.setInputValue(level);
                    break;
                }
                default:
                    break;
            }
        } catch (e) {
            console.error('ws-bridge: message handler error:', e);
        }
    });
}

// ── WebSocket server ────────────────────────────────────────────────────────
const wss = new WebSocketServer({ port });
console.log(`ws-bridge listening on ws://127.0.0.1:${port}`);
if (firmwarePath) console.log(`  firmware: ${firmwarePath}`);
console.log('  waiting for browser connections…');

wss.on('connection', (ws, req) => {
    const ip = req.socket.remoteAddress;
    console.log(`ws-bridge: client connected from ${ip}`);
    handleConnection(ws);
    ws.on('close', () => console.log(`ws-bridge: client disconnected`));
});

// Graceful shutdown
process.on('SIGINT', () => { wss.close(); process.exit(0); });
process.on('SIGTERM', () => { wss.close(); process.exit(0); });
