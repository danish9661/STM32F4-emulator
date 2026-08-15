# Component-attachment API

Public JS API for wiring virtual hardware (LEDs, buttons, sensors, displays)
to the emulator's pins and buses — the rp2040js-style "breadboard" layer, in
the spirit of Wokwi's rp2040js. Everything here is JS built on existing
wasm exports (`stm32-periph-wasm/src/lib.rs`); no Rust changes are needed to
add a new component.

There are two attachment paths, because of a real constraint in the Rust
model (see [peripherals.md](peripherals.md)'s SPI/I2C rows): GPIO pins can
be read/written/watched at any time, but SPI/I2C bus taps must be
registered **before** the emulator's `init()` runs, because the `Spi`/`I2c`
peripheral objects snapshot their attached-device list once at construction
and never rescan it.

## GPIO: pins, anytime

Available on the object `createEmulator()`/`createSTM32F407()` returns —
usable any time after construction, including mid-run:

```js
const p = emu.pin('A', 5);      // or emu.pin(0, 5) — index or letter
p.read();                       // what the guest is driving out (bool)
p.readInput();                  // the input level the guest sees (bool)
p.write(true);                  // drive an input level into the guest

const unwatch = emu.watchPin('A', 5, (level) => {
    console.log('PA5 ->', level);
});
unwatch();                      // stop watching
```

`watchPin` polls once per `step()` (not a true per-instruction Rust-side
event) — fine for firmware-paced GPIO toggles (LED blink, button-driven
IRQs), the same granularity the browser demo's frame loop already uses to
watch other devices.

### `LED` / `Button` (`site/components.js`)

Minimal starter components built on `pin()`/`watchPin()` — templates, not
a full catalog:

```js
import { LED, Button } from 'stm32f4-emulator'; // or site/components.js directly

const led = new LED(emu, 'A', 5);
led.watch((on) => console.log('LED', on ? 'ON' : 'OFF'));
led.value;                      // current state (bool), respects activeLow

const btn = new Button(emu, 'A', 0, { activeLow: false }); // EXTI0-style, active-high
btn.press();
btn.release();
```

The full set: `LED`, `Button`, `Pwm`, `I2cRegisterDevice`, `Potentiometer`
— all exported from `stm32f4-emulator` (or `site/components.js` directly).

Write your own component the same way: wrap `emu.pin()`/`emu.watchPin()`
in a small class.

## ADC: analog values, anytime

Like GPIO (and unlike the bus taps below), ADC channel injection is a
global override with no "before init()" constraint:

```js
emu.setAdcChannel('ADC1', 3, 2048);   // force channel 3 to a 12-bit value
emu.clearAdcChannel('ADC1', 3);       // revert to the synthetic default

const pot = new Potentiometer(emu, 'ADC1', 3, { min: 0, max: 100 });
pot.value = 75;                       // mapped onto the 0-4095 ADC range
pot.release();                        // stop overriding the channel
```

Without an override, channels keep the emulator's synthetic defaults
(16/17 = temperature, 18 = Vbat, everything else pseudo-random).

**Timing note:** a conversion only completes once enough *emulated
instructions* have elapsed (the ADC model is instruction-count driven).
The driver batches its instruction-counter updates every `tickEvery`
instructions (default 5000), so if you're triggering conversions manually
rather than from firmware, `emu.step()` with a budget above that
threshold — a smaller budget never advances the counter and the
conversion never completes. See `site/test_component_adc.mjs`.

## Pwm and I2C register devices

`Pwm` decodes a timer's CR1/CCER/PSC/ARR/CCRn into `.freq`/`.duty`:

```js
const pwm = new Pwm(emu, 'TIM2', 1);  // timer name + channel 1-4
pwm.freq;  // Hz (0 when the timer/channel is disabled)
pwm.duty;  // 0-1
```

`freq` assumes the standard 168 MHz F407 clock tree: APB2 timers
(TIM1/8/9/10/11) tick at 168 MHz, APB1 timers (everything else) at
84 MHz. If your firmware configures the buses differently, pass the real
timer clock: `new Pwm(emu, 'TIM3', 2, { clockHz: 42e6 })`.

`I2cRegisterDevice` wraps a register-file device registered through the
`ext_devices.regfile` construction option (pointer-addressed I²C devices
like the DS3231):

```js
const emu = await createSTM32F407({
    firmware,
    ext_devices: { regfile: [{ peripheral: 'I2C1', address: 0x68, size: 20, init: [] }] },
});
const dev = new I2cRegisterDevice(emu, 'I2C1');
dev.get(0x00);          // read a register the firmware wrote
dev.set(0x11, 0x1B);    // seed a register the firmware will read
```

## SPI / I2C: custom bus devices, at construction time

Pass devices via the `ext_devices` option to `createEmulator`/
`createSTM32F407`, alongside the existing `oled`/`tft`/`spi_flash` configs:

```js
const emu = await createSTM32F407({
    firmware,
    ext_devices: {
        spiDevices: [{
            peripheral: 'SPI1', cs: 'PA4', dc: null,
            handler(events, pushMiso) {
                for (const ev of events) {
                    if (ev & 0x80000000) continue; // CS edge marker
                    const byte = ev & 0xFF;
                    // ... decode your device's protocol ...
                    pushMiso(new Uint8Array([0x00])); // answer a read
                }
            },
        }],
        i2cDevices: [{
            peripheral: 'I2C1', address: 0x42,
            handler(events, pushRx) {
                for (const ev of events) {
                    if (ev & 0x80000000) continue; // START/STOP marker
                    const byte = ev & 0xFF;
                    // ... decode your device's protocol ...
                }
            },
        }],
    },
});
```

`handler(events, push)` is called once per `step()` with any new events
since the last call (empty calls are skipped). The event encoding is the
same raw tap format the built-in `oled`/`tft` devices decode internally —
see `processOled`/`processTft` in `site/emulator.js` for a worked example
of parsing a real protocol (SSD1306 / ILI9341) on top of the same taps.

## Multiple firmwares in one process (fixed 2026-08-15)

Sequential `createEmulator()` instances in a single process are supported.
Booting `blinky`, then `rtc_test`, then `buzzer_test` in one script works in
any order; `site/test_multi_instance.mjs` covers three orderings plus a
seven-instance run and is part of `npm test`.

This used to be a hard "one firmware per process" rule, because the second
instance's I2C register-file path hung right after its first UART line. Two
independent causes, both now fixed:

- `stm32-periph-wasm/src/lib.rs` held the system in a `OnceLock`, whose
  `set()` accepts only the first value. Every later `init()`/`init_svd()`
  was silently discarded, so instance 2 ran on instance 1's entire
  peripheral tree. `SYS` is now an `AtomicPtr` to a leaked `Box`, so each
  `init` installs a fresh system.
- The module-level device list and the tap/atomic globals in
  `system.rs` accumulated across instances, so peripheral constructors
  (which bind by first match) attached to the previous instance's devices.
  `createEmulator()` now calls `reset_state()` before registering devices.

Instances are still not safe to run *concurrently* — there is exactly one
active system per process, and creating a new one detaches the old. Close
an instance before creating the next, as the tests do. Note also that the
old system is leaked rather than freed (see the `SYS` comment in `lib.rs`
for why), so a process that boots thousands of firmwares will grow.

## Out of scope

- No true per-instruction GPIO edge events from Rust — `watchPin` is
  step-granularity polling. A `gpio_tap`-style Rust event queue (matching
  `spi_tap`/`i2c_tap`) is a possible follow-up if that granularity proves
  too coarse for a specific firmware.
- FSMC has no device-attachment path (`docs/peripherals.md`'s FSMC row) —
  `read_data`/`write_data` are stubs; a memory-mapped-display bridge would
  need new Rust work, not just a JS API.
