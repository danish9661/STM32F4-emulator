---
sidebar_position: 4
title: Architecture
description: How the STM32F407 emulator is structured — CPU core, Rust peripheral model, JS drivers, and how they communicate.
---

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
│   • delivery on/off selects guest ISRs vs SRAM-flag signaling     │
└───────────────┬──────────────────────────┬────────────────────────┘
                 │ emu.step(budget)         │ drainUart /
                 │                          │ dma_* / eth_* / uart_rx_byte
┌───────────────▼──────────────────────────▼────────────────────────┐
│        Rust Thumb-2 CPU + peripheral model (one WASM module)     │
│        (stm32-periph-wasm → stm32_periph_wasm_bg.wasm)           │
│   • Cortex-M4 core: exact inline exception entry/return (NVIC,   │
│     SysTick, SVC/PendSV, WFI sleep) — no hooks, no pump          │
│   • register map parsed from the vendor SVD (site/vendor/stm32f407.svd) │
│   • 41 peripheral modules (USART, GPIO, RCC, DMA, ETH, TIM, ...)  │
│   • NVIC pending/enable/active, SysTick, system clock             │
│   • System state: UART output buffer, ETH atomics, DMA queue,     │
│     INSTRUCTION_COUNT (the global clock)                          │
└──────────────────────────────────────────────────────────────────┘
```

Everything runs headless: no SDL, no native code, no hardware. The same
firmware binaries that run here also run on a real F407. (Unicorn 2.1.4
was the original CPU core and the differential-test oracle; it was removed
in AGENTS.md §23 after bit-identical parity was proven — hook/pump/wedge
notes below are marked as archaeology where they appear.)

## Execution model

Firmware is loaded at 0x08000000 (flash). SP/PC are taken from the vector
table at 0x08000000 / 0x08000004, PC is OR'd with 1 (Thumb bit).

The driver calls `emu.step(budget)` repeatedly (default budget 200000
instructions, env `MAX_BATCH` in `cli.mjs`):

- **`cli.mjs` (Node)**: batch execution — one `step()` runs up to
  `maxBatch` instructions. After each batch the driver drains UART,
  services DMA, ETH TX/RX, and feeds queued gateway frames before the
  next step. A smaller 1500-instruction budget applies at round
  boundaries (`smallBatch`, flips back on `Offer IP=`). WASM calls are
  batched too: `tick_n(5000)` every 5000 instructions keeps gateway RX
  servicing prompt.
- **Browser (`site/emulator.js`)**: one `step()` per `requestAnimationFrame`
  with a generous budget (up to 100k instructions); a full network round
  finishes in a few frames.

### The 40k-instruction wedge (archaeology, AGENTS.md §7)

The old Unicorn 2.1.4 WASM build wedged permanently if a single
`emu_start` ran ~40k+ instructions without a stop condition firing
(reproduced with empty-RX-queue recv-wait spins) — which is why the old
CLI capped batches at 20000. The Rust core has no such limit (soaks run
200k-instruction batches); the current `maxBatch` default of 200000 is a
servicing cadence, not a wedge guard. See
[progress-and-future.md](progress-and-future.md).

## MMIO routing

Peripheral accesses call straight into the WASM model — one width-correct
model call per access (`periph_read(addr, size)` /
`periph_write(addr, size, value)`), which updates register state and side
effects (interrupts, DMA queueing, ETH polls, UART TX). There are no JS
memory hooks (the Unicorn-era `HOOK_MEM_READ/WRITE` over MMIO ranges are
gone with the backend).

Bit-banding (0x42000000 aliases) is supported in the model.

## Clock and timing

There is no wall-clock in the model. `INSTRUCTION_COUNT` (an atomic u64,
advanced by the core as it executes, in 16-instruction chunks, plus
`tick_n(delta)` from the driver/sleep paths) is the global clock:

- TIM counters advance via `tick()` (instruction-count/prescaler driven);
  polled CNT reads evaluate the live count, so no tick needs to run first.
- IWDG, WWDG, RTC, ADC, RNG are self-timed — they compare
  `INSTRUCTION_COUNT` against a threshold on register access.
- SysTick programs `nvic.systick_period`; `System::tick()` re-pends the
  SysTick exception periodically.
- RCC HSE/PLL "ready" bits set after instruction-count delays.
- The post-step driver tick (`tick_peripherals()`) advances peripheral
  state without re-adding the step budget to the clock (so totals stay
  exact); `tick_n` is kept for the sleep paths.

## Interrupts

The NVIC module tracks a u128 pending mask + enable/active/priority
arrays. `set_intr_pending(irq)` sets pending only — it does NOT
auto-enable (hardware-accurate): delivery happens only when the firmware
sets the ISER bit; a disabled pending IRQ stays pending until taken or
cleared via ICPR. Drivers query `has_pending_interrupt()` (deliverable
only) and the core delivers the highest-priority enabled IRQ inline with
exact stacking. ICSR VECTPENDING reports the exception vector number.

Two delivery styles:

1. **Polling firmware (ETH demos)**: delivery stays off
   (`deliver_irqs` false). The driver signals completion by writing the
   firmware's SRAM `eth_irq_flag` and the model's DMASR bits directly;
   the firmware polls in SRAM. This is fast and safe.
2. **Interrupt-driven firmware** (`rx_interrupt_test`, `rx_crypto_test`,
   `eth_irq_test`, `freertos_test`): the driver opts in
   (`enable_irqs` / `irq_eth` / `freertos` / `lowpower`), the Rust core
   delivers the real handler inline with exact stacking, and the firmware
   owns its flags through the handler. The opt-in must NOT be enabled for
   polling ETH firmware — a real ETH_IRQHandler re-scans rx_desc and
   corrupts the driver's frame bookkeeping (see progress-and-future.md).

(Archaeology: the old `processInterrupts` guest-IRQ pump — a 32-byte
exception frame pushed by JS, the handler run via a separate `emu_start`,
context restored after the handler aborted on `bx lr` — is gone with
Unicorn. The Rust core takes exceptions inline *after* each instruction
completes, so the mid-`str` PC hazard that pump worked around cannot
occur.)

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

The Rust core drains staged mem-to-mem copies inline right after the
guest's EN store (bytes moved in guest RAM, completion latched).
Peripheral-side transfers stage a `DmaTransfer` record (direction,
stream, src/dst, size, peripheral address, PINC, PSIZE); the JS driver
performs the actual copies via
`dma_get_pending()` / `dma_set_completed(stream, ok)`, then the model
raises the stream IRQ and latches TCIF/HTIF (consumed via IFCR w1c
semantics).

## External devices

Taps and device images are registered from JS **before `init_svd`**
(the Spi/I2c peripherals snapshot their device lists at construction):

| Device | Bus | Behavior |
|---|---|---|
| SPI NOR flash (`ext_devices/SpiFlash`) | SPI | JEDEC ID (0x9F), device ID (0x90), status regs, ReadData/FastRead streaming, WriteEnable-gated PageProgram/SectorErase4k committing on CS deassert (program ANDs bits, erase sets 0xFF, WEL auto-clears like real W25Q); `dummy_pending` dummy byte while command/address clock in |
| I²C EEPROM (`ext_devices/I2cEeprom`) | I2C | 1/2-byte address phase, byte read/write into RAM copy |
| Pointer regfile (`ext_devices/I2cRegfile`, DS3231 RTC shape) | I2C | auto-increment pointer, JS seeds/reads via `i2c_regfile_get`/`set` |
| SPI tap (`spi_tap`/`spi_take_events`/`spi_push_miso`) | SPI | protocol-agnostic slave: CS/DC edges + bytes queued for JS, JS answers reads — the oled/tft/custom-device bridge |
| I²C tap (`i2c_register_slave`/`i2c_take_tx`/`i2c_push_rx`) | I2C | protocol-agnostic slave: START/STOP + bytes queued for JS, JS answers reads |
| QSPI flash (`qspi_register_flash`) | QSPI | indirect read/write image bound before init |
| FSMC bank taps (`ext_devices.fsmcDevices`) | FSMC | report access address + value (8080-mode RS/DC decode) |
| Custom SPI/I2C devices (`ext_devices.spiDevices`/`i2cDevices`) | SPI/I2C | embedder-defined bus protocols on the same taps |
| DCMI camera feed (`dcmi_feed_frame`) | DCMI | JS-fed frames consumed by the capture pipeline |

(The old `ext_devices/{lcd,touchscreen,usart_probe,display}.rs`
protocol-specific models were removed in favor of these generic taps —
real device behavior lives in JS in `site/emulator.js`.)

## WASM exports (peripheral model)

`init_svd`, `periph_read/write`, `tick_n`, `tick_peripherals`,
`has_pending_interrupt`, `get_fpu_state`/`set_sreg`/`set_fpscr`,
`dma_get_pending`/`dma_set_completed`, `gpio_set_input`,
`is_watchdog_reset_requested`, `uart_rx_byte`, `get_uart_output`,
`eth_is_tx_poll`/`eth_get_tx_desc_addr`/`eth_clear_tx_poll`,
`eth_is_rx_poll`/`eth_get_rx_desc_addr`/`eth_clear_rx_poll`,
`eth_tx_done`, `eth_rx_done`, `eth_signal_rx_poll`, `uart_set_cts`,
`uart_fault_rx`, `uart_idle`, `uart_break_pending`, `uart_lin_break`,
`uart_sc_nack`, `uart_irda_rx`, `spi_slave_select`/`spi_slave_clock`,
`sdio_bus_width`, `rtc_tamper_pin`, `flash_rdp_level`, `rng_seed_entropy`,
plus the device-tap and scope-probe families.

Full details: [peripherals.md](peripherals.md), [usage.md](usage.md).
