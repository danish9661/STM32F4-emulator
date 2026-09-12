---
sidebar_position: 3
title: Boards
description: Supported STM32F4 boards and chips — what is implemented, what was verified, and what is left.
---

# Boards

Every supported board is a **Cortex-M4F** — one shared CPU core, no decoder
work per board. A board variant is an SVD (register map), flash/RAM sizes,
and pins (`site/boards.js`). Pick one from the **Board** menu on the
[console](/stm32F4-emulator/console/console.html): the preset list filters
to firmware built for it, and deep links work as
`console.html?board=stm32f401` / `console.html?fw=arduino_disco_f407vg`.

## Variants

Sizes are the superset per SVD family (a smaller die's firmware runs
unchanged — the initial SP comes from its own vector table).

| Chip | Flash / RAM | Boards (Arduino-validated) | LED | Serial |
|---|---|---|---|---|
| STM32F401 | 512K / 96K | BlackPill F401CC | PC13 | USART1 |
| | | Nucleo-F401RE | PA5 | USART2 |
| STM32F411 | 512K / 128K | BlackPill F411CE | PC13 | USART1 |
| | | Nucleo-F411RE | PA5 | USART2 |
| STM32F407 | 1M / 192K | Discovery F407VG | PD12 | USART2 |
| STM32F407VE/ZE | 512K / 192K | Black F407VE | PA6 | USART1 |
| | | Black F407ZE | PF10 | USART1 |
| STM32F429 | 2M / 256K | Discovery F429ZI | PG13 | USART1 |

Blinky presets exist for all eight (`blinky_f401`, `blinky_f411`,
`blinky_f407g`, `blinky_nucleo_f401`, `blinky_nucleo_f411`,
`blinky_f429`, `blinky_f407ve`, `blinky_f407ze` — the VE/ZE ones reuse
the stock blinky, so they blink PA5). The same Arduino sketch (Serial
prints + LED blink + SysTick `delay()`) is built for all eight targets
(`arduino_bp_f401cc`, …) — the full Arduino stack on every variant.

## What is implemented

- **Shared core** — CPU, FPU, MPU, and exception delivery are identical on
  every board; board work never touches the decoder.
- **Per-board maps** — Keil DFP SVDs in `site/vendor/stm32f*.svd` plus
  sizes/labels in `site/boards.js`. SVDs without a system block get the
  core SCB/MPU/SysTick/FPU windows registered automatically; the FSMC
  controller is registered when the SVD omits it (F429 calls it FMC),
  and `SAI`/`DBG` SVD names are accepted as aliases.
- **~190 presets pass board-gated** — `site/test_board_matrix.mjs` (in
  `npm test`) boots every portable demo on every compatible map and
  asserts its completion markers: only passing pairs become presets
  (`BOARDS_OF_FIRMWARE`). Silicon-absent peripherals (CAN/DAC on
  F401/F411, SAI/USB gaps, UART4) fail honestly and stay unlisted.
- **Real-firmware validation** — `site/test_arduino_boards.mjs` (in
  `npm test`) boots the Arduino build per board and asserts banner +
  ticks + LED toggles + no fault: **8/8 PASS**.
- **Browser** — board menu with per-board preset filtering, `?board=`
  deep links, and `test_browser.mjs` coverage (blinky per board, two
  Arduino builds, plus device/IRQ/USB/DMA showcases per family).

## What is left

- **DMA2D (F429)** — modeled (`dma2d.rs`: R2M/M2M/PFC/blend, TCIF/IRQ56)
  with a 5-phase IRQ-driven firmware proof (`dma2d_test`, in `npm test`
  and the browser). No gap remains.
- **F429 Ethernet** — fully working: same-sources builds
  (`eth_http/dhcp/test/irq_test_f429`, SRAM layouts nm-identical to
  F407), gateway runs (DHCP→TCP→HTTP, DHCP loop, TX test) and netsim
  flows on the Keil map; polling + IRQ paths, ARP/DHCP/TCP/HTTP all
  covered. No firmware speaks ICMP/DNS — nothing to verify there.
- **F429 GPIOK** — on silicon, covered by the generic GPIO bank, but no
  firmware drives a K pin yet.
- **Known model gaps (fail identically on stock F407, not board issues)** —
  DCMI empty-capture IRQ + SDIO CMDSENT flag (comprehensive_test's two
  red checks), `new/deep_periph_test` (never green anywhere; kept as
  boot-only F407 presets), `rx_interrupt_test` Arduino builds (sketch ISR
  collides with the core's USART1 handler on every FQBN).
- **M0+ chips are out of scope** — different core; see `cpu_bug.md`.

## Adding a board

1. Drop the chip SVD into `site/vendor/` (restore it after every
   `wasm-pack` rebuild — the out-dir is wiped) and register sizes/label
   in `site/boards.js`.
2. Build the firmware with the right link script, top-of-RAM SP, and the
   board's real LED/UART pins.
3. Add a Node harness (`site/test_<name>.mjs`) and wire it into
   `npm test`.
4. List it in `BOARDS_OF_FIRMWARE`, add the dropdown entry in
   `site/index.html`, and add a `test_browser.mjs` marker.
