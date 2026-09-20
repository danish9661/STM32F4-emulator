// STM32F4 high-level facade (rp2040js / avr8js style) over the Rust-CPU
// emulator. Thin: zero runtime overhead — every call delegates to the
// underlying `emu` produced by createEmulator(); GPIO/USART/SPI/I2C events ride
// the bus taps. CPU execution is the WASM-native Thumb-2 interpreter; the
// on-chip peripherals are a Rust WASM model.
//
// F1-API parity notes (see stm32api.md — F1 lives in a separate repo, this
// file covers F4 only):
// - Factories: create/fromBin/fromELF/fromHex + loadBin/loadHex/loadELF.
// - execute() returns {instCount, stopped}; step() returns {pc, instCount,
//   stopped} (both drain UART into usart1, like F1's event drain).
// - gpio.pin() coerces index ports; GPIOPin.on('change') returns unsubscribe.
// - USART slots are per-chip: F401/F411 expose 1,2,6 only (SVD census),
//   absent slots are null. TX is a single shared model buffer, so
//   onData/output only ever fire on usart1 (usart2-6 send() works, their
//   onData never fires — model limit, not a bug).
// - SPI/I2C stay create-time specs (taps snapshot at init — post-create
//   callback assignment like F1's spi[ch].onTransfer is impossible without a
//   Rust rescan; a silent no-fire shim would be worse than the explicit opt).
// - Polled top-level callbacks (onExtiEdge/onCanTx/...) dispatch from
//   model-readable registers once per execute()/step() — the F4 model has
//   no core event queue (no drainEvents). Polls skip absent-silicon
//   addresses (CAN on F401/F411, missing TIMs) — reads are benign-0.
// - Symbol/SWD/JTAG/pwr/js-peripheral helpers are JS-side; the model has
//   no DP/MEM-AP/calibrated-current/hook-table, so behavior is documented
//   shims (live file/IDCODE/PWR_CR reads where a source exists).
// - `chip` (f401/f411/f407/f407ve/f429) resolves SVD + flash/RAM sizes +
//   clock + IDCODE + presence lists; unknown chip throws. Explicit
//   svdXml/flash_size/ram_size opts override the table. createEmulator()
//   callers hitting the model directly must pass the same triple
//   themselves (the facade does it for you).
// Virtual-peripheral API (Wokwi-style): SPI/I2C taps must be registered before
// the model's init_svd() (the Spi/I2c peripheral snapshots its device list once
// at construction), so they are declared at create() time via the `spi`/`i2c`
// options — exactly like rp2040js components. No Rust change is needed: the
// model already emits transaction-level events (spi_take_events /
// i2c_take_events) and accepts injected reply bytes (spi_push_miso /
// i2c_push_rx).
import { createEmulator } from './emulator.js';
import { parseElf, parseIntelHex, parseMap } from './loaders.js';

const FLASH_BASE = 0x08000000;
const FLASH_SIZE = 0x00100000;
const RAM_BASE = 0x20000000;

// ── Chip table (SVD truth, site/vendor/*.svd + site/boards.js) ─────────
// Every F4 here is a Cortex-M4F — one shared CPU core, no decoder work
// per chip. A variant is SVD (register map) + flash/RAM sizes + clock.
// USART/TIM/CAN/DAC/LTDC/GPIO presence below was read off the four SVDs
// (Y/x census); bases are identical wherever the peripheral exists.
const CHIPS = {
    stm32f401: {
        svd: 'stm32f401.svd', flash_size: 0x80000, ram_size: 0x18000,
        maxClockMHz: 84, label: 'STM32F401 (512K/96K)', idcode: 0x423,
        usarts: [1, 2, 6], timers: [1, 2, 3, 4, 5, 8, 9, 10, 11],
        can: [], dac: false, ltdc: false, gpioBanks: 6, // A-F
    },
    stm32f411: {
        svd: 'stm32f411.svd', flash_size: 0x80000, ram_size: 0x20000,
        maxClockMHz: 100, label: 'STM32F411 (512K/128K)', idcode: 0x431,
        usarts: [1, 2, 6], timers: [1, 2, 3, 4, 5, 8, 9, 10, 11],
        can: [], dac: false, ltdc: false, gpioBanks: 6, // A-F
    },
    stm32f407: {
        svd: 'stm32f407.svd', flash_size: 0x100000, ram_size: 0x30000,
        maxClockMHz: 168, label: 'STM32F407 (1M/192K)', idcode: 0x413,
        usarts: [1, 2, 3, 4, 5, 6], timers: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        can: [1, 2], dac: true, ltdc: true, gpioBanks: 11, // A-K
    },
    stm32f407ve: {
        svd: 'stm32f407.svd', flash_size: 0x80000, ram_size: 0x30000,
        maxClockMHz: 168, label: 'STM32F407VE/ZE (512K/192K)', idcode: 0x413,
        usarts: [1, 2, 3, 4, 5, 6], timers: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        can: [1, 2], dac: true, ltdc: true, gpioBanks: 11, // A-K
    },
    stm32f429: {
        svd: 'stm32f429.svd', flash_size: 0x200000, ram_size: 0x40000,
        maxClockMHz: 180, label: 'STM32F429 (2M/256K)', idcode: 0x419,
        usarts: [1, 2, 3, 4, 5, 6], timers: [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14],
        can: [1, 2], dac: true, ltdc: true, gpioBanks: 11, // A-K
    },
};

function resolveChip(opts = {}) {
    const key = String(opts.chip || opts.board || 'stm32f407').toLowerCase();
    const table = CHIPS[key];
    if (!table) throw new Error(`unknown chip '${opts.chip || opts.board}' (have: ${Object.keys(CHIPS).join(', ')})`);
    return { key, ...table };
}

function chipInfo(key) {
    const k = String(key || 'stm32f407').toLowerCase();
    const table = CHIPS[k];
    if (!table) throw new Error(`unknown chip '${key}' (have: ${Object.keys(CHIPS).join(', ')})`);
    return { key: k, ...table };
}

export { CHIPS, chipInfo };

// A Cortex-M vector table with a non-zero SP/PC so createEmulator's reset-vector
// check passes; the real firmware is written later via loadBin/loadHex/loadELF,
// which also resets SP/PC from the loaded image.
const PLACEHOLDER_VECTOR = new Uint8Array([0x00, 0x00, 0x20, 0x00, 0x85, 0x01, 0x00, 0x08]);

function buildFlashImage(bytes, base, flashSize = FLASH_SIZE) {
    const img = new Uint8Array(flashSize);
    const off = base - FLASH_BASE;
    if (off < 0 || off + bytes.length > flashSize) {
        throw new Error(`flash image of ${bytes.length} bytes at 0x${base.toString(16)} does not fit in FLASH 0x${FLASH_BASE.toString(16)} (chip flash 0x${flashSize.toString(16)})`);
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
// the MCU drives the output level, and returns an unsubscribe function
// (F1 `on()` parity — F4 previously returned `this`). Inputs can be driven
// from the host with `setInputValue`.
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
                    for (const l of [...this._listeners]) l(!!v);
                });
            }
            this._listeners.push(cb);
            return () => {
                const i = this._listeners.indexOf(cb);
                if (i >= 0) this._listeners.splice(i, 1);
                if (this._listeners.length === 0 && this._unwatch) {
                    this._unwatch();
                    this._unwatch = null;
                }
            };
        }
        return () => {};
    }
    addListener(cb) { return this.on('change', cb); }
    // F1 `read()` returns 0|1; F4 returns bool. Both are truthy/falsy
    // compatible; keep bool here (existing callers rely on it).
    read() { return !!this.mcu._emu.pin(this.port, this.pin).read(); }
    readInput() { return !!this.mcu._emu.pin(this.port, this.pin).readInput(); }
    // F1 `setInput(high)` parity (alias of setInputValue).
    setInput(high) { this.setInputValue(high); }
    setInputValue(high) { this.mcu._emu.pin(this.port, this.pin).write(!!high); }
    // F4-only: ADC channel injection has no F1 equivalent (F1's ADC model is
    // event-based). There is no analog-wire layer, so no setAnalog here.
    detach() {
        if (this._unwatch) { this._unwatch(); this._unwatch = null; this._listeners = []; }
    }
}

// F407 USART base addresses (SVD-verified). The model has one shared UART
// TX buffer, but RX injection (`uart_rx_byte`) is per-base, so each USART
// object targets its own peripheral.
const USART_BASE = {
    1: 0x40011000, 2: 0x40004400, 3: 0x40004800,
    4: 0x40004C00, 5: 0x40005000, 6: 0x40011400,
};

// A USART peripheral. `onData` receives each transmitted byte; `send`/
// `sendData` injects bytes into the guest's RX stream (as if received on
// the wire). `output` accumulates this USART's TX bytes (F1 parity).
// MODEL LIMIT: TX is one shared buffer, so only usart1._emit is ever fed
// (see execute()). usart2-6.send() injects RX correctly; their onData and
// output stay empty because the model cannot attribute TX bytes per USART.
export class USART {
    constructor(mcu, n) {
        this.mcu = mcu;
        this.n = n;
        this.onData = null;
        this._buf = [];
    }
    _emit(byte) {
        this._buf.push(byte & 0xFF);
        if (this.onData) this.onData(byte);
    }
    send(data) { this.sendData(data); }
    sendData(data) {
        const bytes = typeof data === 'string' ? new TextEncoder().encode(data) : data;
        const base = USART_BASE[this.n] || USART_BASE[1];
        for (const b of bytes) {
            try { this.mcu._bindings.uart_rx_byte(base, b & 0xFF); }
            catch { this.mcu._emu.sendUart([b & 0xFF]); break; }
        }
    }
    get output() { return String.fromCharCode(...this._buf); }
}

// A DMA stream. `controller` is 1|2 (DMA1/DMA2), `stream` 0-7. The global
// index follows the model's stream enumeration (controller 1: 0-7,
// controller 2: 8-15) so dmaSetCompleted lands on the right latch.
// Prefer mcu.dmaController(1|2).stream(s) over mcu.dma.stream(i): the
// latter is the legacy flat helper (controller 1 only).
export class DMAStream {
    constructor(mcu, streamIndex, controller = 1, stream = null) {
        this.mcu = mcu;
        this.index = streamIndex;
        this.controller = controller;
        this.stream = stream === null ? streamIndex : stream;
    }
    pendingCount() { return this.mcu._emu.dmaPendingCount(); }
    setCompleted(success = true) { this.mcu._emu.dmaSetCompleted(this.index, success); }
    // Live register view for this stream (TCIF/HTIF + CR/NDTR/PAR/MxAR/FCR).
    tcif() { return this.mcu.dmaController(this.controller).tcif(this.stream); }
    htif() { return this.mcu.dmaController(this.controller).htif(this.stream); }
    cr() { return this.mcu.dmaController(this.controller).cr(this.stream); }
    ndtr() { return this.mcu.dmaController(this.controller).ndtr(this.stream); }
}

// Full DMA controller view (Wokwi/OpenHW/Velxio integration): LISR/HISR +
// per-stream CR/NDTR/PAR/M0AR/M1AR/FCR + completion. Controllers 1|2
// (DMA1 @0x40026000, DMA2 @0x40026400 — SVD-verified); streams 0-7;
// LISR covers 0-3, HISR 4-7, 6-bit field per stream (bit4 = TCIF).
// Per-stream regs: CR @+0x10+s*0x18, NDTR @+0x14+s*0x18, PAR @+0x18+s*0x18,
// M0AR @+0x1C+s*0x18, M1AR @+0x20+s*0x18, FCR @+0x24+s*0x18.
export class DMAController {
    constructor(mcu, controller = 1) {
        this.mcu = mcu;
        this.controller = controller;
        this.base = DMA_BASE[controller] || DMA_BASE[1];
    }
    _field(s) {
        const isr = this.mcu._evRead(this.base + (s < 4 ? 0 : 4));
        return (isr >>> ((s % 4) * 6)) & 0x3F;
    }
    _reg(s, off) { return this.mcu._evRead(this.base + 0x10 + s * 0x18 + off); }
    lisr() { return this.mcu._evRead(this.base); }
    hisr() { return this.mcu._evRead(this.base + 4); }
    tcif(s) { return (this._field(s) & (1 << 4)) !== 0; }
    htif(s) { return (this._field(s) & (1 << 3)) !== 0; }
    cr(s) { return this._reg(s, 0); }
    ndtr(s) { return this._reg(s, 4) & 0xFFFF; }
    par(s) { return this._reg(s, 8); }
    m0ar(s) { return this._reg(s, 12); }
    m1ar(s) { return this._reg(s, 16); }
    fcr(s) { return this._reg(s, 20); }
    stream(s) { return new DMAStream(this.mcu, this.controller === 2 ? s + 8 : s, this.controller, s); }
}

// ── Model register map for the polling dispatcher ──────────────────────
// SVD-verified bases. EXTI PR @+0x14 (w1c — dispatcher clears on dispatch);
// CAN1/2 TSR @+0x08 (TXOK/RQCP in low 24 bits), RF0R/RF1R @+0x0C/0x10
// (FMP in low 2 bits); TIMx SR @+0x10 (UIF bit0, CCxIF bits1-4); DMA1/2
// LISR @+0x00, HISR @+0x04 (TCIF per-stream bit4 of each 6-bit field);
// USB OTG_FS GOTGINT @+0x04 (SEDET bit2), GINTSTS @+0x14 (USBRST bit12,
// ENUMDNE bit13, RXFLVL bit4, IEPINT bit18, OEPINT bit19); ITM STIM0-31
// @0xE0000000+n*4 ( drained, no peek needed — take is non-destructive
// when empty).
const EXTI_BASE = 0x40013C00, EXTI_PR = 0x14;
const CAN_BASE = { 1: 0x40006400, 2: 0x40006800 };
const TIM_BASE = {
    1: 0x40010000, 2: 0x40000000, 3: 0x40000400, 4: 0x40000800,
    5: 0x40000C00, 6: 0x40001000, 7: 0x40001400, 8: 0x40010400,
    9: 0x40014000, 10: 0x40014400, 11: 0x40014800, 12: 0x40001800,
    13: 0x40001C00, 14: 0x40002000,
};
const DMA_BASE = { 1: 0x40026000, 2: 0x40026400 };
const USB_FS_BASE = 0x50000000;
const ITM_STIM0 = 0xE0000000;

// ── Platform display surface ("DRM" — dumb raster manager) ─────────────
// Wokwi/OpenHW/Velxio render virtual screens from raw framebuffers; the F4
// model already decodes all three display paths in emulator.js. This class
// is a thin read-only view over the live `emu` handle — no copies except
// the returned typed arrays, no canvas/DOM dependency (works headless).
// - oled: SSD1306 128x64 page-addressed (needs ext_devices.oled at create).
//   fb is 128*64 bytes of 0/1 pixels, row-major (fb[y*128+x]).
// - tft: ILI9341 240x320 RGB565 big-endian (needs ext_devices.tft).
//   fb is 240*320*2 bytes, pixel = (fb[p]<<8)|fb[p+1].
// - ltdc: LTDC layer0 scanout — read live from guest RAM via the layer
//   regs (no ext_devices needed; reads zero when the layer is off).
// All getters return null/0 when the device was not enabled at create.
export class Display {
    constructor(mcu) { this._mcu = mcu; }
    get oled() {
        const o = this._mcu._emu.oled;
        if (!o) return null;
        // NOTE: emu.oled.frame is a FUNCTION (() => count) on the live
        // handle (emulator.js) — call it; tolerate a raw number (mocks).
        return { w: 128, h: 64, fb: o.fb, frame: typeof o.frame === 'function' ? o.frame() : o.frame };
    }
    get tft() {
        const t = this._mcu._emu.tft;
        if (!t) return null;
        return { w: t.w, h: t.h, fb: t.fb, frame: typeof t.frame === 'function' ? t.frame() : t.frame };
    }
    // LTDC layer0: geometry from WHPCR/WVPCR + PFCR/CFBAR/CFBLR, pixels
    // from guest RAM. Supports pf 0 (ARGB8888) + 2 (RGB565). Returns null
    // when the layer is off OR the chip has no LTDC (F401/F411).
    ltdc() {
        if (!this._mcu.chip.ltdc) return null;
        const R = (a) => { try { return this._mcu._emu.read32(a) >>> 0; } catch { return 0; } };
        const LTDC = 0x40016800;
        const gcr = R(LTDC + 0x18), l1cr = R(LTDC + 0x84);
        if (!(gcr & 1) || !(l1cr & 1)) return null; // LTEN + LEN
        const pf = R(LTDC + 0x94) & 7;
        if (pf !== 0 && pf !== 2) return null;
        const wh = R(LTDC + 0x88), wv = R(LTDC + 0x8C);
        const w = (wh & 0xFFF) + 1, h = ((wh >>> 16) & 0xFFF) + 1;
        const cfbar = R(LTDC + 0xAC) >>> 0;
        const cfblr = R(LTDC + 0xB0) >>> 0;
        const pitch = (cfblr >>> 16) || (w * (pf === 0 ? 4 : 2));
        const lineBytes = (cfblr & 0x1FFF) || w * (pf === 0 ? 4 : 2);
        void wv;
        const bpp = pf === 0 ? 4 : 2;
        const fb = new Uint8Array(w * h * bpp);
        for (let y = 0; y < h; y++) {
            for (let x = 0; x < w * bpp; x++) {
                const addr = cfbar + y * pitch + x;
                if (x >= lineBytes) break;
                try { fb[y * w * bpp + x] = this._mcu._emu.read32(addr & ~3) >>> ((addr & 3) * 8) & 0xFF; }
                catch { break; }
            }
        }
        return { w, h, pf: pf === 0 ? 'ARGB8888' : 'RGB565', fb, addr: cfbar, pitch };
    }
}

// F1 `gpio` object parity: GPIO wraps the mcu so `gpio.pin()` matches the
export class GPIO {
    constructor(mcu) { this._mcu = mcu; }
    pin(port, pin) { return this._mcu.gpio.pin(port, pin); }
}

// F1 `spi[ch]` parity: per-bus handle with `onTransfer` + `injectMiso`.
// The underlying F4 tap is create-time (see header): assigning onTransfer
// post-create only observes the pre-declared spec — pass callbacks in the
// `spi: [...]` create opt for real wiring. Kept for call-shape compat.
export class SPI {
    constructor(mcu, ch) {
        this._mcu = mcu;
        this.ch = ch;
        this.onTransfer = null;
    }
    injectMiso(bytes) {
        const peri = `SPI${this.ch}`;
        this._mcu.spi.pushMiso(peri, bytes);
    }
}

// F1 `i2c[ch]` parity: per-bus handle with onStart/onWrite/onRead/onStop +
// `injectRx`. Same create-time caveat as SPI above.
export class I2C {
    constructor(mcu, ch) {
        this._mcu = mcu;
        this.ch = ch;
        this.onStart = null;
        this.onWrite = null;
        this.onRead = null;
        this.onStop = null;
    }
    injectRx(bytes) {
        const peri = `I2C${this.ch}`;
        this._mcu.i2c.pushRx(peri, bytes);
    }
}

export class STM32F4 {
    constructor(emu, bindings = null, chip = null, chipSvdXml = null) {
        this._emu = emu;
        this._bindings = bindings;
        this.chip = chip || chipInfo('stm32f407');
        // Per-chip SVD text (Node: loaded in _create; browser: caller passes
        // svdXml explicitly). Kept for chip-aware helpers (e.g. SVD-verified
        // pin maps) without re-reading files per call.
        this.chipSvdXml = chipSvdXml || null;
        this._spiSpecs = [];
        this._i2cSpecs = [];
        this._pinListeners = new Map();
        this._pinUnsub = null;
        this.gpio = new GPIO(this);
        // Back-compat: gpio.pin was a bare function before the GPIO class.
        // Bank-validated: F401/F411 expose A-F only (SVD census) — a pin on
        // G/K throws a useful error instead of driving a benign-0 hole.
        const _pinFn = (port, pin) => new GPIOPin(this, port, pin);
        const _origPin = this.gpio.pin.bind(this.gpio);
        void _origPin;
        this.gpio.pin = (port, pin) => {
            const p = typeof port === 'string' ? port.toUpperCase()
                : String.fromCharCode(65 + (port | 0));
            const bank = p.charCodeAt(0) - 65;
            const maxBank = (this.chip.gpioBanks || 11) - 1;
            if (p.length !== 1 || bank < 0 || bank > maxBank || (pin | 0) < 0 || (pin | 0) > 15) {
                throw new Error(`gpio.pin('${port}', ${pin}): ${this.chip.key} exposes banks A-${String.fromCharCode(65 + maxBank)}, pins 0-15`);
            }
            return _pinFn(p, pin | 0);
        };
        // Six USART slots; only the chip's SVD-present USARTs are live —
        // the rest are null (F401/F411 have 1,2,6 only). TX is a single
        // shared model buffer: only usart1._emit is fed (see execute());
        // RX injects per-base and works on every live USART.
        const mkUsart = (n) => new USART(this, n);
        for (const n of [1, 2, 3, 4, 5, 6]) {
            this[`usart${n}`] = this.chip.usarts.includes(n) ? mkUsart(n) : null;
        }
        // `usart` stays the USART1 alias (existing callers); the indexed
        // map mirrors F1's `usart: {1,2,3}` shape, extended to 6 (live only).
        this.usart = this.usart1;
        this.usarts = {};
        for (const n of this.chip.usarts) this.usarts[n] = this[`usart${n}`];
        // F1 `uartRx(byte)` / `uartOutput` USART1 shortcuts.
        // ── Wokwi/OpenHW/Velxio integration callbacks (F1 parity) ──
        // Set directly, default null. Dispatched once per execute()/step()
        // from MODEL-READABLE state (no core event queue on F4 — the F1
        // drainEvents discriminants don't exist here). Each poll is a
        // bounded set of read32/take calls; with no callback set the poll
        // is skipped entirely (zero overhead when unused):
        // - onExtiEdge(line): EXTI PR w1c — read, clear, fire per set bit.
        // - onCanTx(can): CAN TSR TXOK/RQCP newly set since last poll.
        // - onCanRx(can,id,len,data): CAN RF0R/RF1R FMP newly arrived;
        //   frame read from the mailbox regs (11-bit id, 8B data; FD/extended
        //   frames report via canInject/canInjectFd paths instead).
        // - onTimUpdate(tim): TIM SR UIF newly set (cleared on dispatch).
        // - onTimCapture(tim,ch,val): TIM SR CCxIF newly set + CCR latch.
        // - onDmaTc(controller, stream): DMA LISR/HISR TCIF newly set.
        // - onWdogReset(which): IWDG(1)/WWDG(2) reset flag newly set
        //   (1=IWDG, 2=WWDG — F1 `which` parity).
        // - onUsbIn(ep,data): USB OTG_FS IN-complete (usb_take_in drains).
        // - onItmByte(port,byte): ITM STIM port drain (printf path).
        // - onFsmcAccess(bank,off,write,size,val): FSMC bank tap drain
        //   (requires ext_devices.fsmcDevices at create — same rule as SPI).
        // Stubs with NO model source stay absent: onAdcDone/onDacWrite/
        // onCrcResult/onRtcAlarm/onHostTx/onHostRx/onI2cAlert (use read32/
        // polling or the bus taps; a fake event would be worse than none).
        this.onExtiEdge = null;
        this.onCanTx = null;
        this.onCanRx = null;
        this.onTimUpdate = null;
        this.onTimCapture = null;
        this.onDmaTc = null;
        this.onWdogReset = null;
        this.onUsbIn = null;
        this.onItmByte = null;
        this.onFsmcAccess = null;
        this._evLast = {
            extiPr: 0, canTsr: { 1: 0, 2: 0 }, canFmp: { 1: [0, 0], 2: [0, 0] },
            timSr: {}, dmaLisr: { 1: 0, 2: 0 },
            wdog: 0, usbIn: {},
        };
        this.dma = {
            stream: (index) => new DMAStream(this, index),
            controller: (ctl) => new DMAController(this, ctl),
        };
        // F1 `spi1..3` / `i2c1..3` (+ indexed maps) parity — call-shape
        // compat only (see SPI/I2C classes above for the create-time caveat).
        // All three buses exist on every F4 chip in the table (SVD census).
        this.spi1 = new SPI(this, 1);
        this.spi2 = new SPI(this, 2);
        this.spi3 = new SPI(this, 3);
        this.spiBus = { 1: this.spi1, 2: this.spi2, 3: this.spi3 };
        this.i2c1 = new I2C(this, 1);
        this.i2c2 = new I2C(this, 2);
        this.i2c3 = new I2C(this, 3);
        this.i2cBus = { 1: this.i2c1, 2: this.i2c2, 3: this.i2c3 };
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
        // Platform display surface ("DRM"): live OLED/TFT/LTDC framebuffers
        // for Wokwi/OpenHW/Velxio screen widgets (see the Display class).
        this.display = new Display(this);
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
        const chip = resolveChip(opts);
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
        // Chip-resolved emulator sizing + SVD + map identity. Explicit opts
        // win (a caller passing svdXml/flash_size overrides the table); the
        // default SVD text is loaded per chip here so `chip` alone is
        // enough — index.mjs passes its own svdXml (F407), which is why the
        // default only applies when the caller did NOT supply one. The chip
        // hint drives the model's DBGMCU IDCODE via init_svd_chip.
        const needSvd = !opts.svdXml;
        let chipSvdXml = null;
        if (needSvd) {
            try {
                const mod = await import('node:fs');
                chipSvdXml = mod.readFileSync(
                    new URL(`./vendor/${chip.svd}`, import.meta.url), 'utf8');
            } catch { chipSvdXml = null; } // browser: caller must pass svdXml
        }
        const emu = await createEmulator({
            flash_size: chip.flash_size, ram_size: chip.ram_size,
            chipHint: chip.svd.replace(/\.svd$/, ''),
            ...(chipSvdXml ? { svdXml: chipSvdXml } : {}),
            ...opts, firmware, ext_devices,
        });
        const mcu = new STM32F4(emu, bindings, chip, chipSvdXml || opts.svdXml || null);
        mcu._spiSpecs.push(...(opts.spi || []));
        mcu._i2cSpecs.push(...(opts.i2c || []));
        return mcu;
    }

    // F1 factory parity: fromELF/fromBin/fromHex (firmware image + opts).
    static async fromELF(buf, opts = {}) { return STM32F4.create({ ...opts, firmware: buf }); }
    static async fromBin(buf, opts = {}) { return STM32F4.create({ ...opts, firmware: buf }); }
    static async fromHex(text, opts = {}) {
        const { flash } = parseIntelHex(text);
        if (!flash) throw new Error('fromHex: no flash image in HEX text');
        return STM32F4.create({ ...opts, firmware: flash });
    }

    // ── firmware loading (chip-sized flash image) ──
    // loadBin/loadELF size against the CHIP's flash (not the 1M F407
    // default): a 2M F429 image on a 512K F401 throws instead of silently
    // truncating. loadHex/loadELF pass the flash through the chip-sized
    // window too (RAM segments ride extraMem, sized by the parser).
    loadBin(bytes, base = FLASH_BASE) {
        const buf = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
        const flash = buildFlashImage(buf, base, this.chip.flash_size);
        this._emu.loadImage({ flash });
    }
    loadHex(text) {
        const { flash, ram } = parseIntelHex(text, this.chip.flash_size, this.chip.ram_size);
        const extraMem = [];
        if (ram) extraMem.push({ addr: RAM_BASE, data: ram });
        this._emu.loadImage({ flash: flash || new Uint8Array(0), extraMem });
    }
    loadELF(bytes) {
        const { flash, extraMem } = parseElf(bytes, this.chip.flash_size, this.chip.ram_size);
        this._emu.loadImage({ flash: flash || new Uint8Array(0), extraMem });
    }

    // ── execution ──
    // Both step the core, drain UART into usart1, then dispatch the
    // peripheral-event poll. Returns F1-shaped results.
    execute(cycles = 100000) {
        const r = this._emu.step(cycles);
        this._drainUartToUsart1();
        this._pollEvents();
        return { instCount: r.instCount, stopped: r.stopped };
    }
    step(cycles = 100000) {
        const r = this._emu.step(cycles);
        this._drainUartToUsart1();
        this._pollEvents();
        return { pc: r.pc, instCount: r.instCount, stopped: r.stopped };
    }
    _drainUartToUsart1() {
        const out = this._emu.drainUart();
        if (!out) return;
        if (typeof out === 'string') {
            for (let i = 0; i < out.length; i++) this.usart1._emit(out.charCodeAt(i));
        } else {
            for (let i = 0; i < out.length; i++) this.usart1._emit(out[i]);
        }
    }
    // ── peripheral-event dispatch (one bounded poll per execute/step) ──
    // Safe read: model reads never throw on mapped regs; guard anyway so a
    // callback can never break stepping.
    _evRead(addr) {
        try { return this._emu.read32(addr) >>> 0; } catch { return 0; }
    }
    _pollEvents() {
        const L = this._evLast, B = this._bindings, E = this._emu;
        // EXTI: PR is w1c — read pending, clear, fire per line.
        if (this.onExtiEdge) {
            const pr = this._evRead(EXTI_BASE + EXTI_PR) & 0x7FFFFF;
            if (pr) {
                try { E.write32(EXTI_BASE + EXTI_PR, pr); } catch {}
                for (let line = 0; line < 23; line++) {
                    if (pr & (1 << line)) {
                        try { this.onExtiEdge(line); } catch {}
                    }
                }
            }
        }
        // CAN TX: TSR TXOK/RQCP bits newly set since last poll. Skipped
        // for CAN-less chips (F401/F411 have no CAN silicon — the poll
        // would read benign-0 holes forever).
        if (this.onCanTx) {
            for (const can of this.chip.can) {
                const tsr = this._evRead(CAN_BASE[can] + 0x08);
                const newly = (tsr & 0x070707) & ~L.canTsr[can];
                L.canTsr[can] = tsr & 0x070707;
                if (newly) {
                    try { this.onCanTx(can); } catch {}
                }
            }
        }
        // CAN RX: FMP level (not edge) per FIFO; read id/len/data from the
        // mailbox (slot 0: RIR @+0x1B0, RDTR @+0x1B4, RDLR @+0x1B8,
        // RDHR @+0x1BC). 11-bit id = RIR>>21; len = RDTR&0xF. Fires while
        // FMP>0 and the mailbox changed since the last fire (guest may
        // hold the frame for many polls before RFOM-releasing it).
        if (this.onCanRx) {
            for (const can of this.chip.can) {
                for (let fifo = 0; fifo < 2; fifo++) {
                    const fmp = this._evRead(CAN_BASE[can] + 0x0C + fifo * 4) & 0x3;
                    if (fmp === 0) { L.canFmp[can][fifo] = null; continue; }
                    const slot = fifo === 0 ? 0 : 3;
                    const base = 0x1B0 + slot * 0x10;
                    const rir = this._evRead(CAN_BASE[can] + base);
                    const rdtr = this._evRead(CAN_BASE[can] + base + 4);
                    const rdlr = this._evRead(CAN_BASE[can] + base + 8);
                    const rdhr = this._evRead(CAN_BASE[can] + base + 12);
                    const key = `${rir.toString(16)}:${rdtr.toString(16)}:${rdlr.toString(16)}:${rdhr.toString(16)}`;
                    if (L.canFmp[can][fifo] === key) continue; // same frame: already fired
                    L.canFmp[can][fifo] = key;
                    const id = (rir >>> 21) & 0x7FF;
                    const len = rdtr & 0xF;
                    const data = [
                        rdlr & 0xFF, (rdlr >>> 8) & 0xFF, (rdlr >>> 16) & 0xFF, (rdlr >>> 24) & 0xFF,
                        rdhr & 0xFF, (rdhr >>> 8) & 0xFF, (rdhr >>> 16) & 0xFF, (rdhr >>> 24) & 0xFF,
                    ].slice(0, len);
                    try { this.onCanRx(can, id, len, data); } catch {}
                }
            }
        }
        // TIM update + capture: SR UIF/CCxIF newly set. UIF/CCxIF are w1c
        // on silicon — clear on dispatch so each edge fires once. Only
        // the chip's SVD-present timers are polled (F401/F411 lack
        // 6,7,12,13,14 — those addresses are benign-0 holes there).
        if (this.onTimUpdate || this.onTimCapture) {
            for (const tim of this.chip.timers) {
                const base = TIM_BASE[tim];
                if (base === undefined) continue;
                const sr = this._evRead(base + 0x10) & 0x1F;
                const last = L.timSr[tim] || 0;
                const newly = sr & ~last;
                L.timSr[tim] = sr;
                if (newly & 1) {
                    try { E.write32(base + 0x10, sr & ~1); L.timSr[tim] &= ~1; } catch {}
                    if (this.onTimUpdate) { try { this.onTimUpdate(tim); } catch {} }
                }
                if (this.onTimCapture) {
                    for (let ch = 0; ch < 4; ch++) {
                        if (newly & (2 << ch)) {
                            const ccr = this._evRead(base + 0x34 + ch * 4) & 0xFFFF;
                            try { E.write32(base + 0x10, (L.timSr[tim] &= ~(2 << ch))); } catch {}
                            try { this.onTimCapture(tim, ch, ccr); } catch {}
                        }
                    }
                }
            }
        }
        // DMA terminal-count: LISR/HISR TCIF per stream newly set.
        if (this.onDmaTc) {
            for (const ctl of [1, 2]) {
                const lisr = this._evRead(DMA_BASE[ctl]) >>> 0;
                const hisr = this._evRead(DMA_BASE[ctl] + 4) >>> 0;
                let cur = 0;
                for (let s = 0; s < 4; s++) {
                    if (lisr & (1 << (s * 6 + 4))) cur |= 1 << s;
                    if (hisr & (1 << (s * 6 + 4))) cur |= 1 << (s + 4);
                }
                const prev = L.dmaLisr[ctl] & 0xFF;
                const fresh = cur & ~prev;
                L.dmaLisr[ctl] = cur;
                for (let s = 0; s < 8; s++) {
                    if (fresh & (1 << s)) { try { this.onDmaTc(ctl, s); } catch {} }
                }
            }
        }
        // Watchdog reset: latch flags newly set (1=IWDG, 2=WWDG).
        if (this.onWdogReset) {
            let cur = 0;
            try {
                if (B && typeof B.iwdg_reset_flag === 'function' && B.iwdg_reset_flag()) cur |= 1;
                if (B && typeof B.wwdg_reset_flag === 'function' && B.wwdg_reset_flag()) cur |= 2;
            } catch {}
            const fresh = cur & ~L.wdog;
            L.wdog = cur;
            if (fresh & 1) { try { this.onWdogReset(1); } catch {} }
            if (fresh & 2) { try { this.onWdogReset(2); } catch {} }
        }
        // USB IN-complete: drain per-EP IN blobs (non-destructive when empty).
        if (this.onUsbIn && B && typeof B.usb_take_in === 'function') {
            for (let ep = 0; ep < 4; ep++) {
                let blob = null;
                try { blob = B.usb_take_in(ep); } catch { blob = null; }
                if (blob && blob.length) {
                    try { this.onUsbIn(ep, Array.from(blob)); } catch {}
                }
            }
        }
        // ITM stimulus ports: drain queued printf bytes.
        if (this.onItmByte && B && typeof B.itm_take_port === 'function') {
            for (let port = 0; port < 32; port++) {
                let pending = 0;
                try {
                    pending = (typeof B.itm_port_pending === 'function')
                        ? B.itm_port_pending(port) : 1;
                } catch { pending = 0; }
                if (!pending) continue;
                let blob = null;
                try { blob = B.itm_take_port(port); } catch { blob = null; }
                if (blob && blob.length) {
                    for (const b of blob) { try { this.onItmByte(port, b & 0xFF); } catch {} }
                }
            }
        }
        // FSMC bank taps: drain access events (needs fsmcDevices at create).
        if (this.onFsmcAccess && typeof E.takeFsmcEvents === 'function') {
            for (let bank = 0; bank < 4; bank++) {
                let ev = null;
                try { ev = E.takeFsmcEvents(bank); } catch { ev = null; }
                if (!ev || !ev.length) continue;
                for (let i = 0; i + 1 < ev.length; i += 2) {
                    const hdr = ev[i] >>> 0, val = ev[i + 1] >>> 0;
                    const write = (hdr & 0x80000000) !== 0;
                    const off = hdr & 0x7FFFFFFF;
                    try { this.onFsmcAccess(bank, off, write, 4, val); } catch {}
                }
            }
        }
    }
    // F1 `uartRx(byte)` / `uartOutput` USART1 shortcuts.
    uartRx(byte) { this.usart1.sendData([byte & 0xFF]); return true; }
    get uartOutput() {
        let s = '';
        for (const b of this.usart1._buf) s += String.fromCharCode(b);
        return s;
    }
    // Pure-JS map-symbol helpers (F1 setSymbols/resolveSymbol parity for
    // the symbols parseMap already provides; no model symbol table on F4).
    setSymbols(mapText) {
        this._symbols = parseMap(mapText);
        return this._symbols.length;
    }
    resolveSymbol(pc) {
        const syms = this._symbols || [];
        let best = null;
        for (const s of syms) {
            if (s.addr <= pc && (!best || s.addr > best.addr)) best = s;
        }
        if (!best) return null;
        const off = (pc - best.addr) >>> 0;
        return off ? `${best.name}+0x${off.toString(16)}` : best.name;
    }

    // ── DMA controller (Wokwi/OpenHW/Velxio integration) ──
    // Full controller view — see the DMAController class above.
    dmaController(ctl = 1) { return new DMAController(this, ctl); }

    // ── ADC: live channel injection (anytime, no init constraint) ──
    // The model keeps a global override table (unlike the SPI/I2C taps);
    // without an override the channel reads the synthetic default
    // (16/17 = temp/Vref, 18 = Vbat, else pseudo-random). Verified by the
    // Potentiometer component + site/test_component_adc.mjs.
    setAdcChannel(peripheral, channel, value) {
        this._bindings.adc_set_channel_value(peripheral, channel, value);
    }
    clearAdcChannel(peripheral, channel) {
        this._bindings.adc_clear_channel_value(peripheral, channel);
    }
    // Drain ADC samples staged by EOC-triggered DMA requests (CR2 DMA bit).
    takeAdcDma() {
        try { return Array.from(this._bindings.adc_take_dma()); }
        catch { return []; }
    }

    // ── CAN: host-side frame injection (anytime) ──
    // Standard 11-bit frames; gated on chip CAN silicon (F401/F411 throw
    // a useful error instead of sinking into a benign-0 hole).
    canInject(id, dlc, data) {
        if (!this.chip.can.length) {
            throw new Error(`canInject: ${this.chip.key} has no CAN silicon (SVD census)`);
        }
        this._emu.canInject(id & 0x7FF, dlc & 0xF, data);
    }
    canInjectFd(id, data, brs = false) {
        if (!this.chip.can.length) {
            throw new Error(`canInjectFd: ${this.chip.key} has no CAN silicon (SVD census)`);
        }
        this._bindings.can_inject_fd(id >>> 0, new Uint8Array(data), !!brs);
    }

    // ── TIM: edge injection + PWM readback (anytime) ──
    // The emulator has no external signal source, so a TIx edge is a host
    // call (mirrors the model export; firmware polls CCR like tim_capture_demo).
    timInjectCapture(timer, ch) {
        if (!this.chip.timers.includes(timer | 0)) {
            throw new Error(`timInjectCapture: TIM${timer} absent on ${this.chip.key} (SVD census)`);
        }
        this._emu.timInjectCapture(`TIM${timer | 0}`, ch & 0x3);
    }
    // PWM pulse width in µs for timer/channel at a clock rate (model probe;
    // pass the timer clock, e.g. 84e6 for APB1 — same basis as Pwm).
    timPwmPulseUs(timer, ch, clockHz) {
        return this._bindings.tim_pwm_pulse_us(`TIM${timer}`, ch, clockHz);
    }
    timOcMode(timer, ch) {
        return this._bindings.tim_oc_mode(`TIM${timer}`, ch);
    }

    // ── DAC: hardware trigger + underrun (anytime) ──
    // Gated on chip DAC silicon (F401/F411 throw instead of sinking).
    dacTrigger(ch, src, dmaStaged = false) {
        if (!this.chip.dac) {
            throw new Error(`dacTrigger: ${this.chip.key} has no DAC silicon (SVD census)`);
        }
        this._bindings.dac_hw_trigger(ch, src, !!dmaStaged);
    }
    dacUnderrun(ch) {
        if (!this.chip.dac) return false;
        try { return !!this._bindings.dac_underrun(ch); } catch { return false; }
    }

    // ── I2S audio capture drain (anytime) ──
    // DR writes by the guest land in the model capture FIFO; the speaker
    // device (ext_devices.speaker) drains it per step, but a platform can
    // also drain directly here (returns Float32 samples).
    takeSpeakerSamples() {
        try { return this._emu.takeSpeakerSamples(); }
        catch { return new Float32Array(0); }
    }

    // ── USB OTG_FS host calls (anytime; harness-driven like the model) ──
    // SETUP/OUT inject into EP0/endpoints; takeIn drains device-to-host IN
    // blobs; reset/enumerated drive the device state machine. HS twins
    // exist on the model (usb_hs_*) but the facade targets FS (all chips
    // have FS silicon; HS is F407/F429-only and needs its own window).
    usbInjectSetup(bytes) {
        return !!this._bindings.usb_inject_setup(new Uint8Array(bytes));
    }
    usbInjectOut(ep, bytes) {
        return !!this._bindings.usb_inject_out(ep, new Uint8Array(bytes));
    }
    usbTakeIn(ep) {
        try { return Array.from(this._bindings.usb_take_in(ep)); }
        catch { return []; }
    }
    usbReset() { try { this._bindings.usb_reset(); } catch {} }
    usbEnumerated() { try { this._bindings.usb_enumerated(); } catch {} }

    // ── ITM stimulus drain (anytime) ──
    // Port 0 sinks to the UART console in the model; ports 1-31 queue
    // per-port streams. takeItm drains one port (empty when idle).
    takeItm(port) {
        try { return Array.from(this._bindings.itm_take_port(port)); }
        catch { return []; }
    }
    itmPending(port) {
        try { return this._bindings.itm_port_pending(port) >>> 0; }
        catch { return 0; }
    }

    // ── FSMC bank taps (needs ext_devices.fsmcDevices at create) ──
    // Same create-time rule as SPI/I2C (Fsmc binds banks once at
    // construction). takeFsmc drains access events; pushFsmc answers reads.
    takeFsmc(bank) {
        try { return Array.from(this._emu.takeFsmcEvents(bank)); }
        catch { return []; }
    }
    pushFsmc(bank, values) {
        try { this._emu.pushFsmcData(bank, values); } catch {}
    }

    // ── I2C register-file devices (DS3231 RTC style, anytime) ──
    // The file must be registered at create via ext_devices.regfile (or
    // the ext_devices.rtc shorthand, which also enables emu.rtc decode).
    // After that, get/set run any time (the RTC panel reads live).
    regfileGet(peripheral, offset) {
        return this._bindings.i2c_regfile_get(peripheral, offset);
    }
    regfileSet(peripheral, offset, value) {
        this._bindings.i2c_regfile_set(peripheral, offset, value & 0xFF);
    }

    // ── Live device views (same handles the browser panels read) ──
    // Null unless the matching ext_devices entry enabled the device at
    // create. oled/tft/rtc/buzzer decode in emulator.js; camera feeds the
    // DCMI sensor (feed anytime, stop/start the ext_devices.camera source).
    get oled() { return this._emu.oled; }
    get tft() { return this._emu.tft; }
    get rtc() { return this._emu.rtc; }
    get buzzer() { return this._emu.buzzer; }
    get camera() { return this._emu.camera; }

    // ── engine access ──
    read32(addr) { return this._emu.read32(addr); }
    write32(addr, val) { return this._emu.write32(addr, val); }
    memRead32(addr) { return this._emu.read32(addr); }
    memWriteBytes(addr, bytes) {
        const arr = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
        // Probe-style write (F1 memWriteBytes parity): the wuc shim owns
        // cpu.mem_write (bypasses flash protection + MPU, like a probe).
        // wuc.mem_write enforces the mapped-range check — out-of-range
        // throws, same as the edge-case contract.
        const wuc = this._emu.uc;
        if (wuc && typeof wuc.mem_write === 'function') {
            wuc.mem_write(addr >>> 0, arr);
            return;
        }
        for (let i = 0; i < arr.length; i++) {
            const w = addr & ~3, off = addr & 3;
            let word = 0;
            try { word = this._emu.read32(w) >>> 0; } catch { word = 0xFFFFFFFF; }
            const sh = off * 8;
            word = ((word & ~(0xFF << sh)) | ((arr[i] & 0xFF) << sh)) >>> 0;
            this._emu.write32(w, word);
        }
    }
    periphRead(addr, width) {
        void width;
        return this._emu.read32(addr);
    }
    periphWrite(addr, width, value) {
        void width;
        return this._emu.write32(addr, value);
    }
    getRegisters() { return this._emu.getRegisters(); }
    getPc() { return this._emu.getRegisters().PC; }
    getSp() { return this._emu.getRegisters().SP; }
    faultInfo() {
        if (typeof this._emu.faultInfo === 'function') return this._emu.faultInfo();
        return null;
    }
    takeFault() {
        const f = this.faultInfo();
        return f ? [f.pc >>> 0, 0] : null;
    }
    // ── F1 debug-port parity (honest shim — see header) ──
    // No SWD/JTAG DP exists on the F4 model (no DHCSR/MEM-AP registers).
    // These give OpenHW/Velxio-shaped call points with documented behavior:
    swdHalted() { return false; }
    swdHalt() { /* no halt path: steps are bounded round-trips */ }
    swdResume() { /* no-op (never halted) */ }
    swdStep() { this.step(1); return 1; }
    swdAddWatch(kind, addr, len) {
        void kind; void len;
        return this._ensureWatchpoints().length;
    }
    swdRemoveWatch(slot) { void slot; }
    swdTakeTrip() { return []; }
    swdDpRead(addr) { void addr; return 0; }
    swdDpWrite(addr, value) { void addr; void value; }
    swdApRead(bank, reg) { void bank; void reg; return 0; }
    swdApWrite(bank, reg, value) { void bank; void reg; void value; }
    // DCRSR-style core register access (0-12, 13 SP, 14 LR, 15 PC, 16 xPSR).
    swdRegRead(idx) {
        const r = this._emu.getRegisters();
        if (idx >= 0 && idx <= 12) return r[`R${idx}`] >>> 0;
        if (idx === 13) return r.SP >>> 0;
        if (idx === 14) return r.LR >>> 0;
        if (idx === 15) return r.PC >>> 0;
        if (idx === 16) return r.XPSR >>> 0;
        return 0;
    }
    swdRegWrite(idx, value) { void idx; void value; /* read-only shim */ }
    jtagReset() { /* no TAP on the F4 model */ }
    jtagIr(ir) { void ir; }
    jtagIdcode() { return this._evRead(0xE0042000) >>> 0; }
    jtagDp(addr, rnw, wdata) { void addr; void rnw; void wdata; return 0; }
    jtagAp(bank, reg, rnw, wdata) { void bank; void reg; void rnw; void wdata; return 0; }
    _ensureWatchpoints() {
        if (!this._watchpoints) this._watchpoints = [];
        return this._watchpoints;
    }
    // F1 `addJsPeripheral(base, size, read, write)` parity: F4 has no
    // model-side MMIO hook table, so this is a JS-side shim — reads/writes
    // go through periphRead/periphWrite only when the guest has no mapping
    // there is no mapping check available, so the shim records the region
    // and exposes jsPeripheralRead/Write helpers the platform calls
    // explicitly. Documented as a shim (not silicon).
    addJsPeripheral(base, size, read, write) {
        const list = this._ensureWatchpoints();
        const entry = { base: base >>> 0, size: size >>> 0, read, write, js: true };
        list.push(entry);
        return true;
    }
    // ── F1 power parity (estimate from PWR_CR + per-chip clock) ──
    // No calibrated current model on F4 (F1's pwrEstimate is DS5319-based).
    // Returns a documented ROUGH estimate in µA from the PWR_CR LPDS/PDDS
    // bits, scaled by the chip's max clock (RUN current ~ linear in MHz).
    pwrMode() {
        let cr = 0;
        try { cr = this._emu.read32(0x40007000) >>> 0; } catch {}
        if (cr & 0x02) return 3; // PDDS: STANDBY
        if (cr & 0x01) return 2; // LPDS: STOP (approx)
        return 0; // RUN (SLEEP-via-WFI is transient — not latched)
    }
    pwrEstimate() {
        const mode = this.pwrMode();
        if (mode === 3) return 4; // STANDBY ~4 µA class (all chips)
        if (mode === 2) return 400; // STOP ~0.4 mA class (all chips)
        // RUN scales with clock: 168 MHz F407 ~30 mA class.
        return Math.round(30000 * (this.chip.maxClockMHz / 168));
    }
    stop() { return this._emu.stop(); }
    reset() { return this._emu.reset(); }
    close() { return this._emu.close(); }
    // Host reset/boot control (real-device Reset button semantics — see
    // emulator.js): resetCpu() re-runs the vector table with peripherals
    // kept; setNrst(true/false) holds/releases the NRST line (steps become
    // clock-only no-ops while held); bootPreset({flash, extraMem}) reloads
    // the image then resets. All degrade gracefully on older handles.
    resetCpu() {
        if (typeof this._emu.resetCpu === 'function') return this._emu.resetCpu();
        return this._emu.reset();
    }
    setNrst(asserted) {
        if (typeof this._emu.setNrst === 'function') return this._emu.setNrst(asserted);
        return false;
    }
    isNrstAsserted() {
        if (typeof this._emu.isNrstAsserted === 'function') return this._emu.isNrstAsserted();
        return false;
    }
    bootPreset(image) {
        if (typeof this._emu.bootPreset === 'function') return this._emu.bootPreset(image);
        return this._emu.loadImage(image);
    }
    // Board LED readout: { bank, pin, label, on, moder } for the board's
    // on-board LED. `boardKey` defaults to this instance's chip (so a
    // `chip: 'stm32f401'` facade answers PC13 without being told twice);
    // an explicit key still wins, and Nucleo fw-name aliases route PA5.
    // `on` is the guest-driven ODR level; `output` is whether MODER has the
    // pin as output (false before the firmware configures it).
    ledStatus(fwName, boardKey) {
        const led = this._ledFor(fwName, boardKey || (this.chip && this.chip.key));
        const moder = this._emu.read32(0x40020000 + led.bank * 0x400) >>> 0;
        const odr = this._emu.read32(0x40020000 + led.bank * 0x400 + 0x14) >>> 0;
        const mode = (moder >>> (led.pin * 2)) & 3;
        return { bank: led.bank, pin: led.pin, label: led.label, on: ((odr >>> led.pin) & 1) !== 0, output: mode === 1, moder };
    }
    _ledFor(fwName, boardKey) {
        const aliases = { blinky_nucleo_f401: { bank: 0, pin: 5, label: 'PA5' }, blinky_nucleo_f411: { bank: 0, pin: 5, label: 'PA5' } };
        if (fwName && aliases[fwName]) return aliases[fwName];
        const map = {
            stm32f401: { bank: 2, pin: 13, label: 'PC13' },
            stm32f411: { bank: 2, pin: 13, label: 'PC13' },
            stm32f407: { bank: 3, pin: 12, label: 'PD12' },
            stm32f407ve: { bank: 0, pin: 6, label: 'PA6' },
            stm32f429: { bank: 6, pin: 13, label: 'PG13' },
        };
        return map[boardKey] || map.stm32f407;
    }
}
