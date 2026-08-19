# STM32F4 Emulator (`stm32f4-emu`)

[![npm version](https://img.shields.io/npm/v/stm32f4-emu.svg)](https://www.npmjs.com/package/stm32f4-emu)
[![npm downloads](https://img.shields.io/npm/dm/stm32f4-emu.svg)](https://www.npmjs.com/package/stm32f4-emu)
[![License: GPL v3](https://img.shields.io/badge/License-GPLv3-blue.svg)](LICENSE)
[![CI](https://github.com/danish9661/stm32F4-emulator/actions/workflows/ci.yml/badge.svg)](https://github.com/danish9661/stm32F4-emulator/actions/workflows/ci.yml)
[![Pages](https://github.com/danish9661/stm32F4-emulator/actions/workflows/pages.yml/badge.svg)](https://github.com/danish9661/stm32F4-emulator/actions/workflows/pages.yml)

An STM32F407 microcontroller emulator that runs real Cortex-M4 firmware. It
combines a **Unicorn CPU core** (QEMU-derived, compiled to WASM) with a
**Rust peripheral model** (RCC, USART, GPIO, DMA, ETH, TIM, NVIC, ...) also
compiled to WASM — so the whole machine runs headless in **Node.js or a
browser tab**, with no SDL, no native deps, no hardware.

It ships three real networking firmwares (`eth_http`, `eth_dhcp`, `eth_test`)
that do DHCP + TCP + HTTP against a simulated (or a real gVisor-backed)
network, a browser demo, and a publishable npm package.

## Live demo

The browser demo deploys to GitHub Pages:

**https://danish9661.github.io/stm32F4-emulator/**

A single console page that starts **idle** — nothing runs until you pick a
firmware: a preset dropdown with 31 bundled binaries (network demos, a
bare-metal LED blinker, peripheral/crypto/UART/SPI test binaries), custom
firmware upload (`.bin`, Intel `.hex`, `.elf` — with loadable RAM segments
and symbols — plus `.map` for a symbol table), Run/Stop/Reset, a **gateway
URL field** to connect a real network stack (openhw-gw + gVisor) with a
scripted network (netsim) as fallback, a live UART terminal (with **UART
RX input** — type into the console and the firmware reads it; newline
characters excluded per HTML spec, see AGENTS.md §11), GPIO pin readout
for banks A–E, and key peripheral registers. Interrupt-driven firmware
(`rx_interrupt_test`, `rx_crypto_test`) is serviced by an opt-in guest-IRQ
pump; polling firmware (the ETH demos) never uses it. For automation, a
preset can auto-boot via the URL: `?fw=eth_http`, `?fw=blinky`, `?fw=crypto_test`, …

## DOOM (in the browser)

**[`site/doom.html`](site/doom.html)** runs DOOM 1 shareware
(doomgeneric, ported to the emulated F407) at ~25 FPS in a
headless-Chrome-verified browser page — playable, but below DOOM's native
35 fps: at ~918k guest instructions per rendered frame, 35 fps would need
~32 MIPS and the Unicorn WASM core tops out near 20-24 (details and the
measurements in AGENTS.md §16). Because the guest mixes one frame of audio
per rendered frame, sound below 35 fps plays slightly slow and pitched-down
rather than breaking up — the worklet rate-matches instead of inserting
gaps, and the stats line reports it (`audio 0.72x`). The page: 320×200
CMAP256 framebuffer,
WASD + arrows + Ctrl/Space/Shift + F-keys, I2S audio out (mixer →
AudioWorklet at 11025 Hz), and a realtime lock that paces the guest to
wall time. **Save/load works**: the firmware stages savegames to an
EXTRAM region (2 slots × 256 KB at 0xC0080000) and `doom.js` mirrors
them to `localStorage['doom-save-N']`, so F6 (quick-save), F2 (save
menu), F9 (quick-load, 'y' to confirm) and F3 (load menu) survive page
reloads.

- Controls: move W/S/A/D + arrows · strafe Shift · fire Ctrl · use
  Space · menu Enter/Esc · save F2/F6 · load F3/F9 · F1/F10/F11/F12.
- Boot → menu → gameplay verified end-to-end by
  `node site/test_doom.mjs` (boot markers, menu navigation to E1M1,
  palette + framebuffer, W-move + turn, audio, **save → `SAVE ok slot=0`
  → load handshake**).
- `site/doom1.wad` is the 4.2 MB shareware WAD; the firmware never reads
  it from flash — the driver loads it into 8 MB of `extra_mem`
  (0xB8000000).

## Quickstart

```bash
# browser demo locally
npm run serve                  # then open http://127.0.0.1:8123

# node end-to-end flow test (boot -> DHCP -> TCP -> HTTP, 2 rounds)
npm test                       # == node site/test_flow.mjs

# gateway-backed run: firmware talks to a REAL network stack (gVisor)
cd stm32-periph-wasm/pkg
node cli.mjs ../../eth_http/eth_http.bin 10000000 \
  --gateway --config=../../eth_http/config.yaml
# (requires an HTTP server at 127.0.0.1:8092; see AGENTS.md §10)
```

## Use it as a library (npm package)

`npm pack` produces `stm32f4-emu` — the full emulator as a library,
with all WASM assets, the SVD register map, and the firmware binaries
bundled:

```js
import { createSTM32F407, createNetSim, FIRMWARES } from 'stm32f4-emu';

const netsim = createNetSim();                        // canned DHCP/TCP/HTTP peer
const emu = await createSTM32F407({
    firmware: FIRMWARES.eth_http.bytes,               // any STM32F4 firmware blob
    onTx: (frame) => {
        for (const reply of netsim.onTx(frame)) emu.injectFrame(reply);
    },
});

emu.step(100000);              // run up to 100k instructions
const uart = emu.drainUart();  // collect UART output
emu.injectFrame(packetBytes);  // inject an Ethernet frame
```

See `site/test_flow.mjs` and the exports in `index.mjs` for the full API
(`decodeFirmware`, `createEmulator`, `createNetSim` re-exports).

Attach virtual hardware (LEDs, buttons, PWM readers, analog sensors,
custom SPI/I2C devices) to pins and buses with the component API —
`emu.pin()`/`emu.watchPin()`/`emu.setAdcChannel()` plus the
`LED`/`Button`/`Pwm`/`Potentiometer`/`I2cRegisterDevice` components, or
`ext_devices.spiDevices`/`i2cDevices` for your own bus protocol. See
[docs/components.md](docs/components.md).

## Drive it from an AI agent (MCP)

`mcp/server.mjs` exposes the emulator over the Model Context Protocol —
boot firmware, step, read UART, poke pins, inject ADC values, and inspect
registers as tools an MCP client (Claude Code, Claude Desktop) can call:

```bash
npm install && npm run mcp
```

The MCP SDK is an *optional peer dependency*, so installing this package
as a library stays dependency-free; only MCP users need
`npm i @modelcontextprotocol/sdk zod`. See [docs/mcp.md](docs/mcp.md) for
the tool reference and client config.

## Firmwares

| Firmware | What it does | Success marker |
|---|---|---|
| `eth_http/` | DHCP + TCP client + HTTP GET + prints the response | `TCP connected`, `=== HTTP <len>b ===` |
| `eth_dhcp/` | Loops DHCP Discover/Offer/Request/Ack | `DHCP SUCCESS` |
| `eth_test/` | Raw ETH TX/RX self-test | `ETH Test: done` |
| `blinky/` | **No ethernet** — LED blinker on GPIOA PA5 + UART tick counter | `tick N LED=ON/OFF` |
| `doom/` | **DOOM 1 shareware** (doomgeneric F407 port, browser page `site/doom.html`) | `node site/test_doom.mjs` (boot + menu + gameplay + save/load) |

Plus 17 more test binaries (`crypto_test`, `hal_test`, `timer_test`,
`periph_test`, `echo_test`, `blink_serial`, `rx_interrupt_test`,
`spi_tft_test`, …) from the `*_test/` directories — all boot headless and
print a banner over UART (probe: `node site/probe_firmwares.mjs`).

All are bare-metal (no RTOS), built with the Arduino core's
arm-none-eabi-gcc, and driven purely through memory-mapped registers —
the same firmware binaries run on the emulator and on real silicon. That
equivalence is for **logic**, not **timing**: timers/ADC/RNG/RTC/watchdogs
are instruction-count driven rather than wall-clock driven, so firmware
relying on real-time behavior (PWM frequency matching real hardware,
watchdog timeouts close to spec) will diverge — see docs/progress-and-future.md#known-limitations.

## Architecture

```
firmware .bin ──► Unicorn WASM CPU ──► memory hooks
                          │
                    periph_read/write ──► Rust peripheral model (WASM)
                                          RCC USART GPIO DMA ETH TIM NVIC
                          │
                    UART out / ETH TX frames ──► driver (cli.mjs / site/emulator.js)
                          │
                    RX frames injected (netsim, or real gVisor gateway)
```

- **CPU**: Unicorn 2.1.4 compiled to WASM executes Thumb-2 code; every
  read/write to a hooked MMIO range is routed into the Rust model, which
  answers by writing the modeled register value back into guest memory.
- **Peripherals**: a `wasm-bindgen` crate (`stm32-periph-wasm/`); registers
  and bit fields come from the vendor SVD (`monox/stm32f407.svd`).
- **Ethernet**: TX is captured from the DMA descriptors; RX frames are
  injected into the RX ring and the firmware's `eth_irq_flag` (SRAM) drives
  polling — no interrupts required. Optionally, a Go gateway
  (`openhw-local-gateway/`) with a gVisor network stack makes the firmware
  talk to a real network: `node cli.mjs <fw.bin> <inst> --gateway`.
- **Browser build** (`site/`): same modules as ESM; `site/emulator.js` is an
  import-free universal factory; `site/netsim.js` is a canned network peer;
  `site/loaders.js` parses Intel HEX / ELF32 / linker-map files. The demo
  page's gateway mode uses the exact same protocol as the Node CLI:
  raw Ethernet frames over a WebSocket (`/api/network-gateway`), `RESET`
  control message on reboot.

## Repository layout

```
├── site/                    Single-page console + universal emulator factory
│   ├── index.html, app.js   Console UI (UART, presets, loaders, gateway, GPIO)
│   ├── doom.html, doom.js   DOOM page (gameplay + save/load via localStorage)
│   ├── emulator.js          Import-free emulator factory (Node + browser)
│   ├── components.js        Virtual components (LED/Button/Pwm/Potentiometer/...)
│   ├── netsim.js            Canned DHCP/TCP/HTTP network peer (fallback)
│   ├── loaders.js           Intel HEX / ELF32 / linker-map parsers
│   ├── test_flow.mjs        Node E2E flow test (npm test)
│   ├── test_blinky.mjs      Node blinky GPIO test (npm test)
│   ├── test_rx_interrupt.mjs Node UART-interrupt test (npm test)
│   ├── test_component_*.mjs Component-API tests, one firmware each (npm test)
│   ├── test_doom.mjs        Node DOOM boot/menu/gameplay/save test
│   └── vendor/              Browser WASM build, SVD, Unicorn
├── index.mjs, package.json  npm package entry (stm32f4-emu)
├── mcp/                     MCP server (drive the emulator from an AI agent)
├── .github/workflows/       CI (Linux/Windows/macOS test matrix) + Pages deploy
├── tools/make_firmware.mjs  Regenerates site/firmware.js from eth_*/.bin
├── stm32-periph-wasm/       Rust peripheral model (WASM build + pkg/)
├── eth_http/ eth_dhcp/ eth_test/   Sample network firmwares + configs
├── doom/                    DOOM 1 port (doomgeneric f407 target + WAD path)
├── openhw-local-gateway/    Go gateway (gVisor network stack)
├── scripts/verify_ethernet.sh   Regression runner for all three firmwares
├── src/, monox/, saturn/    Native SDL emulator (upstream heritage)
└── AGENTS.md                Full architecture, build steps, runbook
```

## Building from source

```bash
# Rust peripheral model (Node target; browser target goes to site/vendor/)
cd stm32-periph-wasm && wasm-pack build --release --target nodejs

# firmware (bare-metal Makefiles; toolchain from the Arduino core)
TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-" \
  make -C eth_http          # also eth_dhcp, eth_test

# native SDL emulator (upstream heritage, not the headless path)
cd stm32-emulator-main && cargo build --release
```

`wasm-pack` writes `site/vendor/.gitignore` containing `*` after a browser
rebuild — delete it so the vendor assets stay tracked/committed.

## Testing

- `npm test` — flow test (`site/test_flow.mjs`) + blinky test
  (`site/test_blinky.mjs`) + interrupt-UART test (`site/test_rx_interrupt.mjs`)
  + component-API tests (`site/test_component_{led,button,pwm,i2cregfile}.mjs`,
  each against real firmware — LED/blinky, Button/exti_test, Pwm/buzzer_test,
  I2cRegisterDevice/rtc_test), exit 0 = all PASS. Each test file boots
  exactly one firmware in its own `node` process — `createEmulator()`
  instances aren't safe to reuse across different firmware in the same
  process (see docs/components.md). The same suite runs in CI on
  `ubuntu-latest`/`windows-latest`/`macos-latest`
  ([.github/workflows/ci.yml](.github/workflows/ci.yml)) on every push.
- `node site/test_doom.mjs` — DOOM boot → menu → E1M1 gameplay + save/load
  (see the DOOM section above).
- `scripts/verify_ethernet.sh [max_inst]` — runs all three firmwares through
  the gateway, asserts the success markers and 0 `TCP fail`.
- Soak-tested: 200M-instruction gateway runs with 1000+ consecutive TCP
  rounds, 0 failures (details in AGENTS.md §10).

## Documentation

- [docs/architecture.md](docs/architecture.md) — how the emulator is put
  together (CPU, peripheral model, drivers, ETH flow, interrupts).
- [docs/peripherals.md](docs/peripherals.md) — all 33 peripherals and the
  level each is implemented to, plus external devices and known gaps.
- [docs/usage.md](docs/usage.md) — CLI, browser, and npm-library usage,
  config files, env vars, building.
- [docs/components.md](docs/components.md) — attach virtual LEDs, buttons,
  PWM/analog sensors, and custom SPI/I2C devices to pins/buses
  (rp2040js-style component API).
- [docs/mcp.md](docs/mcp.md) — the MCP server: drive the emulator as tools
  from Claude Code / Claude Desktop or any MCP client.
- [docs/benchmarks.md](docs/benchmarks.md) — throughput numbers, soak
  results, tunables.
- [docs/progress-and-future.md](docs/progress-and-future.md) — status,
  known limitations, roadmap.

## License & Credits

- **License**: GPL-3.0-only. See [LICENSE](LICENSE).
- **Unicorn CPU Core**: Powered by [Unicorn.js](https://github.com/AlexAltea/unicorn.js) by [Alex Altea](https://github.com/AlexAltea) (WASM/JS port of the [Unicorn Engine](https://www.unicorn-engine.org/) CPU emulator, derived from QEMU, licensed under GPLv2).
- **Heritage**: Fork and continuation of [nviennot/stm32-emulator](https://github.com/nviennot/stm32-emulator) (native SDL 3D printer emulator by Nicolas Viennot). The headless WASM peripheral model, networking stack, browser demo, virtual components API, MCP server, and npm package are new work built on that base.
- **DOOM**: Ported using [doomgeneric](https://github.com/ozkl/doomgeneric) by Ozkan Sezgin.
