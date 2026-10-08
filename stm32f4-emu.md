# stm32f4-emu — hook spec (for emulator agent)

Our runner: `OpenHW-studio-frontend/src/worker/runners/stm32f4-runner.ts`.
Engine site: `public/vendor/board-engines/stm32f4-emu/site/{stm32f4.js,emulator.js}`.
Status: **wedged — needs core-side work**. Firmware faults/stops at the first I2C data write (`millis()` frozen after ~200, reboot loop, zero runner callbacks).

## What to add / fix

1. **Diagnose the first-write fault (P0)**. Evidence: frozen `millis()` ⇒ ~0 instructions retired per `step()` (a flag-spin would still advance SysTick). Suspects: `fault_pc` set (`emulator.js:1370-1371`) or the WFI branch (`:1325-1356`). Expose to JS: `stopped` flag + `fault_pc` (or equivalent) so our discriminator probe can read them.
2. **Broadcast I2C event queue (P0)**. Today `processOled()` (`emulator.js:498-500`, run at `:767`) drains the same `i2c_take_events(I2C1)` queue **before** `processI2cDevices()` (`:732-737`, `:778`) — with `ext.oled` on I2C1, per-address observer specs (`withAssets.i2c`) structurally starve even on a healthy bus. Fix: deliver each event to ALL consumers (broadcast), or give observers their own queue — never one drain starving another.
3. **Read-path parity with F1 (P1)**. `withAssets.i2c` specs accept `onStart/onWrite/onStop` but **no `onRead`**; `parseI2c` (`site/stm32f4.js:189-206`) never dispatches reads (only `pushRx`, which our runner never fills). Add `onRead` + RX-fill — MANDATORY, not optional: every I2C sensor in the registry (MPU6050, BMP180, ADXL345, DS1307 RTC, EEPROM, PCA9685) reads, so without this the whole future sensor shelf stays dark on F4.
4. Keep F1-style semantics where they exist: pre-`init_svd` address-keyed `i2c_register_slave` (`emulator.js:431-435`) is fine as long as (2) broadcasts.

## Constraints

- Additive-only; F4 Arduino serial/boot cells already pass — no behavior change when `withAssets.i2c` is absent.
- Runner `stepBudget` ignores the engine `stopped` flag today; after exposing it, keep stepping side-effect-free when stopped.

## Cross-cutting constraints (sync/SAB — engine side of the contract)

Engines never touch SharedArrayBuffer: each engine runs as a plain
synchronous step function inside its board Worker, and only our
`arm-runner-base.ts` touches SAB (pin bitfields, barrier cells, STEP
collect). Two things the engine must still guarantee so multi-board
lockstep holds:

1. **Reset clears everything, synchronously** — cores, peripherals, pending
   events, and any `stopped`/fault flags. A sticky flag desyncs one board's
   lane forever while the others advance. (Directly relevant to §1: the
   first-write fault flag must not survive `reset()`.)
2. **Expose a cycle/instruction counter per step** — feeds sim-time edge
   stamps (replacing the runner's wall-clock approximation) and the
   skew/pace accounting. One counter serves both.

## Further surface (same agent — full protocol audit 2026-10)

Already present in-engine, runner-unwired (our side, listed so nothing is missed):

5. **ADC**: `setAdcChannel/clearAdcChannel` (`emulator.js:705-706`) exists —
   our runner returns `NaN` (`stm32f4-runner.ts:387`). Engine task: keep +
   document it; wiring is ours.
6. **PWM/timer-out**: TIM-register buzzer model (`ext_devices.buzzer`,
   TIM3/TIM4) exists — generalize/document for servo/LED cells if cheap.
7. **SDIO blocks / camera frame / FSMC / QSPI** (`stm32f4.js:634-669`):
   present; no cells written yet — P2, keep stable, do not expand scope.

## Acceptance (our e2e, in order)

1. Discriminator probe (to be written): OLED firmware through first-write window ⇒ `emu.oled.frame > 0` (`stm32f4.js:393-399`, incremented at `emulator.js:530`) AND firmware `millis()` delta ≈ 0 AND `stopped == true`/`fault_pc != 0xFFFFFFFF`. This conjunction = "bus transacted, core faulted".
2. Full cell: Disco F407 SSD1306 `vramFill > 0` via I2C1 (mirrors the green F1 cell).
