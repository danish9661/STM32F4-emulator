---
sidebar_position: 8
title: Progress & Future
description: What works today, known limitations, and the implementation roadmap.
---

# Progress and future work

Status as of 2026-08-09. The authoritative living log is AGENTS.md; this
document is the readable summary.

## What works today

### Emulator core
- Full Cortex-M4 Thumb-2 execution via Unicorn 2.1.4 (WASM) — the same
  firmware binaries run here and on real silicon, for **logic**; timing
  is instruction-count driven, not wall-clock (see Known limitations #4).
- 33 peripheral modules, 28 detailed (see [peripherals.md](peripherals.md)).
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
  31 firmware presets, custom `.bin`/`.hex`/`.elf`/`.map` upload, gateway
  connection to real gVisor networking, live GPIO grid, peripheral register
  readout, packet viewer. Deployed to GitHub Pages.
- **DOOM runs in the browser** (`site/doom.html`): the doomgeneric F407
  port boots, plays the shareware WAD at ~25 FPS (realtime-locked guest
  clock, I2S mixer → AudioWorklet), and **saves/loads games to
  `localStorage`** (F2/F6 save, F3/F9 load — firmware stages savegames in
  EXTRAM at 0xC0080000, 2 slots × 256 KB, mirrored by doom.js).
  Below DOOM's native 35 fps: ~918k guest instructions/frame means 35 fps
  needs ~32 MIPS against a ~20-24 MIPS core ceiling (measured 2026-08-14,
  see AGENTS.md §16 — `emu.step()` size and `-O2` ruled out; dropping the
  per-block counting hook did help, 22 → 25 fps). Audio is mixed per
  rendered frame, so below 35 fps the worklet rate-matches and plays
  slightly slow/pitched-down rather than breaking up.
  Verified: `node site/test_doom.mjs` (boot → menu → E1M1 → `SAVE ok
  slot=0`) and a headless-Chrome CDP smoke (save → reload-less F9
  quick-load → `LOAD ok`, audio continuous).
- `stm32f4-emulator` npm package (packed, not published) with a clean
  `createSTM32F407({firmware})` API.
- CI runs the Node test suite on every push; Pages deploys the demo.

### Recently fixed (2026-08-09)
- **DOOM save/load (2026-08-14)**: firmware save shim routes
  `doomsavN.dsg` file ops to a 2×256 KB EXTRAM staging area through a
  private fd (0x7f00), commits via the newlib `rename` = `_link` +
  `_unlink` chain, and busy-waits for the driver on load — with the
  driver (`doom.js`) mirroring the blob to `localStorage`. Also fixed
  the browser's F-key codes (engine `KEY_F1..F12 = 0xBB..0xC6`; the
  old 0x80..0x8B mapping made F2/F3/F6/F9 dead keys) and passed raw
  ASCII letters through for the save-name entry and 'y' confirms.
  Details + gotchas in AGENTS.md §16.- **Unicorn 40k-instruction wedge**: one `emu_start` running ~40k+
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
- **Hardware-accurate NVIC** (new): `set_intr_pending` no longer
  auto-enables IRQs. Pending is set regardless of ISER; delivery (pump)
  happens only when the firmware sets the enable bit — a disabled pending
  IRQ stays pending until taken or cleared via ICPR, exactly like real
  hardware. `has_pending`/`get_pending_vector` now report only deliverable
  pending, and ICSR VECTPENDING returns the exception vector number (was
  returning the 0-based IRQ number). All interrupt-driven firmwares
  (rx_interrupt, rx_crypto, exti, eth_http/dhcp/test) set ISER explicitly
  and pass unchanged; 20M-inst soak: 121 TCP connected, 0 TCP fail.

## Known limitations

1. ~~**Unicorn WASM wedge**~~ — **resolved 2026-08-10**: does not
   reproduce on Node 22.22 (V8); fresh characterization with count-based
   `emu_start` at n=1000..1,000,000, and 150M-instruction cli.mjs soaks at
   `MAX_BATCH=500000`, ran wedge-free (AGENTS.md §7). Ruling: the original
   wedge was build/environment-specific to an older Node/V8 WASM engine,
   not the vendored Unicorn build itself. `cli.mjs`'s `maxBatch` (default
   200000, `MAX_BATCH` env override) remains, but as a throughput/
   responsiveness knob — the driver still needs periodic `emu_start`
   returns to service DMA/ETH polling and interrupts regardless of any
   wedge — not a workaround for a bug. One unrelated, still-reproducible
   instance killer: passing the `timeout` argument to `emu_start` aborts
   the instance (`qemu_thread_create: Not supported`); nothing in this
   repo passes it.
2. **Guest-IRQ pump vs ETH firmware**: the pump must stay disabled for ETH
   firmware (an emulated ETH_IRQHandler re-scans `rx_desc` and stomps the
   driver's frame bookkeeping).
3. **Hardware paths not modeled**: DCMI has no pixel source (CAN now has
   a real two-node bus with arbitration; I2S/SAI have a WAV-backed DMA
   capture path; LTDC has real scanout + a browser sink). USB OTG is
   **explicitly out of scope**, not a to-do — see roadmap note below.
4. **Timers are instruction-count driven**, not wall-clock driven — a
   `delay_ms(100)` is ~2.4M emulated instructions, so real-time blink
   rates don't hold (documented in AGENTS.md §11).

## Roadmap / future implementation

### Priority 1 — emulator robustness
- [x] Unicorn WASM wedge: not reproducible on Node 22.22 (2026-08-10 —
      count-based `emu_start` returns cleanly at any budget, 500k batches
      × thousands of rounds; the vendored build IS the current unicorn.js
      v2.1.4 arm release). `cli.mjs` default `MAX_BATCH` raised 20k →
      200k (~2.96–3.2 MIPS vs ~2.1). Remaining known landmine: the
      `timeout` argument to `emu_start` aborts the instance
      (`qemu_thread_create: Not supported`) — nothing in the repo passes
      it; a native Node addon or V8-upgrade would fully retire this.
- [ ] Hardware-accurate NVIC: don't auto-enable IRQs in
      `set_intr_pending`; make the pump deliver pending interrupts only
      when ISER bits are set by firmware.
- [x] EXTI ↔ GPIO edge-trigger wiring (GPIO config drives EXTI pends).
- [x] FLASH program/erase emulation (write to the flash backing buffer),
      needed for DFU-style and bootloader firmwares.
- [x] DMA peripheral-side copies chunked in Rust (`dma_periph_read`/
      `dma_periph_write`): one WASM call per transfer instead of size/4
      per-chunk calls from JS; also fixed M2P which previously wrote
      peripheral bytes back into guest RAM. RAM-to-RAM copies stay in
      JS (Unicorn owns guest memory).

### Priority 2 — peripheral depth
- [x] **DOOM audio "only crackling" — FIXED 2026-08-14. The root cause was
      an inverted resample ratio, not the fps shortfall.**
      `site/audio-worklet.js` advanced its 11025 Hz read cursor by
      `sampleRate/11025` (≈4.35) per output sample instead of
      `11025/sampleRate` (≈0.23) — consuming input ~19x too fast, so every
      sound played far above audible pitch and starved instantly. It passed
      every test because the tests only counted samples the guest PRODUCED;
      nothing verified playback consumed them at the right rate.
      Also replaced the underrun policy (which emitted silence AND flushed
      the queue, discarding good audio) with a proportional rate controller
      that matches playback to production and never flushes.
      Measured on the page over 8 s of play: **starved output samples
      364476 → 0**. Below 35 fps audio now plays slow/pitched-down
      (~0.72x at 25 fps) but continuous — matching the slow-motion game.
      Note for future tuning: the rate floor must stay below `fps/35`, and
      the `?v=` on `addModule('audio-worklet.js?v=N')` MUST be bumped on
      every edit or you are testing a cached worklet.
- [x] LTDC scanout + display sink: the model advances a scanline/scanframe
      (2 px per tick from the real SSCR/BPCR/AWCR geometry), fires LIF at
      LIPCR and the frame-end F flag, and pends LTDC IRQ 88; exports
      `ltdc_get_scanline` / `ltdc_get_frame_count`. `ltdc_test` firmware
      paints an ARGB8888 gradient layer, and the browser console renders
      layer0's framebuffer into a canvas panel live (`?fw=ltdc_test`;
      ARGB8888 + RGB565 handled).
- [x] I2S/SAI real audio (WAV-backed DMA): `audio_load_wav` parses a
      RIFF/WAVE PCM16 file into the model source; I2S/SAI DR reads consume
      it (falling back to the synthetic generator), DR writes push into a
      capture FIFO (`audio_take_capture`). The shared SPI block routes
      DR to audio when I2SMOD is set (real silicon shares the register
      block). `audio_test` firmware runs a full DMA1 PERIPH->MEM transfer
      from I2S1_DR and checks the sample checksum — this drove DMA
      fixes below.
- [x] DMA PINC + PSIZE-aware peripheral reads: transfers now carry PINC and
      the peripheral width, so a fixed-address 16-bit DR FIFO yields
      contiguous sample streams (`pinc=0` re-reads the same register in
      `psize` chunks) instead of zero-padded 4-byte groups. Completion
      flags (TCIF/HTIF) are no longer set at EN-write: they latch from the
      JS driver's `dma_set_completed` (via `dma_check_completion` in the
      LISR/HISR read path) and stay set until the guest clears them through
      IFCR — real-w1c semantics.
- [ ] DCMI real pixel source.
- [x] USB OTG — **deferred, not planned**: none of this repo's firmware
      targets or intended use cases (networking demos, DOOM, breadboard-
      style peripheral simulation) exercise USB, and a browser sandbox has
      no real USB host to enumerate against without WebUSB passthrough to
      physical hardware — which isn't emulation. Revisit only if a
      concrete firmware need appears (a lighter USB CDC echo sample is a
      much smaller ask, see Priority 3).
- [ ] EtherCAT / timers in PWM servo mode for the printer heritage
      firmwares.
- [x] CAN bus peer / arbitration between CAN1 and CAN2 on a shared bus:
      TX requests stage frames; each tick arbitrates (lowest arbitration
      ID wins, ties by node then mailbox), the winner's mailbox completes
      (TSR TXOK|TME|RQCP) and the frame broadcasts to every node's RX that
      passes its filter banks (winning transmitter receives its own frame
      like real CAN); losers stay staged for the next free round. Real
      filter semantics: 28 global banks (CAN2 = 14..27), mask/list modes,
      32/16-bit scale, FFA1R FIFO assignment; 3 mailboxes per FIFO at the
      real addresses, FMP/FULL/FOVR and RFOM release. BTR LBKM loopback
      delivers only to the sender. `can_test` firmware: loopback echo
      (id/payload verified) + two-node arbitration (both nodes end with
      both frames); 4 Rust unit tests cover arbitration, loopback,
      filter gating, FIFO overflow/release. Also fixed the minimal
      (no-SVD) register list — it was missing CAN1/CAN2 entirely, so CAN
      firmware silently did nothing in browser/test builds.

### Priority 3 — product / ecosystem
- [x] **Component-attachment API** (2026-08-14, rp2040js-style): public
      `emu.pin()`/`watchPin()`/`i2cRegfile()`/`setAdcChannel()` plus
      `LED`/`Button`/`Pwm`/`Potentiometer`/`I2cRegisterDevice` in
      `site/components.js`, and `ext_devices.spiDevices`/`i2cDevices` for
      embedder-defined bus protocols on the existing SPI/I2C taps. Each
      class is verified against real firmware in its own process
      (`site/test_component_{led,button,pwm,i2cregfile,adc}.mjs`).
      ADC injection needed the one Rust change (a global override table in
      `system.rs` + `adc_set_channel_value`/`adc_clear_channel_value`
      exports) since channels previously only produced LCG noise.
      See [components.md](components.md).
- [x] **MCP server** (2026-08-14): `mcp/server.mjs` exposes the emulator
      as 14 MCP tools (load/step/UART/pins/ADC/registers/memory/components)
      over stdio via the official `@modelcontextprotocol/sdk` — the
      project's first runtime npm dependency. Protocol round-trip smoke
      test: `npm run test:mcp`. See [mcp.md](mcp.md).
- [x] **Windows/macOS CI** (2026-08-14): `ci.yml` runs the suite on
      `ubuntu-latest`/`windows-latest`/`macos-latest`. No source changes
      were needed — the test path was already portable (plain `node`
      invocations, `new URL(..., import.meta.url)` paths, no
      `child_process`/native addons, prebuilt WASM needs no Rust
      toolchain).
- [ ] Publish `stm32f4-emulator` to npm (README documents `npm pack` flow
      already; a `npm publish` + consumer verification remains).
- [ ] GitHub Pages: serve the demo over https (gateway mode currently
      needs http:// for plain `ws://`; document a WSS gateway or a
      local-proxy flow).
- [ ] VS Code extension / devcontainer with the full toolchain
      (wasm-pack, arduino-cli, go). (An MCP server now covers part of the
      "drive the emulator from your editor" use case — see above.)
- [ ] Waveform/DMA trace view in the browser console.
- [x] Interrupt-driven ETH driver (`eth_irq_test`): NVIC ETH IRQ 61 +
      DMAIER, the pump runs `ETH_IRQHandler` which reads DMASR TS/RS,
      scans/re-arms RX descriptors; the driver only signals the model
      (`irq_eth` mode — no SRAM flag writes). Also fixed the RX descriptor
      format: FS/LS marker bits at 28/27 corrupted the frame-length window
      [29:16] (`len<<16` only, like real F407).
- [x] **FreeRTOS port (`freertos_test`)** — verifies the interrupt pump's
      context-switch path end-to-end: TIM3 ISR → `xSemaphoreGiveFromISR(xTimSem)`
      → `portYIELD_FROM_ISR` (PendSV) → scheduler context-switches to the
      higher-priority `vHighTask`, which pends on the semaphore. `vHighTask`
      arms TIM3 itself (self-contained). Wired as `probe_freertos.mjs`
      regression test (wired into `npm test`); the probe is intentionally
      quiet (final summary + `PROBE PASS`/`PROBE FAIL`). This was the
      regression test that caught (and now guards) the mid-`str`
      exception-return PC bug fixed in `site/emulator.js` `processInterrupts`
      (see AGENTS.md §9).
  - **Deeper FreeRTOS coverage — DEFERRED (not needed).** The probe already
    exercises all three yield types (task / ISR / SysTick) plus preemption
    and a binary-semaphore give-from-ISR, which fully guards the
    emulator-specific defect. Inter-task queues, mutex/priority-inheritance,
    task deletion, and a second concurrent live ISR (e.g. UART RX) would
    mostly test *guest* FreeRTOS library code, not new emulator behavior,
    and add maintenance cost for marginal protection. Revisit only if
    `processInterrupts` is reworked or a real FreeRTOS app using those
    primitives is targeted.
 - [ ] USB CDC echo.

### Delivered 2026-08-22 (CLI / DX pass)
 - [x] **`stm32f4-emu` headless CLI** (`cli.mjs`, registered as the `stm32f4-emu`
       `bin`): loads `.bin`/`.elf`/`.hex`, boots it, streams guest UART to
       stdout; flags `--inst`, `--format`, `--verbose`, `--help`, `--version`.
 - [x] **`--verbose` register trace** in `site/emulator.js` (`opts.verbose`),
       traces peripheral MMIO R/W to stderr, capped at 5000 accesses.
 - [x] **Actionable firmware-load errors**: empty/too-small image and zero
       reset-vector throws with guidance; `loaders.js` ELF/HEX parse failures
       name the expected format and likely cause.
 - [x] **`stm32f4-mcp --help` / `--version`** for the MCP server bin.
 - [x] **`CHANGELOG.md`** tracking releases/features.
 - [x] **TypeScript declarations** (`index.d.ts`, `site/emulator.d.ts`) for the
       public Node API (createEmulator / createSTM32F407 / decodeFirmware /
       components) so TypeScript consumers get types.

### Deferred / low-priority (tracked, not scheduled)
 - [ ] **More demo firmwares** — a CAN-bus demo (exercising the two-node
       arbitration model already in `can.rs`) and a deep-sleep / low-power
       (STOP/WFI + RTC wakeup) demo. Useful for showcasing, but the peripheral
       models they need are already verified by existing `can_test` /
       `rtc_test`; the marginal emulator value is a new firmware + test
       harness each. Revisit when a showcase gap is identified.
 - [ ] **Wider edge-case test coverage** ("236/236" style) — the current suite
       already guards every emulator-specific defect (FreeRTOS context switch,
       ETH RX/TX, DMA, I2C/SPI taps, LTDC, audio, RTC). Extra cases would mostly
       re-test *guest* library code. Low value relative to maintenance cost;
       add only when a new bug class appears.
 - [ ] **Website / docs polish** — landing-page copy, diagrams, more in-page
       help. Cosmetic; do alongside the next public-facing push.
 - [ ] **Performance work** — the MIPS ceiling is the Unicorn 2.1.4 WASM core
       (≈20–23 MIPS headless; DOOM runs ~22–24 fps). Already optimized
       (per-block hook, noCountHook path, minimalPolls). No further easy
       headroom without a different CPU core; revisit only if a faster Unicorn
       build or a native (non-WASM) binding becomes available.


## Verification checklist (regression)

```bash
npm test                                    # flow + blinky + rx-interrupt + 5 component tests
npm run test:mcp                            # MCP protocol round-trip (needs npm install)
scripts/verify_ethernet.sh 10000000         # 3 firmwares through gateway
node site/probe_firmwares.mjs               # every preset boots to a banner
node site/test_rx_interrupt.mjs             # interrupt-driven UART/CRC
node site/test_flash.mjs                    # FLASH program/erase (11/11)
node site/test_spi_flash.mjs                # SPI NOR write path (9/9)
node site/test_exti.mjs                     # EXTI GPIO edges (3/3)
node site/test_can.mjs                      # CAN loopback + 2-node arbitration
node site/test_audio.mjs                    # I2S DMA WAV replay + TX capture
node site/test_ltdc.mjs                     # LTDC scanout + framebuffer pixels
(cd stm32-periph-wasm && cargo test --lib)  # native unit + integration tests
SOAK_STATS=1 node cli.mjs ../eth_http/eth_http.bin 200000000 \
  --gateway --config=../../eth_http/config.yaml   # long soak (≈15 min)
```
