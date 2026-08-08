# STM32F4 Emulator

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
firmware: a preset dropdown with 20 bundled binaries (network demos, a
bare-metal LED blinker, peripheral/crypto/UART test binaries), custom
firmware upload (`.bin`, Intel `.hex`, `.elf` — with loadable RAM segments
and symbols — plus `.map` for a symbol table), Run/Stop/Reset, a **gateway
URL field** to connect a real network stack (openhw-gw + gVisor) with a
scripted network (netsim) as fallback, a live UART terminal, GPIO pin
readout for banks A–E, and key peripheral registers. For automation, a
preset can auto-boot via the URL: `?fw=eth_http`, `?fw=blinky`, `?fw=crypto_test`, …

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

`npm pack` produces `stm32f4-emulator` — the full emulator as a library,
with all WASM assets, the SVD register map, and the firmware binaries
bundled:

```js
import { createSTM32F407, createNetSim, FIRMWARES } from 'stm32f4-emulator';

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

## Firmwares

| Firmware | What it does | Success marker |
|---|---|---|
| `eth_http/` | DHCP + TCP client + HTTP GET + prints the response | `TCP connected`, `=== HTTP <len>b ===` |
| `eth_dhcp/` | Loops DHCP Discover/Offer/Request/Ack | `DHCP SUCCESS` |
| `eth_test/` | Raw ETH TX/RX self-test | `ETH Test: done` |
| `blinky/` | **No ethernet** — LED blinker on GPIOA PA5 + UART tick counter | `tick N LED=ON/OFF` |

Plus 16 more test binaries (`crypto_test`, `hal_test`, `timer_test`,
`periph_test`, `echo_test`, `blink_serial`, …) from the `*_test/` directories
— all boot headless and print a banner over UART (probe:
`node site/probe_firmwares.mjs`).

All three are bare-metal (no RTOS), built with the Arduino core's
arm-none-eabi-gcc, and driven purely through memory-mapped registers —
the same firmware binaries run on the emulator and on real silicon.

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
│   ├── emulator.js          Import-free emulator factory (Node + browser)
│   ├── netsim.js            Canned DHCP/TCP/HTTP network peer (fallback)
│   ├── loaders.js           Intel HEX / ELF32 / linker-map parsers
│   ├── test_flow.mjs        Node E2E flow test (npm test)
│   └── vendor/              Browser WASM build, SVD, Unicorn
├── index.mjs, package.json  npm package entry (stm32f4-emulator)
├── tools/make_firmware.mjs  Regenerates site/firmware.js from eth_*/.bin
├── stm32-periph-wasm/       Rust peripheral model (WASM build + pkg/)
├── eth_http/ eth_dhcp/ eth_test/   Sample network firmwares + configs
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
  (`site/test_blinky.mjs`), exit 0 = all PASS.
- `scripts/verify_ethernet.sh [max_inst]` — runs all three firmwares through
  the gateway, asserts the success markers and 0 `TCP fail`.
- Soak-tested: 200M-instruction gateway runs with 1000+ consecutive TCP
  rounds, 0 failures (details in AGENTS.md §10).

## License

GPL-3.0-only. See [LICENSE](LICENSE).

*This repository is a fork/continuation of
[nviennot/stm32-emulator](https://github.com/nviennot/stm32-emulator), which
emulated 3D-printer firmwares (Elegoo Saturn, Anycubic Mono X) in a native
SDL app. The WASM headless emulator, network firmwares, browser demo, and npm
package are new work built on that base.*
