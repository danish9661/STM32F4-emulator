# Architecture

This document explains how the STM32F407 emulator is put together: the CPU
core, the Rust peripheral model, the JS drivers, and how the pieces talk to
each other.

## Overview

```
┌──────────────────────────────────────────────────────────────────┐
│                        Driver (JS)                                │
│   cli.mjs (Node)  or  site/emulator.js + app.js (browser)         │
│                                                                    │
│   • steps the CPU      • drains UART output                       │
│   • injects RX frames  • services ETH TX/RX protocol              │
│   • optional guest-IRQ pump (interrupt-driven firmware only)      │
└───────────────┬──────────────────────────┬────────────────────────┘
                │ uc.emu_start / hooks      │ get_uart_output /
                │                            │ dma_* / eth_* / uart_rx_byte
┌───────────────▼──────────────────────────▼────────────────────────┐
│                 Unicorn 2.1.4 (QEMU-derived, WASM)                │
│                 ARM Cortex-M4 core — executes Thumb-2 firmware    │
│   HOOK_MEM_READ/WRITE on MMIO ranges → routed into the model      │
│   HOOK_BLOCK (cli.mjs) → instruction counting + stop conditions   │
└───────────────┬───────────────────────────────────────────────────┘
                │ periph_read / periph_write (wasm-bindgen exports)
┌───────────────▼───────────────────────────────────────────────────┐
│        Rust peripheral model (stm32-periph-wasm, →WASM)          │
│   • register map parsed from the vendor SVD (monox/stm32f407.svd) │
│   • 33 peripheral modules (USART, GPIO, RCC, DMA, ETH, TIM, ...)  │
│   • NVIC pending/enable/active, SysTick, system clock             │
│   • System state: UART output buffer, ETH atomics, DMA queue,     │
│     INSTRUCTION_COUNT (the global clock)                          │
└───────────────────────────────────────────────────────────────────┘
```

Everything runs headless: no SDL, no native code, no hardware. The same
firmware binaries that run here also run on a real F407.

## Execution model

Firmware is loaded at 0x08000000 (flash). SP/PC are taken from the vector
table at 0x08000000 / 0x08000004, PC is OR'd with 1 (Thumb bit).

The driver calls `uc.emu_start(pc|1, 0, 0, budget)` repeatedly:

- **`cli.mjs` (Node)**: *batch execution* — one `emu_start` runs up to
  `maxBatch` instructions (default 20000, env `MAX_BATCH`). A block hook
  counts instructions and can stop the batch early when the model signals
  work (DMA pending, ETH TX/RX poll, pending interrupt, RX queue non-empty
  with RX poll armed). WASM calls are batched too: `tick_n(5000)` every
  5000 instructions, poll checks every 1000 (env `POLL_EVERY`/`TICK_EVERY`).
  After each batch the driver services DMA, ETH TX/RX, and optionally
  interrupts.
- **Browser (`site/emulator.js`)**: one `step()` per `requestAnimationFrame`
  with a generous budget (up to 100k instructions); a full network round
  finishes in a few frames.

### The 40k-instruction wedge (important constraint)

This Unicorn 2.1.4 WASM build wedges permanently if a single `emu_start`
runs ~40k+ instructions without a stop condition firing (reproduced with
empty-RX-queue recv-wait spins). This is why `maxBatch` defaults to 20000 —
see [progress-and-future.md](progress-and-future.md).

## MMIO routing

Memory hooks are registered over narrow ranges (whole peripheral space
0x40000000–0xB0000000 and system space 0xE0000000–0xE1000000 in the
browser build; narrower ranges in cli.mjs). Each hooked read/write calls:

- `periph_read(addr, size)` → model returns the register value → driver
  writes it back into guest memory with `uc.mem_write` so the firmware sees
  it.
- `periph_write(addr, size, value)` → model updates register state and any
  side effects (interrupts, DMA queueing, ETH polls, UART TX).

Bit-banding (0x42000000 aliases) is supported in the model.

## Clock and timing

There is no wall-clock in the model. `INSTRUCTION_COUNT` (an atomic u64,
incremented by `tick_n(delta)` from the driver) is the global clock:

- TIM counters advance via `tick()` (instruction-count/prescaler driven).
- IWDG, WWDG, RTC, ADC, RNG are self-timed — they compare
  `INSTRUCTION_COUNT` against a threshold on register access.
- SysTick programs `nvic.systick_period`; `System::tick()` re-pends the
  SysTick exception periodically.
- RCC HSE/PLL "ready" bits set after instruction-count delays.

## Interrupts

The NVIC module tracks a u128 pending mask + enable/active/priority arrays.
Peripherals call `set_intr_pending(irq)` (which auto-enables the IRQ in the
model). Drivers query `has_pending_interrupt()` / `get_next_pending_interrupt()`.

Two delivery styles:

1. **Polling firmware (ETH demos)**: the driver *never* runs guest ISRs.
   It signals completion by writing the firmware's SRAM `eth_irq_flag` and
   the model's DMASR bits directly; the firmware polls in SRAM. This is
   fast and safe.
2. **Interrupt-driven firmware** (`rx_interrupt_test`, `rx_crypto_test`):
   an opt-in *guest-IRQ pump* (`processInterrupts`) pushes a 32-byte
   exception frame on the stack, runs the real handler via
   `uc.emu_start(handler_pc, …)`, and restores the full context *including
   XPSR* (condition flags!) after the handler aborts on `bx lr`
   (EXC_RETURN unsupported). The pump must NOT be enabled for ETH firmware
   — a real ETH_IRQHandler re-scans rx_desc and corrupts the driver's
   frame bookkeeping (see progress-and-future.md).

## Ethernet flow

- **TX**: firmware writes DMATPDR → model sets `ETH_TX_POLL` + descriptor
  address → driver stops the batch, captures the packet from the TX
  descriptor (`tx_desc[0][0]&0x3FFF` = length, `tx_desc[0][1]` = data
  pointer), forwards it to the network peer (netsim or gateway), calls
  `eth_tx_done()` and writes `eth_irq_flag |= 1` in guest SRAM.
- **RX**: driver injects a frame into the RX ring (`rx_buf` + descriptor
  ownership flip, or the RX queue in the browser build), sets
  `ETH_IRQ_FLAG |= 2`, and the firmware's `eth_recv_packet` picks it up.
  Descriptor index rotates across injections so consecutive frames land in
  different `rx_buf` slots (avoids buffer-clobber races in the guest).

## Network peers

- **netsim** (`site/netsim.js`): canned DHCP Offer/Ack + TCP SYN-ACK +
  HTTP response + bare ACK. Deterministic, no network.
- **openhw-local-gateway** (`openhw-local-gateway/`, Go + gVisor): a real
  network stack. The driver connects over a WebSocket
  (`/api/network-gateway`); binary frames are raw Ethernet both ways, a
  text `RESET` frame tears down the gVisor session table. Firmware then
  talks to a real TCP peer (e.g. an HTTP server on 127.0.0.1:8092).

## DMA

Rust queues `DmaTransfer` records (direction, stream, src/dst, size) on
EN; the JS driver performs the actual memory copies via
`dma_get_pending()` / `dma_set_completed(stream, ok)`, then the model
raises the stream IRQ and sets TCIF/HTIF.

## External devices

`ExtDevices` (registered from JS before init) attach to peripherals:

| Device | Bus | Behavior |
|---|---|---|
| SPI NOR flash (`spi_flash.rs`) | SPI | JEDEC ID, read data streaming; program/erase parsed but not implemented |
| I²C EEPROM (`i2c_eeprom.rs`) | I2C | 1/2-byte address phase, byte read/write into RAM copy |
| 128×64 LCD (`lcd.rs`) | SPI | 0xFB command → drawing mode, cursor advance |
| 240×240 display (`display.rs`) | FSMC/parallel | 0x2A/0x2B window, 0x2C drawing, canned read replies |
| Touchscreen (`touchscreen.rs`) | GPIO | touch-detect pin callback, reads 0 |
| USART probe (`usart_probe.rs`) | SPI lookup | buffers bytes, logs lines |

## WASM exports (peripheral model)

`init`, `init_svd`, `periph_read/write`, `tick`, `tick_n`,
`has_pending_interrupt`, `get_next_pending_interrupt`,
`dma_get_pending_count/pending/set_completed`, `gpio_read_output/set_input/read_input`,
`is_watchdog_reset_requested`, `uart_rx_byte`, `get_uart_output`,
`eth_is_tx_poll/get_tx_desc_addr/clear_tx_poll`, `eth_is_rx_poll/...`,
`eth_tx_done`, `eth_rx_done`, `eth_signal_rx_poll`, `eth_signal_tx_poll`,
`add_spi_flash`, `add_i2c_eeprom`, `add_software_spi`.

Full details: [peripherals.md](peripherals.md), [usage.md](usage.md).
