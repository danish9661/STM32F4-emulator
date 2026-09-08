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
