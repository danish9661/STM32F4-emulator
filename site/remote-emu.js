// remote-emu.js — Drop-in replacement for the `emu` object produced by
// createEmulator(), but proxies every call over a binary WebSocket to
// ws-bridge.mjs (Node).  The browser becomes a thin UI; all WASM
// execution happens in Node.
//
// Features beyond a raw proxy:
//   • Auto-reconnect with exponential backoff on connection drops.
//   • Ping/pong keepalive (server pings every 30s, client responds).
//   • Per-request timeout (default 10s) so a stuck server doesn't hang
//     the browser forever.
//   • Connection-state callbacks (onDisconnect / onReconnect) so the UI
//     can show a visible indicator.
//   • Firmware re-send on reconnect: the adapter caches the last
//     loadImage() payload and replays it automatically after a
//     successful reconnect so the new emulator instance is ready.
//
// Usage:
//   import { createRemoteEmulator } from './remote-emu.js';
//   const emu = await createRemoteEmulator('ws://127.0.0.1:8234', {
//       onDisconnect: () => setStatus('disconnected', 'err'),
//       onReconnect:  () => setStatus('reconnected', 'run'),
//   });
//   // emu.step(), emu.drainUart(), emu.read32(), etc. work like the local emu.
//   // On connection drop, pending requests reject and the adapter reconnects.
//   // When the firmware was loaded via loadImage(), it is re-sent automatically.

const MSG = {
    STEP: 0x01, STOP: 0x02, RESET: 0x03, LOAD_IMAGE: 0x04,
    READ32: 0x10, WRITE32: 0x11, GET_REGS: 0x12,
    ETH_RX: 0x20, CAN_RX: 0x21, UART_TX: 0x22,
    SPI_MISO: 0x30, I2C_RX: 0x31, SET_INPUT: 0x40,
    PUSH_UART: 0x80, PUSH_ETH: 0x81, PUSH_GPIO: 0x82,
    STOPPED: 0x8A, PING: 0xFE, PONG: 0xFF,
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

// ── connection-state enum ──────────────────────────────────────────────────
export const ConnectionState = {
    CONNECTING: 'connecting',
    CONNECTED: 'connected',
    RECONNECTING: 'reconnecting',
    DISCONNECTED: 'disconnected',
};

export async function createRemoteEmulator(url, opts = {}) {
    const {
        requestTimeout = 10000,     // ms before a pending request is rejected
        reconnectBase = 500,        // initial reconnect delay (ms)
        reconnectMax = 5000,        // cap on reconnect delay (ms)
        onDisconnect = null,        // () => void  — called on connection loss
        onReconnect = null,         // () => void  — called after successful reconnect + firmware re-send
        onStateChanged = null,      // (state: string) => void  — called on any state transition
        onTx = null,                // (frame: Uint8Array) => void
        onGpio = null,              // (bank, idr, odr, moder) => void
        onStopped = null,           // () => void
    } = opts;

    const pending = new Map();     // id -> { resolve, reject, timer }
    let uartBuf = '';
    let ws = null;
    let state = ConnectionState.DISCONNECTED;
    let reconnectDelay = reconnectBase;
    let reconnectTimer = null;
    let pingTimer = null;
    let alive = true;              // set false on close, prevents reconnect
    let lastFirmware = null;       // { flash: Uint8Array } for auto-reconnect
    let wsReady = false;           // true once onopen fires

    function setState(s) {
        if (state === s) return;
        state = s;
        if (onStateChanged) onStateChanged(s);
    }

    // ── request timeout ─────────────────────────────────────────────────
    function startTimeout(id) {
        const p = pending.get(id);
        if (!p) return;
        p.timer = setTimeout(() => {
            pending.delete(id);
            p.reject(new Error(`request ${id} timed out after ${requestTimeout}ms`));
        }, requestTimeout);
    }

    function clearTimeouts() {
        for (const p of pending.values()) {
            if (p.timer) clearTimeout(p.timer);
        }
    }

    // ── reject all pending (on disconnect) ──────────────────────────────
    function rejectPending(reason) {
        clearTimeouts();
        for (const p of pending.values()) p.reject(reason);
        pending.clear();
    }

    // ── WebSocket setup ─────────────────────────────────────────────────
    function handleMessage(data) {
        const buf = new Uint8Array(data);
        if (buf.length < 1) return;
        const type = buf[0];

        switch (type) {
            case MSG.PONG: {
                alive = true;
                break;
            }
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
                    if (onTx) onTx(frame);
                }
                break;
            }
            case MSG.PUSH_GPIO: {
                if (onGpio) {
                    const bank = buf[1];
                    const idr = readU32(buf, 2);
                    const odr = readU32(buf, 6);
                    const moder = readU32(buf, 10);
                    onGpio(bank, idr, odr, moder);
                }
                break;
            }
            case MSG.STOPPED: {
                if (onStopped) onStopped();
                break;
            }
            case MSG.STEP_RESP:
            case MSG.READ32_RESP:
            case MSG.WRITE32_OK:
            case MSG.LOAD_OK:
            case MSG.REGS_RESP:
            case MSG.ERROR: {
                if (buf.length >= 5) {
                    const id = readU32(buf, 1);
                    const p = pending.get(id);
                    if (p) {
                        pending.delete(id);
                        if (p.timer) clearTimeout(p.timer);
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
    }

    function handleClose() {
        wsReady = false;
        rejectPending(new Error('WebSocket closed'));
        setState(ConnectionState.DISCONNECTED);
        if (pingTimer) { clearInterval(pingTimer); pingTimer = null; }
        if (onDisconnect) onDisconnect();
        if (alive) scheduleReconnect();
    }

    function handleError() {
        // onClose always fires after onError, so reconnect logic is in handleClose.
    }

    function handleOpen() {
        wsReady = true;
        alive = true;
        reconnectDelay = reconnectBase;     // reset backoff on success
        setState(ConnectionState.CONNECTED);
        // Start pong keepalive: server pings every 30s, we must respond.
        // If we miss 3 pings (90s), treat as dead and trigger reconnect.
        alive = true;
        if (pingTimer) clearInterval(pingTimer);
        let missedPongs = 0;
        pingTimer = setInterval(() => {
            if (!alive) {
                missedPongs++;
                if (missedPongs >= 3) {
                    // Server is dead — force close and reconnect.
                    try { ws.close(); } catch {}
                    return;
                }
            } else {
                missedPongs = 0;
                alive = false;  // will be set true by PONG
            }
        }, 30000);
    }

    function scheduleReconnect() {
        if (reconnectTimer) return;
        setState(ConnectionState.RECONNECTING);
        reconnectTimer = setTimeout(async () => {
            reconnectTimer = null;
            await connect();
        }, reconnectDelay);
        // Exponential backoff: double each time, cap at reconnectMax.
        reconnectDelay = Math.min(reconnectDelay * 2, reconnectMax);
    }

    async function connect() {
        return new Promise((resolve) => {
            const newWs = new WebSocket(url);
            newWs.binaryType = 'arraybuffer';

            const onFirstOpen = () => {
                ws = newWs;
                newWs.onmessage = handleMessage;
                newWs.onclose = handleClose;
                newWs.onerror = handleError;
                handleOpen();

                // Re-send firmware if we had one loaded.
                if (lastFirmware) {
                    const f = lastFirmware.flash;
                    const id = nextId++;
                    const msg = new Uint8Array(9 + f.length);
                    msg[0] = MSG.LOAD_IMAGE;
                    new DataView(msg.buffer).setUint32(1, id, true);
                    new DataView(msg.buffer).setUint32(5, f.length, true);
                    msg.set(f, 9);
                    try { newWs.send(msg); } catch {}
                    // Wait for LOAD_OK or ERROR (with timeout).
                    const p = {
                        resolve: () => {
                            if (onReconnect) onReconnect();
                            resolve(true);
                        },
                        reject: (e) => {
                            console.error('ws-bridge: firmware re-send failed:', e.message);
                            resolve(false);
                        },
                    };
                    pending.set(id, { ...p, timer: setTimeout(() => {
                        pending.delete(id);
                        p.reject(new Error('firmware re-send timed out'));
                    }, requestTimeout) });
                } else {
                    resolve(true);
                }
            };

            newWs.onopen = onFirstOpen;
            newWs.onerror = () => {
                newWs.onclose = null;  // don't trigger handleClose for a failed initial connect
                resolve(false);
            };
            newWs.onclose = () => {
                // Initial connect failed — schedule retry.
                newWs.onopen = null;
                resolve(false);
            };
        });
    }

    // ── initial connection ──────────────────────────────────────────────
    setState(ConnectionState.CONNECTING);
    await connect();

    // ── low-level helpers ───────────────────────────────────────────────
    function request(type, ...payloads) {
        if (!wsReady) return Promise.reject(new Error('not connected'));
        const id = nextId++;
        const parts = [new Uint8Array([type]), packU32(id), ...payloads];
        const len = parts.reduce((s, p) => s + p.length, 0);
        const msg = new Uint8Array(len);
        let off = 0;
        for (const p of parts) { msg.set(p, off); off += p.length; }
        return new Promise((resolve, reject) => {
            pending.set(id, { resolve, reject, timer: null });
            try { ws.send(msg); startTimeout(id); }
            catch (e) { pending.delete(id); reject(e); }
        });
    }

    function fire(type, ...payloads) {
        if (!wsReady) return;
        const parts = [new Uint8Array([type]), ...payloads];
        const len = parts.reduce((s, p) => s + p.length, 0);
        const msg = new Uint8Array(len);
        let off = 0;
        for (const p of parts) { msg.set(p, off); off += p.length; }
        try { ws.send(msg); } catch {}
    }

    // ── the adapter object ──────────────────────────────────────────────
    const adapter = {
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

        timInjectCapture(_name, _ch) {},

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
            lastFirmware = { flash: new Uint8Array(f) };  // cache for reconnect
            return request(MSG.LOAD_IMAGE, packU32(f.length), f).then(() => {});
        },

        stop() { fire(MSG.STOP); },
        reset() { fire(MSG.RESET); },

        close() {
            alive = false;                // prevent reconnect
            if (reconnectTimer) { clearTimeout(reconnectTimer); reconnectTimer = null; }
            if (pingTimer) { clearInterval(pingTimer); pingTimer = null; }
            rejectPending(new Error('adapter closed'));
            try { ws.close(); } catch {}
            setState(ConnectionState.DISCONNECTED);
        },

        // ── connection state ────────────────────────────────────────────
        get connectionState() { return state; },
        get isConnected() { return wsReady; },

        // Stubbed — not meaningful over the bridge (devices are in Node).
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

        _ws: ws,
    };

    return adapter;
}
