// remote-emu.js — Drop-in replacement for the `emu` object produced by
// createEmulator(), but proxies every call over a binary WebSocket to
// ws-bridge.mjs (Node).  The browser becomes a thin UI; all WASM
// execution happens in Node.
//
// Usage:
//   import { createRemoteEmulator } from './remote-emu.js';
//   const emu = await createRemoteEmulator('ws://127.0.0.1:8234');
//   // emu.step(), emu.drainUart(), emu.read32(), etc. work like the local emu.

const MSG = {
    STEP: 0x01, STOP: 0x02, RESET: 0x03, LOAD_IMAGE: 0x04,
    READ32: 0x10, WRITE32: 0x11, GET_REGS: 0x12,
    ETH_RX: 0x20, CAN_RX: 0x21, UART_TX: 0x22,
    SPI_MISO: 0x30, I2C_RX: 0x31, SET_INPUT: 0x40,
    PUSH_UART: 0x80, PUSH_ETH: 0x81, PUSH_GPIO: 0x82,
    STOPPED: 0x8A,
    STEP_RESP: 0x90, READ32_RESP: 0x91, WRITE32_OK: 0x92,
    LOAD_OK: 0x93, REGS_RESP: 0x94, ERROR: 0xA0,
};

let nextId = 1;

function packU32(v) {
    return new Uint8Array([v & 0xFF, (v >>> 8) & 0xFF, (v >>> 16) & 0xFF, (v >>> 24) & 0xFF]);
}

function readU32(buf, off) {
    return buf[off] | (buf[off + 1] << 8) | (buf[off + 2] << 16) | (buf[off + 3] << 24);
}

export async function createRemoteEmulator(url, opts = {}) {
    const ws = new WebSocket(url);
    ws.binaryType = 'arraybuffer';
    await new Promise((resolve, reject) => {
        ws.onopen = resolve;
        ws.onerror = () => reject(new Error('WebSocket connection failed: ' + url));
        ws.onclose = () => reject(new Error('WebSocket closed before open'));
    });

    const pending = new Map(); // id -> { resolve, reject }
    let uartBuf = '';
    let connected = true;

    // Handle incoming messages
    ws.onmessage = (ev) => {
        const buf = new Uint8Array(ev.data);
        if (buf.length < 1) return;
        const type = buf[0];

        switch (type) {
            case MSG.PUSH_UART: {
                if (buf.length >= 3) {
                    const len = buf[1] | (buf[2] << 8);
                    if (buf.length >= 3 + len) {
                        uartBuf += new TextDecoder().decode(buf.slice(3, 3 + len));
                    }
                }
                break;
            }
            case MSG.PUSH_ETH: {
                if (buf.length >= 5) {
                    const len = readU32(buf, 1);
                    const frame = new Uint8Array(buf.buffer, buf.byteOffset + 5, len);
                    if (opts.onTx) opts.onTx(frame);
                }
                break;
            }
            case MSG.PUSH_GPIO: {
                if (opts.onGpio) {
                    const bank = buf[1];
                    const idr = readU32(buf, 2);
                    const odr = readU32(buf, 6);
                    const moder = readU32(buf, 10);
                    opts.onGpio(bank, idr, odr, moder);
                }
                break;
            }
            case MSG.STOPPED: {
                if (opts.onStopped) opts.onStopped();
                break;
            }
            case MSG.STEP_RESP:
            case MSG.READ32_RESP:
            case MSG.WRITE32_OK:
            case MSG.LOAD_OK:
            case MSG.REGS_RESP:
            case MSG.ERROR: {
                // Response to a request — resolve the pending promise
                if (buf.length >= 5) {
                    const id = readU32(buf, 1);
                    const p = pending.get(id);
                    if (p) {
                        pending.delete(id);
                        if (type === MSG.ERROR) {
                            const msgLen = buf.length >= 7 ? (buf[5] | (buf[6] << 8)) : 0;
                            const msg = msgLen > 0 ? new TextDecoder().decode(buf.slice(7, 7 + msgLen)) : 'unknown error';
                            p.reject(new Error(msg));
                        } else {
                            p.resolve({ type, buf });
                        }
                    }
                }
                break;
            }
        }
    };

    ws.onclose = () => {
        connected = false;
        for (const p of pending.values()) p.reject(new Error('WebSocket closed'));
        pending.clear();
    };

    function request(type, ...payloads) {
        if (!connected) return Promise.reject(new Error('not connected'));
        const id = nextId++;
        const parts = [new Uint8Array([type]), packU32(id), ...payloads];
        const len = parts.reduce((s, p) => s + p.length, 0);
        const msg = new Uint8Array(len);
        let off = 0;
        for (const p of parts) { msg.set(p, off); off += p.length; }
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject });
            try { ws.send(msg); } catch (e) { pending.delete(id); reject(e); }
        });
    }

    function fire(type, ...payloads) {
        if (!connected) return;
        const parts = [new Uint8Array([type]), ...payloads];
        const len = parts.reduce((s, p) => s + p.length, 0);
        const msg = new Uint8Array(len);
        let off = 0;
        for (const p of parts) { msg.set(p, off); off += p.length; }
        try { ws.send(msg); } catch {}
    }

    return {
        step(maxInst = 100000) {
            return request(MSG.STEP, packU32(maxInst)).then(({ buf }) => ({
                instCount: readU32(buf, 5),
                stopped: buf[9] !== 0,
            }));
        },

        drainUart() {
            const u = uartBuf;
            uartBuf = '';
            return u;
        },

        read32(addr) {
            return request(MSG.READ32, packU32(addr)).then(({ buf }) => readU32(buf, 5));
        },

        write32(addr, value) {
            return request(MSG.WRITE32, packU32(addr), packU32(value)).then(() => {});
        },

        injectFrame(frame) {
            fire(MSG.ETH_RX, packU32(frame.length), frame instanceof Uint8Array ? frame : new Uint8Array(frame));
        },

        sendUart(bytes) {
            const b = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
            const msg = new Uint8Array(3 + b.length);
            msg[0] = MSG.UART_TX;
            msg[1] = b.length & 0xFF;
            msg[2] = (b.length >> 8) & 0xFF;
            msg.set(b, 3);
            try { ws.send(msg); } catch {}
        },

        canInject(id, dlc, data) {
            const d = new Uint8Array(8);
            if (data) d.set(data instanceof Uint8Array ? data : new Uint8Array(data));
            fire(MSG.CAN_RX, new Uint8Array([id & 0xFF, (id >> 8) & 0xFF, dlc & 0xF]), d);
        },

        timInjectCapture(_name, _ch) {
            // Not proxied yet — can be added later if needed.
        },

        getRegisters() {
            return request(MSG.GET_REGS).then(({ buf }) => {
                const regs = {};
                const names = ['R0','R1','R2','R3','R4','R5','R6','R7','R8','R9','R10','R11','R12','SP','LR','PC'];
                for (let i = 0; i < names.length; i++) {
                    regs[names[i]] = readU32(buf, 5 + i * 4) >>> 0;
                }
                return regs;
            });
        },

        loadImage(flash, extraMem) {
            const f = flash instanceof Uint8Array ? flash : new Uint8Array(flash);
            return request(MSG.LOAD_IMAGE, packU32(f.length), f).then(() => {});
        },

        stop() { fire(MSG.STOP); },
        reset() { fire(MSG.RESET); },
        close() {
            connected = false;
            try { ws.close(); } catch {}
        },

        // Stubbed out — not meaningful over the bridge (devices are in Node).
        pin() { return null; },
        watchPin() {},
        i2cRegfile: null,
        setAdcChannel() {},
        clearAdcChannel() {},
        oled: null, tft: null, buzzer: null, rtc: null,
        takeSpeakerSamples() { return new Float32Array(0); },
        pushFsmcData() {},
        takeFsmcEvents() { return []; },
        camera: { feed() {}, stop() {}, start() {}, get frames() { return 0; } },
        get rxQueue() { return []; },

        // Expose ws for diagnostics.
        _ws: ws,
    };
}
