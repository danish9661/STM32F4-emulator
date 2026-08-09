# Progress and future work

Status as of 2026-08-09. The authoritative living log is AGENTS.md; this
document is the readable summary.

## What works today

### Emulator core
- Full Cortex-M4 Thumb-2 execution via Unicorn 2.1.4 (WASM) — the same
  firmware binaries run here and on real silicon.
- 33 peripheral modules, 26 detailed (see [peripherals.md](peripherals.md)).
- Deterministic instruction-count clock (timers, ADC, RNG, RTC, watchdogs).
- NVIC interrupt model with an opt-in guest-IRQ pump for interrupt-driven
  firmware (UART RX + crypto firmwares verified in Node and in a browser).
- Register map from the vendor SVD; bit-banding support.

### Networking (the flagship feature)
- Three real bare-metal network firmwares:
  - `eth_http` — DHCP + TCP client + HTTP GET, prints the response
  - `eth_dhcp` — DHCP loop, prints `DHCP SUCCESS`
  - `eth_test` — raw ETH TX/RX self-test
- Two network peers: canned `netsim` (deterministic) and a real gVisor
  stack via `openhw-local-gateway` (Go). The firmware does real DHCP and
  real TCP against a real kernel network stack over a WebSocket.
- Multi-round soaks: 200M-instruction runs, 1000+ consecutive TCP
  connections with 0 failures (see [benchmarks.md](benchmarks.md)).

### Browser demo + npm package
- Single-page console (site/): UART terminal with **bidirectional RX**,
  21 firmware presets, custom `.bin`/`.hex`/`.elf`/`.map` upload, gateway
  connection to real gVisor networking, live GPIO grid, peripheral register
  readout, packet viewer. Deployed to GitHub Pages.
- `stm32f4-emulator` npm package (packed, not published) with a clean
  `createSTM32F407({firmware})` API.
- CI runs the Node test suite on every push; Pages deploys the demo.

### Recently fixed (2026-08-09)
- **Unicorn 40k-instruction wedge**: one `emu_start` running ~40k+
  instructions without a stop condition permanently wedges this WASM build
  (broken timeout path, `qemu_thread_create: Not supported`). Fix:
  `maxBatch` capped at 20000 (`MAX_BATCH` env). After the cap: 604 TCP
  connected / 0 fail / 0 timeouts in 47.9 s (~12.6 rounds/s).
- **UART RX buffer cap**: 16-byte model buffer silently dropped the 16th
  byte (a trailing `\n`), breaking browser RX smoke tests with exactly-16-
  byte sends. Cap raised to 64; verified 3-byte, 16-byte, and split sends
  in headless Chrome.
- **XPSR restore** in the interrupt pump (condition flags must survive the
  fake-ISR abort) — eliminated a ~1-in-2 `TCP fail` flake.
- **HTTP 000b bug**: consecutive RX injections landed in the same
  `rx_buf[0]`, so a queued second frame was parsed with a stale length.
  Fix: rotate the injection index across RX descriptor slots.
- **Per-round gateway restart** now opt-in (`GW_RESTART=1`); default mode
  runs consecutive rounds at full speed with no restart.
- **SPI NOR flash write path** (full): real-MISO timing (a dummy byte
  shifts out while the command/address bytes clock in — `dummy_pending`
  in `SpiFlash`), WriteEnable-gated PageProgram/SectorErase4k that commit
  on CS deassert (program ANDs bits, erase writes 0xFF, WEL auto-clears on
  completion like real W25Q). Program/erase buffering lives in `self.cmd`
  args so data bytes aren't mis-parsed as new opcodes. CS deassert edges
  reach the flash via GPIO write callbacks (`register_cs_callbacks`,
  wired in both `from_svd` and `new_wasm` — `from_svd` was missing the
  wiring, which silently killed commit for the SVD/`init_svd` path).
  `spi_flash_test` firmware: 9/9 PASS (JEDEC, WEL set/cleared, readback,
  second program, erase). Native integration test
  `peripherals::spi::tests::firmware_flow_via_gpio_cs` exercises the same
  flow through real MMIO + GPIO edges.
- **EXTI GPIO edge path** (new): `Exti::scan_lines` runs per tick, reads
  the GPIO line level of the SYSCFG-selected port, and pends IRQs on
  RTSR/FTSR edges; `exti_test` firmware: 3/3 PASS (fired once, PR cleared
  by handler, fired on 2nd edge). Test-driver note: an edge must be
  observed low by a tick before re-raising — injecting low+high in one
  JS iteration is invisible to the model.
- **FLASH program/erase** (new): `spi_flash_test`'s flash region now
  programs (PG) and sector-erases (SER) the backing buffer via the JS
  flash-command driver; `flash_test` 11/11 PASS.

## Known limitations

1. **Unicorn WASM wedge** (above) — the cap costs a few % throughput;
   the real fix is a newer/better Unicorn build.
2. **Guest-IRQ pump vs ETH firmware**: the pump must stay disabled for ETH
   firmware (an emulated ETH_IRQHandler re-scans `rx_desc` and stomps the
   driver's frame bookkeeping).
3. **Hardware paths not modeled**: DCMI/LTDC/SAI/I2S are synthetic-data
   only; DMA copies happen in JS; CAN has no bus peer.
4. **Timers are instruction-count driven**, not wall-clock driven — a
   `delay_ms(100)` is ~2.4M emulated instructions, so real-time blink
   rates don't hold (documented in AGENTS.md §11).

## Roadmap / future implementation

### Priority 1 — emulator robustness
- [ ] Replace/repair the Unicorn WASM build (native binding or newer
      wasm build with working timeouts + no 40k-instruction wedge), then
      raise `maxBatch` back and re-measure.
- [ ] Hardware-accurate NVIC: don't auto-enable IRQs in
      `set_intr_pending`; make the pump deliver pending interrupts only
      when ISER bits are set by firmware.
- [x] EXTI ↔ GPIO edge-trigger wiring (GPIO config drives EXTI pends).
- [x] FLASH program/erase emulation (write to the flash backing buffer),
      needed for DFU-style and bootloader firmwares.
- [ ] Move DMA memory copies into Rust (one less JS round-trip, faster
      per-transfer).

### Priority 2 — peripheral depth
- [ ] DCMI real pixel source, LTDC framebuffer scanout with a display
      sink.
- [ ] CAN bus peer / arbitration with a second node.
- [ ] I2S/SAI real audio (WAV-backed DMA).
- [ ] USB OTG (huge: host/device state machine) — biggest single gap.
- [ ] EtherCAT / timers in PWM servo mode for the printer heritage
      firmwares.

### Priority 3 — product / ecosystem
- [ ] Publish `stm32f4-emulator` to npm (README documents `npm pack` flow
      already; a `npm publish` + consumer verification remains).
- [ ] GitHub Pages: serve the demo over https (gateway mode currently
      needs http:// for plain `ws://`; document a WSS gateway or a
      local-proxy flow).
- [ ] VS Code extension / devcontainer with the full toolchain
      (wasm-pack, arduino-cli, go).
- [ ] Waveform/DMA trace view in the browser console.
- [ ] More firmware samples: USB CDC echo, FreeRTOS port, an
      interrupt-driven ETH driver (would exercise the pump + ETH together
      properly).
- [ ] Windows/macOS CI coverage (currently Linux runner).

## Verification checklist (regression)

```bash
npm test                                    # flow + blinky + rx-interrupt
scripts/verify_ethernet.sh 10000000         # 3 firmwares through gateway
node site/probe_firmwares.mjs               # every preset boots to a banner
node site/test_rx_interrupt.mjs             # interrupt-driven UART/CRC
node site/test_flash.mjs                    # FLASH program/erase (11/11)
node site/test_spi_flash.mjs                # SPI NOR write path (9/9)
node site/test_exti.mjs                     # EXTI GPIO edges (3/3)
(cd stm32-periph-wasm && cargo test --lib)  # native unit + integration tests
SOAK_STATS=1 node cli.mjs ../eth_http/eth_http.bin 200000000 \
  --gateway --config=../../eth_http/config.yaml   # long soak (≈15 min)
```
