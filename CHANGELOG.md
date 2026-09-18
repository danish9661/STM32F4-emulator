# Changelog

All notable changes to `stm32f4-emu` are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); this project uses
date-based entries rather than strict SemVer until the first published release.

## [Unreleased]

### Session: pkg parity + trace view + servo + IDCODE follow-ups
- `stm32-periph-wasm/pkg` (nodejs) rebuilt to byte-identical parity with
  `site/vendor` (`cmp` clean); all 9 gap-10/IDCODE wasm exports verified
  from the packaged glue. AGENTS §38 stale "pkg NOT rebuilt" note fixed.
- Trace (waveform) view in the browser console: per-frame sampling of up
  to 4 MMIO addresses (analog auto-scale, `:bN` bit plots) + a DMA
  pending-count strip, painted on `#traceCanvas` (verified headless:
  9767 non-bg pixels on a PA5 `:b5` trace).
- Servo support (printer heritage, no EtherCAT silicon on F4 — the honest
  close of that roadmap item): model `tim_oc_mode`/`tim_pwm_pulse_us`
  probes + `Pwm.mode`/`modeName` + `Servo` component (pulse→angle) +
  `test_component_servo.mjs` (mode=6, 1500 us, 90.0° PASS).
- `comprehensive_test` IDCODE check made per-map aware (F429 reports
  DEV_ID 0x419 now): rebuilt stock + family bins, `firmware.js` regen.
- AGENTS.md stale 2026-09-11 UNCOMMITTED headers cleared (all landed).

### npm 1.1.0 release prep (verified, NOT published)
- `npm pack` verified: 29 files, 1.7 MB tarball / 9.8 MB unpacked, `files`
  allowlist covers `index.mjs`, `cli.mjs`, the MCP server, all of `site/`
  (console, emulator, vendor WASM+SVDs, firmware bundle) and
  `tools/make_firmware.mjs`. Pack output is gitignored (`*.tgz`).
- Consumer test (tarball installed into a scratch dir, 2026-09-18): blinky
  boots over the packaged `index.mjs` API (banner + `tick 0`), and all 9
  gap-10/IDCODE wasm exports resolve as functions from the packaged vendor
  glue. EXIT 0, no publish performed (`npm publish` remains a maintainer
  decision — package name `stm32f4-emu` unclaimed check + provenance left
  for release day).
- Version bumped 1.0.1 → 1.1.0 (new peripherals surface: SDIO CMD24, QSPI
  mmap, LTDC CLUT, I2C slave, DMA FCR/DBM, DAC DMAUDR, per-map IDCODE).

### Added
- **Peripheral gap batches 6–8** (model + mock pins + board-doc rows, all
  synced to `site/docs-src/` + `website/docs/`):
  - Batch 6: FLASH error flags (WRPERR/PGSERR/PGAERR, OPTLOCK/OPTSTRT),
    SPI HW CRC + OVR/MODF/FRE/BSY, USART CTSE flow control + FE/PE fault
    injection, SDIO ACMD prefix + wide-bus + DAT1 IRQ, RTC wakeup timer +
    timestamp + tamper + smooth calibration.
  - Batch 7: SDIO width-scaled data timing (DTIMEOUT/DCRCFAIL/RXOVERR/
    TXUNDERR), USART LIN break + Smartcard T=0 NACK loop + IrDA pulse
    classes, SPI slave gating (NSS/SSM+SSI, DR preload, harness SCK),
    RTC tamper-pin physics (sample-count filter, BKPR erase), FLASH RDP
    levels + MER timing window.
  - Batch 8: ADC overrun (OVR latches, DR read clears the pair), USART
    IDLE latch + SBK TX break + PEIE/LBDIE IRQ paths (PE moved off EIE),
    TIM one-pulse mode (CEN self-clears at update), GPIO LCKR key sequence
    + per-pin config freeze (incl. OTYPER `&`/`===` precedence fix).
  - Mock-consumer harness: 168 → 312 checks (`t_flash_err`,
    `t_spi_crc_err`, `t_usart_flow_err`, `t_sdio_acmd`, `t_rtc_wut_ts`,
    `t_sdio_timing`, `t_usart_protocols`, `t_spi_slave_gate`,
    `t_rtc_tamper_phys`, `t_flash_rdp`, `t_honor_pass`, `t_gap9`,
    `t_gap10`).
  - Batch 9: ADC injected group (JSWSTART/JAUTO, JL/JOFR/JDR/JEOC/JSTRT,
    ALIGN, CONT, JAWDEN gate), TIM1/TIM8 BDTR/MOE/break + RCR repetition
    + EGR software events, RTC SHIFTR shift + ALRMASSR MASKSS gate, USART
    mute mode (RWU/WAKE).
  - Batch 10 (the six "out of scope" walls, knocked down): SDIO CMD24
    single-block write (image round-trip), QSPI memory-mapped window
    (AHB 0x90000000 live reads), LTDC CLUT load + L8/AL44/AL88 resolve,
    I2C slave mode (OAR match → ADDR → DR rx/tx → STOP), DMA FCR
    thresholds/FEIF/DBM-direct-TEIF/CT-flip, DAC TSEL mux + DMAUDR
    underrun — each with native test + mock pin + compiled guest firmware
    (`gap10_*`, 6/6 on guest + 6/6 in-browser CDP smoke).
- **Docs audit pass (2026-09-18)**: rewrote stale `architecture.md` (the
  Rust core is the sole backend — no hooks/pump/wedge), fixed
  `MAX_BATCH`/preset-count/UART-RX/device-panel/DOOM-fps staleness in
  `usage.md`, added the post-§23 backend note to `benchmarks.md`, closed
  the DCMI/USB/demo-firmware roadmap items in `progress-and-future.md`,
  and pinned batch-8 mock names into all five board pages.

### Added
- **`stm32f4-emu` CLI** (`bin`): headless runner that loads a `.bin`/`.elf`/`.hex`
  firmware, boots it, and streams the guest UART to stdout. Supports
  `--inst <N>` (instruction budget), `--format auto|bin|hex|elf`,
  `--verbose` (peripheral register trace), `--help`, and `--version`.
- **`--verbose` debug mode**: `createEmulator({ verbose })` (and the CLI's
  `--verbose`) traces every peripheral MMIO read/write to stderr, capped at
  5000 accesses so a chatty firmware can't flood the terminal.
- **Actionable firmware-load errors**: `createEmulator` now rejects an empty or
  too-small image and a zero reset vector with a message explaining what a valid
  STM32F4 firmware looks like; `loaders.js` ELF/HEX parse failures now name the
  expected format and likely cause.
- **`stm32f4-mcp --help` / `--version`** for the MCP server bin.

### Removed
- **Unicorn CPU backend**: the vendored Unicorn 2.1.4 engine
  (`site/vendor/unicorn_arm.*`, `stm32-periph-wasm/pkg/unicorn_arm.*`, the
  `stm32-periph-wasm/package/` distribution), the `cpu_backend`/`unicorn`
  emulator options, the JS ISR pump, and the `?cpu=` UI switch. The Rust
  Thumb-2 core (proven bit-identical over 543 differential-fuzz vectors plus
  lockstep traces) is now the sole backend. `probe_freertos.mjs`,
  `test_doom.mjs`, and the `test:fuzz` oracle were removed or replaced by
  their Rust-core equivalents; `pkg/cli.mjs` was ported onto
  `createEmulator`.

### Fixed
- **Synchronous mem-to-mem DMA completion**: polling firmware that checks
  NDTR/dst/flags on the instructions right after enabling the stream
  (`edge_test`, `periph_test`) now sees the transfer complete inline. The
  Rust core drains staged mem-copy transfers straight after the guest's EN
  store (bytes moved in guest RAM + TCIF/HTIF latched); peripheral-side
  transfers still stage for the JS driver.
- **Live timer/counter reads**: TIM CNT, WWDG counter/EWIF, and DCMI FIFO
  state are now evaluated from the live instruction clock on read, so polled
  checks (`TIM CNT advances`, `WWDG EWIF set`, `DCMI FNE set`) pass even when
  no model tick ran since the enabling write. The core publishes executed
  instructions in 16-inst chunks; the post-step driver tick no longer
  re-adds the budget (`tick_peripherals`).
- **WWDG counts with WDGA clear** (reset generation still needs WDGA):
  firmware can observe the EWIF edge reset-free, matching silicon.
- **DCMI FNE (SR bit 2)** reflects FIFO/sensor state; polled DR reads pull
  live pixels mid-capture.
- **PWM duty math** uses u64: ARR=0xFFFFFFFF (reset default) no longer
  traps on divide-by-zero during transient config windows.
- **edge_test SPI clocking**: JEDEC/device-ID/16-bit reads now discard the
  command-phase dummy byte first (reads never clock on real SPI either).
- **FreeRTOS interrupt-pump context-switch bug**: a task-context `portYIELD()`
  (a `str` to SCB ICSR `PENDSVSET`) was stopped mid-instruction with PC frozen
  at the store; the exception frame saved that frozen PC, so the resumed task
  re-executed the store and re-pended PendSV forever — deadlocking the
  scheduler when the highest-priority task yielded. `processInterrupts` now
  advances the saved return PC past the store (matching real Cortex-M).

## [0.1.0] — baseline

### Added
- STM32F407 emulator: Unicorn 2.1.4 (WASM) Cortex-M4 CPU + a Rust peripheral
  model (RCC, GPIO, USART, TIM, NVIC/SysTick/EXTI, ETH+DMA, I2C, SPI/I2S, CAN,
  LTDC, DCMI, RTC, ADC, FSMC, FLASH).
- Networking: `eth_http` / `eth_dhcp` / `eth_test` firmwares with a canned
  `netsim` and a real gVisor-backed gateway (`openhw-local-gateway`).
- Browser single-page console (UART, GPIO grid, gateway, device panels) and the
  DOOM (doomgeneric F407) port running in a Web Worker.
- Node API (`createSTM32F407`, `createEmulator`, `decodeFirmware`,
  `createNetSim`), component-attachment API, and an MCP server.
- FreeRTOS port firmware (`freertos_test`) verifying the ISR →
  `xSemaphoreGiveFromISR` → PendSV context-switch path, wired as the
  `probe_freertos.mjs` regression test.
