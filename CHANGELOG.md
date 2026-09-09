# Changelog

All notable changes to `stm32f4-emu` are documented here. The format is based
on [Keep a Changelog](https://keepachangelog.com/); this project uses
date-based entries rather than strict SemVer until the first published release.

## [Unreleased]

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
