# Usage: CLI and browser

Two ways to drive the emulator: the Node CLI (`cli.mjs`) for headless runs,
soaks, and CI, and the browser console (`site/`) for interactive demos.

---

## CLI (Node, headless)

### Prerequisites

- Node 22+.
- The WASM bindings + Unicorn in `stm32-periph-wasm/pkg/` (committed; only
  rebuild if you change the Rust model — see [Building](#building)).
- For gateway runs: the built gateway `openhw-local-gateway/openhw-gw`
  (build with `cd openhw-local-gateway && go build -mod=vendor -o openhw-gw .`),
  and (for eth_http) an HTTP server at 127.0.0.1:8092 that the firmware
  will GET. `scripts/verify_ethernet.sh` starts one itself.

### Quick start

```bash
cd stm32-periph-wasm/pkg

# 1) netsim-free quick run: just boots the firmware and runs 10M instructions
node cli.mjs ../../eth_http/eth_http.bin 10000000

# 2) full gateway run (real gVisor network; needs :8092 HTTP server up)
node cli.mjs ../../eth_http/eth_http.bin 10000000 \
  --gateway --config=../../eth_http/config.yaml

# 3) connect to an ALREADY RUNNING gateway process (external, no spawn)
node cli.mjs ../../eth_http/eth_http.bin 10000000 \
  --connect --config=../../eth_http/config.yaml
```

### Arguments

| Argument | Meaning |
|---|---|
| `<firmware.bin>` | firmware binary (positional). In config mode the config's `load:` file wins |
| `[max_instructions]` | instruction budget (default 1_000_000; env `MAX_INST`) |
| `--config=<path>` | YAML config: SVD, memory regions, load file, devices, patches. Repeatable |
| `--gateway` | spawn and connect the local `openhw-gw` gateway (real gVisor stack) |
| `--connect` | connect to an external gateway (driver cannot kill it; uses `RESET` control messages) |
| `--regs` | dump register state (env `SHOW_REGS=1`) |
| `--uart=<addr>` | USART base address (default 0x40011000; env `UART_ADDR`) |

### Config file (`eth_http/config.yaml` pattern)

```yaml
cpu:
  svd: ../saturn/stm32f407.svd     # register map
  vector_table: 0x08000000
regions:
  - name: ROM
    start: 0x08000000
    load: eth_http.bin            # firmware (path relative to the yaml)
    size: 0x100000
  - name: RAM
    start: 0x20000000
    size: 0x20000
devices:
  usprt_probe:
    - peripheral: USART1          # wire the usart-probe ext device
```

### Environment variables

| Env | Default | Effect |
|---|---|---|
| `MAX_BATCH` | 20000 | instructions per `emu_start` (must stay < ~40k — Unicorn WASM wedge, see progress-and-future.md) |
| `MAX_INST` | 1M | instruction budget when the positional arg is omitted |
| `TICK_EVERY` | 5000 | tick_n()/watchdog/interrupt check interval (instructions) |
| `POLL_EVERY` | 1000 | DMA/ETH poll check interval (instructions) |
| `GW_RESTART` | 0 | 1 = restart gateway per round (legacy) |
| `GW_PATH` | repo-relative | path to the `openhw-gw` binary |
| `RX_HEX` | 0 | 1 = dump first 64 B of each injected RX frame |
| `DBG_TX` / `DBG_RX` | 0 | 1 = trace TX/RX frames |
| `DBG_FLAG` / `DBG_IRQF` / `DBG_PC` / `DBG_GW` / `DBG_DMA` | 0 | diagnostics (flag writes, ISR state, PC trace, gateway events, DMA regs) |
| `SOAK_STATS` | 0 | 1 = print soak stats at the end |
| `UART_ADDR` | 0x40011000 | USART base for TX/RX injection |
| `SHOW_REGS` | 0 | 1 = dump registers |

### Other firmwares

```bash
node cli.mjs ../../eth_dhcp/eth_dhcp.bin 10000000 --gateway --config=../../eth_dhcp/config.yaml
node cli.mjs ../../eth_test/eth_test.bin  10000000 --gateway --config=../../eth_test/config.yaml
node cli.mjs ../../blinky/blinky.bin 10000000
```

Watch for the success markers: `TCP connected` / `=== HTTP <len>b ===`
(eth_http), `DHCP SUCCESS` (eth_dhcp), `ETH Test: done` (eth_test).

### Regression script

```bash
scripts/verify_ethernet.sh [max_inst]   # all 3 firmwares + HTTP server, exit 0 = pass
```

---

## Browser (site/, interactive)

### Run locally

```bash
python3 -m http.server 8123 --directory site
# open http://127.0.0.1:8123
```

`file://` will NOT work — the SVD and WASM are fetched at runtime.

Or in the repo root: `npm run serve`.

### Deployed demo

https://danish9661.github.io/stm32F4-emulator/ (GitHub Pages, CI-deployed).

### What you get

- **Preset dropdown** — 31 bundled firmwares. Auto-boot with
  `?fw=eth_http`, `?fw=blinky`, `?fw=crypto_test`, … (or `?fw=<name>`
  for any preset).
- **UART terminal** — firmware TX scrolls here; the input box sends bytes
  to the emulated USART (RX works — verified end-to-end). HTML spec strips
  CR/LF from `<input>`, so newline-terminated RX firmwares
  (`rx_interrupt_test`, `rx_crypto_test`) need the newline sent via the
  debug handle: `window.__emu.sendUart([...bytes, 10])` from the devtools
  console.
- **Custom firmware upload** — `.bin`, Intel `.hex`, `.elf` (RAM segments
  preloaded; symbols from symtab), and `.map` files (symbols only).
- **Run / Stop / Reset** — Reset sends the gateway `RESET` control message
  when connected.
- **Gateway URL field** — WebSocket to `ws://host:port/api/network-gateway`
  (real gVisor stack). Connected: all TX frames go to the real network and
  RX frames are injected from it. Disconnected: canned `netsim` fallback.
  NOTE: GitHub Pages is https, which blocks plain `ws://` — use a locally
  served page for gateway mode.
- **GPIO pin grid** (banks A–E) — live MODER/ODR/IDR readout; the blinky
  preset's PA5 toggles visibly.
- **Key peripheral registers** — ETH DMASR/MACCR, USART1 SR, RCC AHB1ENR.
- **Packet viewer** — see TX frames the firmware emits.

### DOOM page (`site/doom.html`)

A second page (linked from the console's header/footer) that runs DOOM 1
shareware on the emulated F407 — `site/doom1.wad` (4.2 MB) is loaded into
8 MB of `extra_mem` by the driver, and the guest renders the CMAP256
framebuffer to a 640×400 canvas at ~24 MIPS / ~24 FPS with audio (I2S
mixer → AudioWorklet, 11025 Hz).

- **Controls**: move W/S/A/D + arrows · strafe Shift · fire Ctrl · use
  Space · menu Enter/Esc · save F2 (menu) / F6 (quick-save) · load F3
  (menu) / F9 (quick-load, then 'y' to confirm).
- **Save/load**: the firmware stages savegames to EXTRAM (0xC0080000,
  2 slots × 256 KB) and `doom.js` mirrors them to
  `localStorage['doom-save-0'|'doom-save-1']` — saves survive page
  reloads, and `saveMap` is restored at boot so the load menus show them.
- Terminal: the UART box under the screen shows the guest's boot prints
  and save/load confirmations (`SAVE ok slot=0 bytes=…`, `LOAD ok …`).
- Node regression: `node site/test_doom.mjs` (boot → menu → E1M1 →
  quick-save flow).

### Browser debug handles

`app.js` exposes `window.__emu` / `window.__bindings` for automation
(e.g. headless-Chrome CDP drivers). Useful entries:

```js
window.__emu.sendUart(bytes)          // inject UART RX bytes
window.__emu.step(maxInst)            // run one step
window.__emu.drainUart()              // pull TX output
window.__emu.injectFrame(bytes)       // inject an Ethernet frame
window.__emu.getRegisters()           // { R0..R12, SP, LR, PC, XPSR }
window.__emu.close()                  // tear down
window.__bindings.read32(addr)        // raw model read
```

### Browser smoke tests (Node drivers, headless Chrome)

- `/tmp/opencode/site_smoke.mjs` — CDP driver for the console page
  (boots a preset, asserts banner + a round).
- `/tmp/opencode/rx_smoke.mjs` — sends UART bytes via `window.__emu
  .sendUart([...])` and asserts the interrupt-driven CRC output.
- The same page logic is unit-tested in Node without a browser:
  `node site/test_flow.mjs`, `node site/test_blinky.mjs`,
  `node site/test_rx_interrupt.mjs`.

---

## Using the emulator as a library (npm package)

```js
import { createSTM32F407, createNetSim, FIRMWARES } from 'stm32f4-emu';

const netsim = createNetSim();                 // canned DHCP/TCP/HTTP peer
const emu = await createSTM32F407({
    firmware: FIRMWARES.eth_http.bytes,        // or any STM32F4 firmware blob
    onTx: (frame) => {
        for (const reply of netsim.onTx(frame)) emu.injectFrame(reply);
    },
});
emu.step(100000);
console.log(emu.drainUart());
emu.close();
```

`npm pack` produces the tarball (16 files, ~1.2 MB); install from the
tarball to consume it. The package is **not yet published** to the npm
registry.

---

## Building from source

```bash
# Rust peripheral model
cd stm32-periph-wasm
wasm-pack build --release --target nodejs            # Node (pkg/)
wasm-pack build --release --target web --out-dir ../site/vendor   # browser
rm -f ../site/vendor/.gitignore                      # wasm-pack writes '*'

# firmware (bare-metal Makefiles, toolchain from the Arduino core)
TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-" \
  make -C eth_http          # also eth_dhcp, eth_test, blinky, ...

# gateway
cd openhw-local-gateway && go build -mod=vendor -o openhw-gw .
```

Full detail: [architecture.md](architecture.md), [peripherals.md](peripherals.md),
[benchmarks.md](benchmarks.md), [progress-and-future.md](progress-and-future.md),
AGENTS.md.
