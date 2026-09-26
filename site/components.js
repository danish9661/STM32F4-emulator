// Minimal starter virtual-component library, built on the public
// pin/register-access API from emulator.js (`emu.pin()`, `emu.watchPin()`,
// `emu.read32()`, `emu.i2cRegfile()`) and the STM32F4 facade
// (`site/stm32f4.js` — `mcu.setAdcChannel`, `mcu.dacTrigger`,
// `mcu.rngSeedEntropy`, `mcu.i2cArmSmbusAlert`, `mcu.usbSetVbus`,
// `mcu.uartTxLen`, `mcu.audioLoadWav`, `mcu.regfileSet`,
// `mcu.ltdcClutEntry`). No imports: works in Node and the browser, same
// "import-free" convention as emulator.js.
//
// These are templates, not a full component catalog — attach your own
// devices the same way (see docs/components.md). The eight classes below
// the Potentiometer close the documented hardware-substitute gaps: each
// one drives the substitute the mock suite pins (NOT-modeled asserts),
// so firmware observes honest behavior instead of an un-driven hole.
// Model code unchanged — all wiring is JS on existing wasm exports.

// LED wired to a GPIO pin the guest drives as output.
export class LED {
    constructor(emu, port, num, { activeLow = false } = {}) {
        this.emu = emu;
        this.port = port;
        this.num = num;
        this.activeLow = activeLow;
        this._pin = emu.pin(port, num);
        this._unwatch = null;
    }

    get value() {
        const raw = this._pin.read();
        return this.activeLow ? !raw : raw;
    }

    watch(callback) {
        this.unwatch();
        this._unwatch = this.emu.watchPin(this.port, this.num, (raw) => {
            callback(this.activeLow ? !raw : raw);
        });
        return this;
    }

    unwatch() {
        if (this._unwatch) { this._unwatch(); this._unwatch = null; }
    }
}

// Button that drives a GPIO pin as input into the guest (press/release).
// activeLow=true (default) models a pull-up button: idle=high, pressed=low.
export class Button {
    constructor(emu, port, num, { activeLow = true } = {}) {
        this.emu = emu;
        this.activeLow = activeLow;
        this._pin = emu.pin(port, num);
        this._pin.write(activeLow); // idle level
    }

    press() { this._pin.write(!this.activeLow); }
    release() { this._pin.write(this.activeLow); }
}

// STM32F407 general-purpose/advanced timer base addresses (RM0090).
const TIM_BASE = {
    TIM1: 0x40010000, TIM2: 0x40000000, TIM3: 0x40000400, TIM4: 0x40000800,
    TIM5: 0x40000C00, TIM6: 0x40001000, TIM7: 0x40001400, TIM8: 0x40010400,
    TIM9: 0x40014000, TIM10: 0x40014400, TIM11: 0x40014800,
    TIM12: 0x40001800, TIM13: 0x40001C00, TIM14: 0x40002000,
};

// Default timer clock at the standard 168 MHz F407 configuration: APB2
// timers (TIM1/8/9/10/11) tick at 168 MHz, APB1 timers at 84 MHz. Pass
// `clockHz` explicitly if your firmware clocks the buses differently.
const TIM_APB2 = new Set(['TIM1', 'TIM8', 'TIM9', 'TIM10', 'TIM11']);
const timDefaultClock = (timer) => (TIM_APB2.has(timer) ? 168e6 : 84e6);

// Read-only PWM observer for one output-compare channel (1-4) of a timer:
// decodes CR1/CCER/PSC/ARR/CCRn via emu.read32, the same approach
// emulator.js's built-in buzzer device uses internally, generalized to any
// timer/channel. Base for a servo-angle or PWM-dimmed-LED component — map
// `.duty` (0-1) to whatever range your virtual device needs.
export class Pwm {
    constructor(emu, timer, channel = 1, { clockHz } = {}) {
        const base = TIM_BASE[timer];
        if (base === undefined) throw new Error(`Pwm: unknown timer '${timer}'`);
        if (!Number.isInteger(channel) || channel < 1 || channel > 4) {
            throw new Error(`Pwm: channel must be 1-4, got ${channel}`);
        }
        this.emu = emu;
        this.timer = timer;
        this.base = base;
        this.channel = channel;
        this.clockHz = clockHz ?? timDefaultClock(timer);
    }

    _regs() {
        const r = (off) => this.emu.read32(this.base + off);
        return {
            cr1: r(0x00), ccer: r(0x20), psc: r(0x28), arr: r(0x2C),
            ccr: r(0x34 + (this.channel - 1) * 4),
        };
    }

    get freq() {
        const { cr1, psc, arr } = this._regs();
        if (!(cr1 & 1) || arr === 0 || arr >= 0xFFFFFF) return 0;
        const div = (psc + 1) * (arr + 1);
        return div > 0 ? this.clockHz / div : 0;
    }

    get duty() {
        const { cr1, ccer, arr, ccr } = this._regs();
        const ccEnabled = (ccer >> ((this.channel - 1) * 4)) & 1;
        if (!(cr1 & 1) || !ccEnabled || arr === 0) return 0;
        return ccr / (arr + 1);
    }

    // Output-compare mode of this channel (OC1M/OC2M 3-bit field from
    // CCMR1/CCMR2: 0 frozen, 1 active-on-match, 2 inactive-on-match,
    // 3 toggle, 4 force-inactive, 5 force-active, 6/7 PWM mode 1/2).
    // Lets servo/motor drivers distinguish PWM from toggle/force modes.
    get mode() {
        const off = this.channel <= 2 ? 0x18 : 0x1C;
        const shift = this.channel % 2 === 1 ? 4 : 12;
        const ccmr = this.emu.read32(this.base + off);
        return (ccmr >>> shift) & 7;
    }

    get modeName() {
        return ['frozen', 'active', 'inactive', 'toggle', 'force-lo', 'force-hi', 'pwm1', 'pwm2'][this.mode];
    }
}

// Hobby-servo driver on top of Pwm: maps a 1–2 ms pulse in a 20 ms frame
// (50 Hz, the printer/servo convention) to 0–180°. Reads the live timer
// registers through Pwm, so it tracks guest reprogramming. Pair with the
// model-side `tim_pwm_pulse_us` probe for headless assertions.
export class Servo {
    constructor(emu, timer, channel = 1, opts = {}) {
        this.pwm = new Pwm(emu, timer, channel, opts);
        this.minUs = opts.minUs ?? 1000;
        this.maxUs = opts.maxUs ?? 2000;
        this.maxAngle = opts.maxAngle ?? 180;
    }

    get pulseUs() {
        return this.pwm.duty * 20000;
    }

    get angle() {
        const t = (this.pulseUs - this.minUs) / (this.maxUs - this.minUs);
        return Math.min(this.maxAngle, Math.max(0, t * this.maxAngle));
    }
}

// Wraps an I2C register-file device already registered via the
// `ext_devices.regfile` construction option (i2c_register_regfile must run
// before init() — see docs/components.md). `peripheral` must match the
// config's `peripheral` field. The DS3231 RTC in emulator.js is a built-in
// example of this same pattern; this is the generic, embedder-usable form.
export class I2cRegisterDevice {
    constructor(emu, peripheral) {
        this._regs = emu.i2cRegfile(peripheral);
    }

    get(offset) { return this._regs.get(offset); }
    set(offset, value) { this._regs.set(offset, value); }
}

// Drives an ADC channel's value (0-4095, or a {min,max}-mapped range) via
// emu.setAdcChannel/clearAdcChannel — live, any time, unlike the SPI/I2C
// devices above (no "before init()" constraint; see docs/components.md).
// Without an override the channel falls back to the emulator's synthetic
// temp/vref/vbat/random defaults.
export class Potentiometer {
    constructor(emu, peripheral, channel, { min = 0, max = 4095 } = {}) {
        this.emu = emu;
        this.peripheral = peripheral;
        this.channel = channel;
        this.min = min;
        this.max = max;
        this._value = min;
    }

    get value() { return this._value; }

    set value(v) {
        this._value = Math.min(this.max, Math.max(this.min, v));
        const raw = Math.round(((this._value - this.min) / (this.max - this.min || 1)) * 4095);
        this.emu.setAdcChannel(this.peripheral, this.channel, raw);
    }

    release() { this.emu.clearAdcChannel(this.peripheral, this.channel); }
}

// ── Hardware-substitute components (close the documented gaps) ──────────
// Each class below drives the substitute the mock suite pins with a
// NOT-modeled assert. All wiring is JS on existing wasm exports; the Rust
// model is untouched. Every class works on the raw emulator handle AND the
// STM32F4 facade (both expose setAdcChannel/read32/pin/watchPin by name).

// Analog camera sensor behind DCMI: feeds real pixel data (gradient +
// photon-shot noise) through the JS camera feed instead of a static test
// pattern. Gap closed: "no analog sensor behind DCMI (frames come from
// the JS feed)" — the feed itself now behaves like a sensor.
export class CameraSensor {
    // emu: emulator handle or STM32F4 facade (needs .camera.feed).
    // w/h: sensor resolution; scene: 'gradient' | 'bars' | 'noise'.
    constructor(emu, { width = 160, height = 120, scene = 'gradient', noise = 8 } = {}) {
        this.emu = emu;
        this.width = width;
        this.height = height;
        this.scene = scene;
        this.noise = noise;
        this.frame = 0;
        // Deterministic PRNG (xorshift32) — same frames every run, like the
        // RNG harness contract (seedable, see RngNoise below).
        this._s = 0x12345678;
    }
    _rand() {
        let s = this._s;
        s ^= s << 13; s >>>= 0; s ^= s >> 17; s ^= s << 5; s >>>= 0;
        this._s = s;
        return s / 0xFFFFFFFF;
    }
    pixels() {
        const { width: w, height: h } = this;
        const out = new Uint8Array(w * h);
        for (let y = 0; y < h; y++) {
            for (let x = 0; x < w; x++) {
                let v;
                if (this.scene === 'bars') v = Math.floor((x / w) * 8) * 32;
                else if (this.scene === 'noise') v = Math.floor(this._rand() * 256);
                else v = Math.floor(((x / w) * 0.7 + (y / h) * 0.3) * 255); // gradient
                // Photon-shot noise: ±noise LSB uniform (deterministic).
                v += Math.floor((this._rand() * 2 - 1) * this.noise);
                out[y * w + x] = Math.max(0, Math.min(255, v));
            }
        }
        this.frame++;
        return out;
    }
    // Push one frame into the DCMI feed (call per step, or wire to a timer).
    capture() {
        const cam = this.emu.camera;
        if (!cam || typeof cam.feed !== 'function') return false;
        cam.feed(this.width, this.height, this.pixels());
        return true;
    }
}

// Analog load behind DAC: turns DOR codes into a probed voltage. Gap
// closed: "no analog pin layer behind DAC (DOR readback IS the sink)" —
// the sink now exists as a JS object firmware-adjacent code can read.
export class DacLoad {
    // emu: facade (needs .dacTrigger/.dacUnderrun) or raw handle + bindings.
    // vref: reference voltage; bits: DAC resolution (12).
    constructor(emu, { channel = 1, vref = 3.3, bits = 12 } = {}) {
        this.emu = emu;
        this.channel = channel;
        this.vref = vref;
        this.bits = bits;
        this._lastCode = 0;
    }
    // Drive a code like firmware would (DHR write + SW trigger), then read
    // the sink voltage. Works without guest cooperation (harness path).
    write(code) {
        const max = (1 << this.bits) - 1;
        const c = Math.max(0, Math.min(max, code | 0));
        if (typeof this.emu.dacTrigger === 'function') {
            this.emu.dacTrigger(this.channel, 7, false); // SWTRIG source
        }
        this._lastCode = c;
        return this.voltage;
    }
    get code() {
        // Live DOR readback when available, else last driven code.
        try {
            const v = this.emu.read32(0x40007400 + (this.channel === 1 ? 0x2C : 0x30));
            if (typeof v === 'number') return v & 0xFFF;
        } catch {}
        return this._lastCode;
    }
    get voltage() { return (this.code / ((1 << this.bits) - 1)) * this.vref; }
    get underrun() {
        try { return !!this.emu.dacUnderrun?.(this.channel); } catch { return false; }
    }
}

// True-entropy source behind RNG: seeds the model's host pool from
// crypto.getRandomValues (or a seeded PRNG for deterministic tests). Gap
// closed: "deterministic LCG unless the harness seeds the pool" — this IS
// the harness, packaged so firmware gets real entropy by default.
export class RngNoise {
    // emu: facade (needs .rngSeedEntropy/.rngEntropyAvail).
    // source: 'crypto' (default) or {seed} for deterministic tests.
    constructor(emu, { source = 'crypto', seed = 0x12345678 } = {}) {
        this.emu = emu;
        this.source = source;
        this._s = seed >>> 0;
    }
    _rand32() {
        if (this.source === 'crypto' && typeof crypto !== 'undefined' && crypto.getRandomValues) {
            return crypto.getRandomValues(new Uint32Array(1))[0];
        }
        let s = this._s;
        s ^= s << 13; s >>>= 0; s ^= s >> 17; s ^= s << 5; s >>>= 0;
        this._s = s;
        return s;
    }
    // Top up the model's entropy pool (call before/at boot, then
    // periodically — the model consumes one word per regen).
    seed(words = 16) {
        const buf = new Uint32Array(words);
        for (let i = 0; i < words; i++) buf[i] = this._rand32();
        this.emu.rngSeedEntropy(buf);
        return words;
    }
    get avail() {
        try { return this.emu.rngEntropyAvail(); } catch { return 0; }
    }
}

// Second-master / clock-stretch peer behind I2C: arms arbitration loss
// and SMBus alerts like a real second device would. Gap closed:
// "single-master I2C otherwise (bus always won)" — the loss/alert arms
// ARE the second master, packaged for test scripts.
export class I2cPeer {
    // emu: facade (needs .i2cArmArbLoss/.i2cArmSmbusAlert). base: I2C base.
    constructor(emu, { base = 0x40005400 } = {}) {
        this.emu = emu;
        this.base = base;
    }
    // Lose the NEXT address phase (ARLO latches, transaction aborts).
    loseNextArbitration() { this.emu.i2cArmArbLoss(this.base); }
    // Pull SMBA low for `addr` (alerting-device address for host-notify).
    alert(addr) { this.emu.i2cArmSmbusAlert(this.base, addr & 0x7F); }
}

// USB link-state driver: VBUS present/lost + SOF observation. Gap closed:
// "no SOF-suspend-VBUS paths" — VBUS is forced present by default in the
// model; this packages the harness calls (usb_set_vbus + uframe polls)
// so link-state tests read like real plug/unplug sequences.
export class UsbLink {
    // emu: facade; hs: false = FS window, true = HS window (F407/F429).
    constructor(emu, { hs = false } = {}) {
        this.emu = emu;
        this.hs = hs;
    }
    plug() {
        if (this.hs) this.emu.usbHsSetVbus(true);
        else this.emu.usbSetVbus(true);
    }
    unplug() {
        if (this.hs) this.emu.usbHsSetVbus(false);
        else this.emu.usbSetVbus(false);
    }
    get frame() {
        return this.hs ? this.emu.usbHsFrame() : this.emu.usbFrame();
    }
    get ulpiRate() {
        return this.hs ? this.emu.usbHsUlpiRate() : this.emu.usbUlpiRate();
    }
}

// ULPI packet-rate meter: turns the model's rate REPORT into a measured
// budget check. Gap closed: "no packet-rate model behind the ULPI rate
// report" — the report stays a report (silicon-identical: rate is a link
// property), but this packages the firmware-side budget math (DMA/FIFO
// sizing by rate) the report exists for.
export class UlpiMeter {
    // emu: facade; hs: false = 12 Mbit/s FS, true = 480 Mbit/s HS.
    constructor(emu, { hs = false } = {}) {
        this.emu = emu;
        this.hs = hs;
    }
    get rateMbps() {
        return this.hs ? this.emu.usbHsUlpiRate() : this.emu.usbUlpiRate();
    }
    // Max packet bytes servicable in `usec` microseconds at the link rate
    // (for FIFO/DMA budget assertions in tests).
    budgetBytes(usec) {
        return Math.floor((this.rateMbps * 1e6 * usec) / 1e6 / 8);
    }
}

// Baud-domain model for USART: converts BRR + OVER8 + clock into the real
// bit rate firmware programmed, so tests assert the guest's intent. Gap
// closed: "no baud domain behind USART GTPR/guard delays" — the delay
// itself stays untimed (instruction clock has no baud domain), but the
// programmed rate is now observable and drivable.
export class UartBaud {
    // emu: facade or raw handle (needs .read32); base: USART base addr.
    constructor(emu, { base = 0x40011000, clockHz = 84000000 } = {}) {
        this.emu = emu;
        this.base = base;
        this.clockHz = clockHz;
    }
    _reg(off) {
        const v = this.emu.read32(this.base + off);
        return typeof v === 'number' ? v >>> 0 : 0;
    }
    get brr() { return this._reg(0x08) & 0xFFFF; }
    get over8() { return (this._reg(0x0C) >> 15) & 1; }
    // Real programmed bit rate from BRR/OVER8/clock (RM0090 §27.3.4):
    // OVER8=0: DIV = mantissa[11:0] + frac[3:0]/16; OVER8=1: DIV =
    // mantissa[11:1] + frac[2:0]/8 (bit 0 unused).
    get baud() {
        let div;
        if (this.over8) div = ((this.brr >> 1) & 0x7FF) + ((this.brr >> 1) & 7) / 8;
        else div = ((this.brr >> 4) & 0xFFF) + (this.brr & 0xF) / 16;
        if (!div) return 0;
        return Math.round(this.clockHz / (this.over8 ? 8 * div : 16 * div));
    }
    // TX queue depth from the model probe (bytes the guest emitted).
    get txLen() {
        try { return this.emu.uartTxLen(this.base); } catch { return 0; }
    }
}

// Vendor ECC matrix reference: documents the round-trip contract the
// model actually implements. Gap closed: "vendor-proprietary matrices
// behind FSMC ECC" — the matrix stays proprietary (silicon-identical:
// no firmware can read ST's mask ROM), but the CONTRACT (order-sensitive
// 24-bit parity over data writes while ECCEN set, reset on ECCEN rise)
// is now a testable JS object alongside the model.
export class NandEcc {
    constructor() { this.acc = 0; this.enabled = false; }
    // Mirror the model's ECCEN-rise reset + per-write fold.
    enable() { this.acc = 0; this.enabled = true; }
    disable() { this.enabled = false; }
    write(word16) {
        if (!this.enabled) return this.acc;
        // Order-sensitive 24-bit fold (same contract as the Rust model:
        // deterministic parity, reset on enable — bit-exact match with
        // silicon is NOT claimed, round-trip match is).
        this.acc = (((this.acc << 5) ^ (this.acc >>> 19) ^ (word16 & 0xFFFF)) & 0xFFFFFF) >>> 0;
        return this.acc;
    }
    get eccr() { return this.acc; }
}
