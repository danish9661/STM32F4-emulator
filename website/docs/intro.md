---
slug: /
sidebar_position: 1
title: Introduction
---

# STM32F4 Emulator

An STM32F407 microcontroller emulator that runs **real Cortex-M4 firmware** in the browser or Node.js. No SDL, no native code, no hardware required.

## What is it?

A complete STM32F407 emulation combining:

- **Unicorn 2.1.4** — QEMU-derived ARM Cortex-M4 CPU core, compiled to WebAssembly
- **Rust peripheral model** — 33 peripherals (USART, GPIO, DMA, ETH, TIM, CAN, ADC, DAC, I2S, LTDC, ...) compiled to WASM via `wasm-bindgen`
- **JavaScript drivers** — CLI (Node.js) and browser console with live UART, GPIO, packet viewer

The same firmware binaries that run here also run on real F407 hardware.

## Live Demo

**[Launch the emulator](https://danish9661.github.io/stm32F4-emulator/)** — pick a firmware from the dropdown, or upload your own `.bin`/`.hex`/`.elf`.

## Features

| Feature | Status |
|---|---|
| ARM Cortex-M4 Thumb-2 execution | Complete |
| 33 peripheral modules (28 detailed) | Complete |
| Networking (DHCP + TCP + HTTP) | Complete |
| Real gVisor network stack | Complete |
| DOOM in the browser (~25 FPS) | Complete |
| Interrupt-driven firmware | Complete |
| SPI/I2C/GPIO bus taps | Complete |
| DMA transfers | Complete |
| ADC/DAC/RNG/RTC | Complete |
| Watchdog (IWDG + WWDG) | Complete |
| LTDC display scanout | Complete |
| I2S/SAI audio (WAV-backed) | Complete |
| CAN bus (two-node arbitration) | Complete |
| WebSocket bridge (browser ↔ Node) | Complete |
| MCP server (AI agent integration) | Complete |

## Quick Start

### Browser (zero install)

1. Open the [live demo](/stm32F4-emulator/console/)
2. Select a firmware preset (e.g., `blinky` for LED blink, `eth_http` for networking)
3. Click **Boot preset**
4. Watch the UART terminal output

### Node.js

```bash
npm install stm32f4-emu
```

```js
import { createSTM32F407, decodeFirmware } from 'stm32f4-emu';

const mcu = await STM32F4.create({
  firmware: decodeFirmware('blinky'),  // or loadBin/loadHex/loadELF
});

mcu.usart.onData = (bytes) => process.stdout.write(bytes);
mcu.execute(5_000_000);  // run 5M instructions
mcu.close();
```

### CLI

```bash
cd stm32-periph-wasm/pkg
node cli.mjs ../../eth_http/eth_http.bin 10000000
```

## Architecture Overview

```
┌──────────────────────────────────────────────────────────────┐
│                    Driver (JS)                                │
│   cli.mjs (Node)  or  site/emulator.js + app.js (browser)   │
└───────────────────────┬──────────────────────────────────────┘
                        │ hooks
┌───────────────────────▼──────────────────────────────────────┐
│              Unicorn 2.1.4 (QEMU WASM)                       │
│              ARM Cortex-M4 — Thumb-2 firmware                │
└───────────────────────┬──────────────────────────────────────┘
                        │ periph_read / periph_write
┌───────────────────────▼──────────────────────────────────────┐
│          Rust peripheral model (→ WASM)                       │
│   33 modules · SVD register map · NVIC · DMA · ETH           │
└──────────────────────────────────────────────────────────────┘
```

See [Architecture](./architecture.md) for the full breakdown.

## Bundled Firmwares

| Firmware | Peripheral | Description |
|---|---|---|
| `blinky` | GPIO | LED blinker on PA5, UART output |
| `eth_http` | ETH + DMA | DHCP + TCP + HTTP client |
| `eth_dhcp` | ETH | DHCP discover/offer/ack loop |
| `eth_test` | ETH | Raw TX/RX self-test |
| `adc_demo` | ADC | 10-channel ADC voltmeter |
| `dac_demo` | DAC | Sine wave on PA4 |
| `pwm_demo` | TIM | Dual-channel breathing LEDs |
| `can_test` | CAN | Loopback + two-node arbitration |
| `oled_test` | I2C | SSD1306 OLED display |
| `tft_test` | SPI | ILI9341 TFT display |
| `ltdc_test` | LTDC | Layer scanout to canvas |
| `rtc_test` | I2C | DS3231 RTC time read/write |
| `watchdog_demo` | IWDG | Watchdog reset + reboot |
| `deep_sleep_demo` | PWR | STOP mode + RTC wakeup |
| `doom` | ETH + I2S | DOOM 1 shareware in the browser |

## Links

- [GitHub Repository](https://github.com/danish9661/stm32F4-emulator)
- [npm Package](https://www.npmjs.com/package/stm32f4-emu)
- [Live Demo](/stm32F4-emulator/console/)
- [DOOM](/stm32F4-emulator/console/doom.html)
