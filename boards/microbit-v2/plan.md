# micro:bit v2 / v2.2 emulator — build plan

You are bringing up an emulator for the **BBC micro:bit v2** (v2.2 is the
same SoC, minor board revision) by reusing the proven Cortex-M4F CPU core
in `core/`. Do NOT write a CPU decoder — it exists, it is audited (see
`cpu_bug.md` + AGENTS.md §25 in the parent repo), and it runs 133 native
tests green. Your job is everything around it: peripherals, firmware,
drivers, tests.

## 0. First hour (do this before anything else)

```bash
cd core/stm32-periph-wasm
cargo test            # must be 133 passed / 0 failed — proves the snapshot
```

If that is not green, STOP: the snapshot is broken. Re-sync it (see §8)
instead of debugging the core.

M0+ (micro:bit v1 nRF51, interface MCUs) is a separate track with its own
strict-mode gate — out of scope here. This plan is M4F only.

## 1. Chip facts (nRF52833, all load-bearing)

| Item | Value |
|---|---|
| Core | Cortex-M4F (M4 + single-precision FPU) |
| Clock | 64 MHz |
| Flash | 512 KB at **0x00000000** (NOT 0x08000000 like STM32!) |
| RAM | 128 KB at 0x20000000 |
| Peripherals | 0x40000000+ (Nordic map, below) |
| FICR / UICR | 0x10000000 / 0x10001000 (factory info + customer regs — firmware reads these at boot, model them early or boot hangs) |
| NVIC priority bits | 3 implemented (verify in the Product Specification — our raw-byte compare handles any width, so no core change either way) |
| MPU / FPU | Present — reuse `cpu` + `peripherals/mpu.rs` + `peripherals/fpu.rs` as-is |

Board extras (model after the chip boots): 5×5 LED matrix (driven from
GPIO, no controller — your matrix renderer reads pins), buttons A/B,
LSM303 accelerometer+magnetometer (I2C), microphone + speaker, edge
connector, USB, BLE antenna. The interface MCU (DAPLink, M0+) is NOT
emulated — firmware talks to the nRF directly.

## 2. What you got (`core/`)

A snapshot of the parent repo's `stm32-periph-wasm` crate plus the
firmware binaries/SVD/probes its tests need (run `cargo test` in
`core/stm32-periph-wasm` — 133 green proves the snapshot).

- `core/stm32-periph-wasm/src/cpu/` — THE reuse target: `thumb.rs`
  (decoder), `mod.rs` (Cpu, stepping, exception entry/return, delivery),
  `mem.rs` (`Memory` trait + `FlatMemory`), `regs.rs` (incl. S0–S31/FPSCR).
  Snapshot matches parent-repo commit `8a97498` (verify drift with
  `git log --oneline -1` at the repo root).
- `core/stm32-periph-wasm/src/peripherals/` — reference implementations
  (copy the *patterns*, e.g. `tim.rs`, `usart.rs`, `i2c.rs`; most STM32
  models do NOT apply to Nordic registers).
- `core/stm32-periph-wasm/src/system.rs` — process globals (instruction
  clock, pending-fault channels), `lib.rs` — wasm exports.
- `core/docs/` — GAS encoding probes (the method for settling ANY decoder
  question: assemble with the F4 toolchain flags in the README and read
  the halfwords — do NOT guess encodings).

## 3. Reuse contract (what to keep vs replace)

KEEP untouched: everything in `src/cpu/`, `src/system.rs` atomics
semantics, `src/peripherals/{mpu,fpu,nvic}.rs` (NVIC is generic; nRF52's
3-bit priorities work with the raw compare — do not "fix" it).

REPLACE/ADD: every Nordic peripheral as a new file implementing the
`Peripheral` trait (`read`/`write`/`tick` + `as_any_mut`), registered in
both `Peripherals::from_svd` and `Peripherals::new_wasm`, following the
existing QSPI/DWT precedent (explicit registration when the SVD omits
it). Reuse `tick_n` batching + the `INSTRUCTION_COUNT` virtual clock —
do NOT invent a second clock.

REWIRE: `FlatMemory` flash base (0x08000000 → 0x00000000), vector-table
reset read, and any 0x08000000 assumption in the driver you write. The
decoder never hardcodes flash addresses — only the memory map does.

## 4. Biggest adaptation: flash at zero + FICR/UICR

- nRF firmware links flash at 0x00000000 with the vector table at 0.
  VTOR still works (default 0). Your loader must write the image at 0
  and read SP/PC from 0x0/0x4.
- Nordic boot code (and CODAL/MicroPython/Zephyr) reads FICR
  (DEVICEID, INFO) and UICR very early. Model FICR as constants +
  UICR as storage, or nothing boots.

## 5. Peripheral bring-up order (each: model → firmware demo → test)

1. CLOCK + GPIO (P0/P1) + NVMC-stub — blinky to UART marker.
2. TIMER0–4 + RTC0–2 + SysTick-driven delays.
3. UARTE0 (console is your lifeline from here on).
4. TWIM/TWIS + SPIM/SPIS + GPIOTE/PPI (PPI = route events↔tasks; without
   it half the SDK examples stall — model it as direct dispatch).
5. SAADC + TEMP + RNG + PWM + QSPI + USBD.
6. RADIO/BLE last — and SoftDevice (Nordic's binary BLE stack, SVC
   interface) is OUT OF SCOPE for bring-up: target non-SoftDevice
   firmware first (bare-metal, CODAL without BLE, Zephyr without
   SoftDevice). Emulating the SoftDevice API surface is its own project.

Get the SVD first: Nordic nRF52833 SVD (nRF MDK / nrfx / NordicSe
... — fetch `nrf52833.svd` from the Nordic Semiconductor MDK or CMSIS
pack and put it at `nrf52833.svd`; the `init_svd` path consumes it the
same way the STM32 flow consumes `stm32f407.svd`.

## 6. Firmware strategy

Bring-up order: bare-metal blinky (your `blinky_test/`, same Makefile
pattern as the parent repo's `blinky/`) → UART echo → CODAL blinky
(`codal-microbit-v2`, static lib built with the same ARM GCC) →
MicroPython or Zephyr hello. Do NOT start from a full DAL build —
minimize until markers print, then grow.

## 7. Validation (build your own battery early)

Mirror the parent repo: per-firmware Node harnesses asserting UART
markers (exit 0 = PASS), one `npm test`-style chain, and a browser
sweep once the demo page exists. Minimum bar before claiming a
peripheral: boot marker + functional marker + second consecutive run
(state must not leak between instances — see `reset_state`).

## 8. Re-sync + bug workflow

- Re-sync the core: from the repo root,
  `cp -r stm32-periph-wasm/src boards/microbit-v2/core/stm32-periph-wasm/`
  (plus `Cargo.toml`/`Cargo.lock` if changed), then re-run `cargo test`.
  Never hand-edit the snapshot's `cpu/` to fix a board problem — a CPU
  bug is a main-repo bug.
- CPU bugs/suspicions go in the parent repo's `cpu_bug.md` (claim it,
  repro case, exact pc/opcode — the core fails loudly, use that), NOT
  worked around in board code. Read that file first; your issue may
  already be listed as policy.
