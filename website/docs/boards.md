---
sidebar_position: 3
title: Boards
description: Supported STM32F4 boards and chips — what is implemented, what was verified, and what is left.
---

# Supported Boards & Chips

One shared Cortex-M4F CPU core runs every board byte-identically — a board
differs only in SVD (register map), flash/RAM sizes, and clock
(`site/boards.js` — no decoder work per board, see `docs/architecture.md`).
Timing stays instruction-budget based on every chip.

| Board / chip | `board` name | Flash/RAM | Clock | Notes |
|---|---|---|---|---|
| BlackPill STM32F401CC | `stm32f401` | 512K/96K | 84 MHz | superset sizes cover the biggest die (RE); smaller-die firmware runs unchanged, SP comes from its own vector table |
| Nucleo-F401RE | `stm32f401` | 512K/96K | 84 MHz | Arduino headers; `Serial` = USART2 |
| BlackPill STM32F411CE | `stm32f411` | 512K/128K | 100 MHz | +SPI5 over F401 |
| Nucleo-F411RE | `stm32f411` | 512K/128K | 100 MHz | Arduino headers; `Serial` = USART2 |
| Discovery STM32F407VG (default) | `stm32f407` | 1M/192K | 168 MHz | reference target; 128K SRAM + 64K CCM mapped flat |
| Black STM32F407VE | `stm32f407ve` | 512K/192K | 168 MHz | same die, new package; LED PA6, `Serial` = USART1 |
| Black STM32F407ZE | `stm32f407ve` | 512K/192K | 168 MHz | LED PF10, `Serial` = USART1 |
| Discovery STM32F429ZI | `stm32f429` | 2M/256K | 180 MHz | +DMA2D/LTDC/SAI/GPIOK/QSPI; FMC instead of FSMC |

DBGMCU IDCODE reads a constant (`0x10006411`) on every map — the model
does not vary it per chip.

## Per-chip notes (audited)

Silicon peripheral set vs emulator coverage, wiring, demos and quirks —
one page per chip:

- [STM32F401](boards/stm32f401.md) — 36 presets, no CAN/ETH/DAC/DCMI/FSMC
- [STM32F411](boards/stm32f411.md) — 36 presets, F401 + SPI5
- [STM32F407](boards/stm32f407.md) — 64 presets, reference target
- [STM32F407VE/ZE](boards/stm32f407ve.md) — 66 presets, 512K flash package
- [STM32F429](boards/stm32f429.md) — 57 presets, DMA2D/LTDC/Ethernet proofs

Preset counts come from inverting `BOARDS_OF_FIRMWARE` in
`site/boards.js`. The console filters the preset menu to the selected
board (board-only demos hide on other chips; picking one auto-switches
the chip), and deep links work as `console.html?board=stm32f401` /
`console.html?fw=arduino_disco_f407vg`. The UART input box follows the
board's native Serial port (`uartAddr`: USART2 `0x40004400` on
Nucleo/Disc-F407).

## Verification matrix

- Arduino-cli firmware per target boots + ticks + LED toggles:
  `node site/test_arduino_boards.mjs` **8/8** (BlackPill/Nucleo F401/F411,
  Discovery F407VG/F429ZI, Black F407VE/ZE — USART1 or USART2 per board).
- Portable demos on every compatible map: `node
  site/test_board_matrix.mjs` (in `npm test`) — only passing
  board+firmware pairs become presets.
- Ethernet (F407/F429): gateway trio + netsim flows, `scripts/verify_ethernet.sh`.
