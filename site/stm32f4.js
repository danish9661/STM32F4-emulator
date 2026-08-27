// STM32F4 high-level facade (rp2040js / avr8js style) over the Unicorn-based
// emulator. Thin: zero runtime overhead — every call delegates to the
// underlying `emu` produced by createEmulator(); GPIO/USART/SPI/I2C events ride
// the existing MMIO/code hooks. CPU execution is still Unicorn (a prebuilt QEMU
// WASM core); only the on-chip peripherals are a Rust WASM model.
//
// Virtual-peripheral API (Wokwi-style): SPI/I2C taps must be registered before
// the model's init_svd() (the Spi/I2c peripheral snapshots its device list once
// at construction), so they are declared at create() time via the `spi`/`i2c`
// options — exactly like rp2040js components. No Rust change is needed: the
// model already emits transaction-level events (spi_take_events /
// i2c_take_events) and accepts injected reply bytes (spi_push_miso /
// i2c_push_rx).
import { createEmulator } from './emulator.js';
import { parseElf, parseIntelHex } from './loaders.js';

const FLASH_BASE = 0x08000000;
const FLASH_SIZE = 0x00100000;
const RAM_BASE = 0x20000000;

// A Cortex-M vector table with a non-zero SP/PC so createEmulator's reset-vector
// check passes; the real firmware is written later via loadBin/loadHex/loadELF,
// which also resets SP/PC from the loaded image.
const PLACEHOLDER_VECTOR = new Uint8Array([0x00, 0x00, 0x20, 0x00, 0x85, 0x01, 0x00, 0x08]);

function buildFlashImage(bytes, base) {
    const img = new Uint8Array(FLASH_SIZE);
    const off = base - FLASH_BASE;
    if (off < 0 || off + bytes.length > FLASH_SIZE) {
        throw new Error(`flash image of ${bytes.length} bytes at 0x${base.toString(16)} does not fit in FLASH 0x${FLASH_BASE.toString(16)}`);
    }
    img.set(bytes, off);
    return img;
}

// ── SPI event parsing ───────────────────────────────────────────────────────
// events: combined u32 stream. Byte events are v & 0xFF (bit31 clear); a
// DC-level bit (from an optional dc pin) sits at bit 9. CS edges have bit31 set
// and bit30 = 1 means CS asserted (LOW, transfer START), 0 means deasserted
// (HIGH, transfer END) — matching the Rust spi_tap_push_cs encoding.
//
// IMPORTANT: the model drains the event queue once per `step`, so a single
// CS-active transfer is usually split across several `parseSpi` calls. The
// transfer state (inXfer/tx/rx) therefore lives on `spec._st` and persists
// across calls — do not use local variables for it.
function parseSpi(events, push, spec) {
    const st = spec._st || (spec._st = { inXfer: false, tx: [], rx: [] });
    const ch = spec.peripheral;
    for (const v of events) {
        if (v & 0x80000000) {
            if (v & 0x40000000) { // CS asserted (LOW) -> start of transfer
                st.inXfer = true;
                st.tx = [];
                st.rx = [];
            } else { // CS deasserted (HIGH) -> end of transfer
                if (st.inXfer && spec.onTransfer) spec.onTransfer(ch, st.tx, st.rx);
                st.inXfer = false;
            }
        } else {
            const byte = v & 0xFF;
            const dc = (v >> 9) & 1;
            if (!st.inXfer) continue;
            st.tx.push(byte);
            let resp = 0xFF;
            if (spec.onByte) spec.onByte(ch, byte, (b) => { resp = b & 0xFF; push([resp]); }, dc);
            else push([0xFF]);
            st.rx.push(resp);
        }
    }
}

// ── I2C event parsing ───────────────────────────────────────────────────────
// events: combined u32 stream (master-written data bytes + START/STOP edges).
// START edge = (1<<31)|(1<<30); STOP edge = (1<<31). The model does NOT push
// the address+R/W byte (only data bytes written after the address), so onStart
// is called with the device's configured `address` and every subsequent data
// byte is delivered to onWrite. Master reads are served from the push_rx queue
// (i2c_push_rx / onRead); there is no per-read event in the model.
//
// Transfer state persists on `spec._st` across calls (event queue is drained
// per step, splitting a transaction over multiple calls).
function parseI2c(events, push, spec) {
    const st = spec._st || (spec._st = { started: false });
    const periph = spec.peripheral;
    for (const v of events) {
        if (v & 0x80000000) {
            if (v & 0x40000000) { // START
                st.started = true;
                if (spec.onStart) spec.onStart(spec.address, false);
            } else { // STOP
                if (st.started && spec.onStop) spec.onStop(periph);
                st.started = false;
            }
        } else {
            if (!st.started) continue;
            if (spec.onWrite) spec.onWrite(v & 0xFF);
        }
    }
}

// A single GPIO pin. `.on('change', cb)` fires with `true`/`false` whenever
// the MCU drives the output level. Inputs can be driven from the host with
// `setInputValue`.
export class GPIOPin {
    constructor(mcu, port, pin) {
        this.mcu = mcu;
        this.port = port;
        this.pin = pin;
        this._listeners = [];
        this._unwatch = null;
        this._state = null;
    }
    on(event, cb) {
        if (event === 'change') {
            if (!this._unwatch) {
                this._unwatch = this.mcu._emu.watchPin(this.port, this.pin, (v) => {
                    this._state = v;
                    for (const l of this._listeners) l(!!v);
                });
            }
            this._listeners.push(cb);
        }
        return this;
    }
    addListener(cb) { return this.on('change', cb); }
    setInputValue(high) { this.mcu._emu.pin(this.port, this.pin).write(!!high); }
    read() { return !!this.mcu._emu.pin(this.port, this.pin).read(); }
    readInput() { return !!this.mcu._emu.pin(this.port, this.pin).readInput(); }
    detach() {
        if (this._unwatch) { this._unwatch(); this._unwatch = null; this._listeners = []; }
    }
}

// A USART peripheral. `onData` receives each transmitted byte; `sendData`
// injects bytes into the guest's RX stream (as if received on the wire).
export class USART {
    constructor(mcu, n) {
        this.mcu = mcu;
        this.n = n;
        this.onData = null;
    }
    _emit(byte) { if (this.onData) this.onData(byte); }
    sendData(data) {
        const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
        this.mcu._emu.sendUart(bytes);
    }
}

// A DMA stream. The underlying model exposes a single pending-set counter and
// a per-stream "completed" latch; `streamIndex` follows the model's stream
// enumeration (0..N).
export class DMAStream {
    constructor(mcu, streamIndex) {
        this.mcu = mcu;
        this.index = streamIndex;
    }
    pendingCount() { return this.mcu._emu.dmaPendingCount(); }
    setCompleted(success = true) { this.mcu._emu.dmaSetCompleted(this.index, success); }
}

export class STM32F4 {
    constructor(emu, bindings = null) {
        this._emu = emu;
        this._bindings = bindings;
        this._spiSpecs = [];
        this._i2cSpecs = [];
        this.gpio = {
            pin: (port, pin) => new GPIOPin(this, port, pin),
        };
        this.usart = new USART(this, 1);
        // The underlying model exposes a single UART channel in this build, so
        // the common USART aliases all bind to it.
        this.usart1 = this.usart;
        this.usart2 = this.usart;
        this.usart3 = this.usart;
        this.dma = {
            stream: (index) => new DMAStream(this, index),
        };
        this.spi = {
            // specs: array of { peripheral, cs?, dc?, onTransfer?, onByte? }
            specs: this._spiSpecs,
            // Inject MISO bytes the model returns on the next master reads.
            pushMiso: (peripheral, bytes) => {
                if (this._bindings && this._bindings.spi_push_miso) {
                    this._bindings.spi_push_miso(peripheral, new Uint8Array(bytes));
                }
            },
        };
        this.i2c = {
            // specs: array of { peripheral, address, onStart?, onWrite?, onRead?, onStop? }
            specs: this._i2cSpecs,
            // Pre-supply read responses (master reads pop from this queue).
            pushRx: (peripheral, bytes) => {
                if (this._bindings && this._bindings.i2c_push_rx) {
                    this._bindings.i2c_push_rx(peripheral, new Uint8Array(bytes));
                }
            },
        };
    }

    // Create an emulator with assets already resolved (Node). The `firmware`
    // option is optional here: when omitted, call `loadBin`/`loadHex`/`loadELF`
    // before `execute`. `spi`/`i2c` declare virtual peripherals (registered
    // before init_svd, like rp2040js components). Extra options pass through to
    // createEmulator().
    static async create(opts = {}) {
        return STM32F4._create(opts);
    }

    static async _create(opts = {}) {
        const firmware = opts.firmware || PLACEHOLDER_VECTOR;
        const bindings = opts.bindings || null;
        const ext_devices = { ...(opts.ext_devices || {}) };
        const spiDevs = [...(ext_devices.spiDevices || [])];
        const i2cDevs = [...(ext_devices.i2cDevices || [])];
        for (const s of (opts.spi || [])) {
            spiDevs.push({
                peripheral: s.peripheral,
                cs: s.cs ?? null,
                dc: s.dc ?? null,
                handler: (events, push) => parseSpi(events, push, s),
            });
        }
        for (const d of (opts.i2c || [])) {
            i2cDevs.push({
                peripheral: d.peripheral,
                address: d.address,
                handler: (events, push) => parseI2c(events, push, d),
            });
        }
        ext_devices.spiDevices = spiDevs;
        ext_devices.i2cDevices = i2cDevs;
        const emu = await createEmulator({ ...opts, firmware, ext_devices });
        const mcu = new STM32F4(emu, bindings);
        mcu._spiSpecs.push(...(opts.spi || []));
        mcu._i2cSpecs.push(...(opts.i2c || []));
        return mcu;
    }

    // ── firmware loading ──
    loadBin(bytes, base = FLASH_BASE) {
        const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const flash = buildFlashImage(buf, base);
        this._emu.loadImage({ flash });
    }
    loadHex(text) {
        const { flash, ram } = parseIntelHex(text);
        const extraMem = [];
        if (ram) extraMem.push({ addr: RAM_BASE, data: ram });
        this._emu.loadImage({ flash: flash || new Uint8Array(0), extraMem });
    }
    loadELF(bytes) {
        const { flash, extraMem } = parseElf(bytes);
        this._emu.loadImage({ flash: flash || new Uint8Array(0), extraMem });
    }

    // ── execution ──
    execute(cycles = 100000) {
        this._emu.step(cycles);
        const out = this._emu.drainUart();
        if (!out) return;
        if (typeof out === 'string') {
            for (let i = 0; i < out.length; i++) this.usart._emit(out.charCodeAt(i));
        } else {
            for (let i = 0; i < out.length; i++) this.usart._emit(out[i]);
        }
    }

    // ── pass-through to the engine ──
    read32(addr) { return this._emu.read32(addr); }
    write32(addr, val) { return this._emu.write32(addr, val); }
    getRegisters() { return this._emu.getRegisters(); }
    stop() { return this._emu.stop(); }
    reset() { return this._emu.reset(); }
    close() { return this._emu.close(); }
}
