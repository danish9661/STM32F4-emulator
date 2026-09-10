# Arduino UNO R4 (Minima / WiFi) emulator — build plan

You are bringing up an emulator for the **Arduino UNO R4** by reusing the
proven Cortex-M4F CPU core in `core/`. Do NOT write a CPU decoder — it
exists, it is audited (see `cpu_bug.md` + AGENTS.md §25 in the parent
repo), and it runs 133 native tests green. Your job is everything around
it: peripherals, firmware, drivers, tests.

## 0. First hour (do this before anything else)

```bash
cd core/stm32-periph-wasm
cargo test            # must be 133 passed / 0 failed — proves the snapshot
```

If that is not green, STOP: the snapshot is broken. Re-sync it (see §8)
instead of debugging the core.

Start with the **Minima**. The WiFi adds an ESP32-S3 (radio, USB bridge,
LED matrix assist) as a SEPARATE chip over UART — the RA4M1 side comes
first; the ESP32 is a second emulator problem, not a first-week one.

## 1. Chip facts (Renesas RA4M1 R7FA4M1AB3CFM, verified from Arduino docs)

| Item | Value |
|---|---|
| Core | Cortex-M4F (M4 + single-precision FPU) — NOT M33 |
| Clock | 48 MHz |
| Code flash | 256 KB at **0x00000000** (NOT 0x08000000 like STM32!) |
| SRAM | 32 KB at 0x20000000 |
| Data flash (EEPROM emu) | 8 KB — model as plain storage |
| Peripherals | 0x40000000+ (Renesas map, below) |
| IO | 5 V tolerant operation (electrical only — irrelevant to emulation) |
| NVIC priority bits | 4 implemented like STM32 (verify in the Hardware User's Manual) |
| MPU / FPU | Present — reuse `cpu` + `peripherals/mpu.rs` + `peripherals/fpu.rs` as-is |

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
  (copy the *patterns*, e.g. `tim.rs`, `usart.rs`, `i2c.rs`; STM32
  register maps do NOT apply to Renesas peripherals).
- `core/stm32-periph-wasm/src/system.rs` — process globals (instruction
  clock, pending-fault channels), `lib.rs` — wasm exports.
- `core/docs/` — GAS encoding probes (the method for settling ANY decoder
  question: assemble with the F4 toolchain flags in the README and read
  the halfwords — do NOT guess encodings).

## 3. Reuse contract (what to keep vs replace)

KEEP untouched: everything in `src/cpu/`, `src/system.rs` atomics
semantics, `src/peripherals/{mpu,fpu,nvic}.rs`.

REPLACE/ADD: every Renesas peripheral as a new file implementing the
`Peripheral` trait (`read`/`write`/`tick` + `as_any_mut`), registered in
both `Peripherals::from_svd` and `Peripherals::new_wasm`, following the
existing QSPI/DWT precedent (explicit registration when the SVD omits
it). Reuse `tick_n` batching + the `INSTRUCTION_COUNT` virtual clock —
do NOT invent a second clock.

REWIRE: `FlatMemory` flash base (0x08000000 → 0x00000000), vector-table
reset read, and any 0x08000000 assumption in the driver you write. The
decoder never hardcodes flash addresses — only the memory map does.

## 4. Biggest adaptations: flash at zero + clocks + data flash

- RA firmware links flash at 0x00000000 with the vector table at 0.
  VTOR still works (default 0). Your loader must write the image at 0
  and read SP/PC from 0x0/0x4.
- The RA4M1 boots through clock setup (HOCO/MOCO/PLL) and option-setting
  memory. Your clock model MUST at minimum accept the boot's clock-config
  writes benignly (reads back what was written is usually enough to get
  past `SystemInit`), and option bytes read back erased (all-1s) values.
- Data flash (8 KB): plain byte storage with erase-to-1 semantics is
  enough for Arduino EEPROM emulation.

## 5. Peripheral bring-up order (each: model → firmware demo → test)

1. PORT (GPIO) + CAC-stub — blinky to UART marker. (`blink.ino` equivalent
   bare-metal first, Arduino core second.)
2. GPT32/GPT16 (PWM + timers) + AGT (low-power timers, accept-and-count).
3. SCI (UART first — the console is your lifeline; SPI/I2C modes after).
4. ADC14 + DAC12 + RTC + DMAC/DTC (memory-to-memory first, like the
   parent repo's sync-DMA path).
5. ELC (event link controller — Arduino code wires events through it;
   model as direct dispatch or a routing table, same lesson as Nordic
   PPI in the micro:bit plan).
6. WDT/IWDT + CRC + DOC + OPAMP/ACMP (small, mostly accept-and-report).
7. Defer: CTSU (touch), USBFS, CAN, CAC-lock details. The 12×8 LED
   matrix (WiFi) is driven from RA4M1 GPIOs — model it as a pin-grid
   renderer like the parent repo's LTDC panel, not as a peripheral.

SVD: look for the Renesas RA SVD (Keil DFP `Renesas.RA_DFP`, or the FSP
bundle — verify availability yourself; if missing, hand-write the map
from the Hardware User's Manual, peripheral by peripheral in the order
above). The `init_svd` path consumes it the same way.

## 6. Firmware strategy

Bring-up order: bare-metal blinky (your `blinky_test/`, same Makefile
pattern as the parent repo's `blinky/`) → UART echo → Arduino
`Blink.ino` via ArduinoCore-renesas (it wraps Renesas FSP — FSP *runs on
the core*, so it only needs register models, which is exactly what you
are building) → `AnalogInOutSerial`-class peripheral demos. Do NOT start
from a full core build — minimize until markers print, then grow.

## 7. Validation (build your own battery early)

Mirror the parent repo: per-firmware Node harnesses asserting UART
markers (exit 0 = PASS), one chained test script, and a browser sweep
once the demo page exists. Minimum bar before claiming a peripheral:
boot marker + functional marker + second consecutive run (state must not
leak between instances — see `reset_state`).

## 8. Re-sync + bug workflow

- Re-sync the core: from the repo root,
  `cp -r stm32-periph-wasm/src boards/uno-r4/core/stm32-periph-wasm/`
  (plus `Cargo.toml`/`Cargo.lock` if changed), then re-run `cargo test`.
  Never hand-edit the snapshot's `cpu/` to fix a board problem — a CPU
  bug is a main-repo bug.
- CPU bugs/suspicions go in the parent repo's `cpu_bug.md` (claim it,
  repro case, exact pc/opcode — the core fails loudly, use that), NOT
  worked around in board code. Read that file first; your issue may
  already be listed as policy.
