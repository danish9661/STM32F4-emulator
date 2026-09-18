// Minimal starter virtual-component library, built on the public
// pin/register-access API from emulator.js (`emu.pin()`, `emu.watchPin()`,
// `emu.read32()`, `emu.i2cRegfile()`). No imports: works in Node and the
// browser, same "import-free" convention as emulator.js.
//
// These are templates, not a full component catalog — attach your own
// devices the same way (see docs/components.md).

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
