---
sidebar_position: 3
title: Facade platform API
description: STM32F4 integration surface — event callbacks, DMA controller, display framebuffers, SWD/JTAG shim, power, probe memory.
---

# Facade platform API (`STM32F4`)

The `STM32F4` facade (`site/stm32f4.js`, exported from `stm32f4-emu` and
`stm32f4-emu/stm32f4`) is the integration surface for Wokwi/OpenHW/Velxio-style
platforms: rp2040js/avr8js-shaped factories, GPIO/USART/SPI/I2C handles, plus
polled peripheral-event callbacks, a full DMA controller view, live display
framebuffers, an honest SWD/JTAG shim, power estimates, and probe-style memory
access. Pure JS over the existing wasm exports — no Rust change was needed.

```js
import { STM32F4, decodeFirmware } from 'stm32f4-emu';
const mcu = await STM32F4.create({ firmware: decodeFirmware('blinky') });
mcu.gpio.pin('A', 5).on('change', (high) => console.log('PA5', high));
mcu.usart1.onData = (b) => process.stdout.write(String.fromCharCode(b));
mcu.onExtiEdge = (line) => console.log('EXTI', line);
mcu.execute(100_000);
```

## Chip option — is the API for all F4 chips?

Yes. `STM32F4.create({ chip })` with `chip` (or `board`) one of
`stm32f401` / `stm32f411` / `stm32f407` / `stm32f407ve` / `stm32f429`
(default `stm32f407`). One shared Cortex-M4F core — a variant is only
SVD + flash/RAM sizes + clock + IDCODE, resolved from the `CHIPS` table
(SVD truth in `site/vendor/*.svd`, sizes/clocks from `site/boards.js`):

- Per-chip SVD text + `flash_size`/`ram_size` + `chipHint` go to
  `createEmulator` (explicit `svdXml`/`flash_size`/`ram_size` opts still
  override the table). The hint drives the model's DBGMCU IDCODE
  (`init_svd_chip`): F401 0x423, F411 0x431, F407 0x413, F429 0x419 —
  readable live via `jtagIdcode()`.
- The facade surface gates on the chip's SVD presence lists: USART slots
  for absent silicon are `null` (F401/F411: `usart3/4/5` null; 1,2,6
  live), CAN polls iterate `chip.can` (empty on F401/F411 — no wasted
  hole reads), TIM polls iterate `chip.timers` (F401/F411 lack 6,7,12,
  13,14), `display.ltdc()` returns null without LTDC silicon,
  `pwrEstimate()` scales RUN current by the chip clock (84/100/168/180
  MHz), `gpio.pin()` rejects banks past the chip's GPIO count (F401/F411
  are A-F), and `ledStatus()` defaults to the instance chip.
- `mcu.chip` is the resolved record; `CHIPS`/`chipInfo()` are exported
  from both `stm32f4-emu` and `stm32f4-emu/stm32f4`.
- Firmware loaders size to the chip: `loadBin`/`loadHex`/`loadELF`
  reject images outside the chip's flash/RAM windows (never clip).
  `parseIntelHex`/`parseElf` take optional `(flashSize, ramSize)` for
  direct callers (F407-map defaults).
- Verified: `site/test_stm32f4_api.mjs` Test 7 boots all four chips'
  blinkies (LED=ON + per-chip LED + IDCODE + gating + power).

## Factories and firmware loading

- `STM32F4.create(opts)` — assets injected by `index.mjs` in Node
  (`bindings`/`svdXml`/`wasmInit`); pass `firmware` or load after.
- `STM32F4.fromBin(buf, opts?)` / `fromELF(buf, opts?)` / `fromHex(text, opts?)`.
- `loadBin(bytes, base?)` / `loadHex(text)` / `loadELF(bytes)`.
- `execute(cycles)` → `{instCount, stopped}`; `step(cycles)` →
  `{pc, instCount, stopped}`. Both drain UART into `usart1` and run the
  event poll below.

## GPIO / USART / SPI / I2C

- `gpio.pin('A'|0..'K'|10, pin)` → `GPIOPin`: `on('change', cb)` returns an
  unsubscribe function; `read()`/`readInput()`; `setInput(high)` /
  `setInputValue(high)`. No `setAnalog` — the F4 ADC model has channel
  injection (`setAdcChannel`), not an analog-wire layer.
- `usart1..8` (+ `usart` = USART1 alias, `usarts` map of live ports;
  7/8 live on F407/F429 only, `null` on F401/F411).
  `send`/`sendData` injects RX per-USART (SVD-verified bases). Slots for
  absent silicon are `null` (F401/F411: 3,4,5,7,8). **TX is one shared model
  buffer: only `usart1.onData`/`output` ever fire.** `usart2-8.send()`
  works; their `onData` never fires — model limit, not a bug.
- `spi: [{peripheral, cs?, dc?, onTransfer?, onByte?}]` and
  `i2c: [{peripheral, address, onStart?, onWrite?, onRead?, onStop?}]`
  create opts (taps snapshot at `init_svd` — post-create callback
  assignment cannot work; `spi1..3`/`i2c1..3` handles are call-shape compat
  only). `spi.pushMiso()` / `i2c.pushRx()` inject reply bytes.

## Peripheral-event callbacks (polled dispatch)

Set directly, default `null`, dispatched once per `execute()`/`step()` from
model-readable registers (the F4 model has no core event queue — F1's
`drainEvents` discriminants don't exist here). With no callback set the
poll is skipped (zero overhead when unused). A throwing callback never
breaks stepping.

| Callback | Source | Notes |
|---|---|---|
| `onExtiEdge(line)` | EXTI PR (w1c) | Read, cleared on dispatch, fired per set bit. Verified live on `exti_test` (line 0). |
| `onCanTx(can)` | CAN TSR TXOK/RQCP newly set | `can` is 1\|2. |
| `onCanRx(can,id,len,data)` | CAN RF0R/RF1R FMP level + mailbox regs | 11-bit id, `data` sliced to `len`. Verified live on `can_host_rx` (0x123/HELLO). Fires while FMP>0 with a changed mailbox (guest may hold the frame for many polls before RFOM). |
| `onTimUpdate(tim)` | TIM SR UIF newly set | Cleared on dispatch (w1c); `tim` 1-14. |
| `onTimCapture(tim,ch,val)` | TIM SR CCxIF + CCR latch | `ch` 0-3, 16-bit `val`. |
| `onDmaTc(controller,stream)` | DMA LISR/HISR TCIF newly set | `controller` 1\|2, `stream` 0-7. |
| `onWdogReset(which)` | IWDG/2 latch flags | 1=IWDG, 2=WWDG (F1 parity). |
| `onUsbIn(ep,data)` | `usb_take_in` drain | Non-destructive when empty. |
| `onUsbHsIn(ep,data)` | `usb_hs_take_in` drain | HS window (F407/F429 only). |
| `onItmByte(port,byte)` | `itm_take_port` drain | Firmware `printf` path. |
| `onFsmcAccess(bank,off,write,size,val)` | FSMC bank tap drain | Needs `ext_devices.fsmcDevices` at create (same rule as SPI). |
| `onAdcDone(adc,chan,value)` | ADC SR EOC/JEOC newly set | `chan` from SQR3 SQ1 (`0x100` bit = injected group). Non-consuming SR read; value peeked from DR. Guest-paced limit: single-shot firmware that consumes EOC between polls may never show EOC set — CONT/DMA/injected/fine steps always report. |
| `onDacWrite(chan,value)` | DAC DOR newly changed | DOR IS the pin value. Gated on DAC silicon (never fires on F401/F411). |
| `onCrcResult(value)` | CRC DR newly changed | Write-accumulate + CR-reset edges. |
| `onRtcAlarm(which)` | RTC ISR newly set | 0=A, 1=B, 2=wakeup (WUTF), 3=timestamp. Non-consuming ISR read. Tamper (TAMP1F) is NOT reported — enable TAMP1E + drive via `rtc_tamper_pin`. |
| `onI2cAlert(peripheral,asserted)` | I2C SR1 SMBALERT edge | Armed via `i2c_arm_smbus_alert` (peer pulling SMBA). |

Absent (no model source — the model is a USB *device*, no host channels
exist): `onHostTx`/`onHostRx` exist as never-firing F1-shape placeholders.
Host-driven traffic uses the methods below instead (`usbInject*`/
`usbTakeIn`, `ethInject` + create-time `onTx`).

## Live injection + device views (platform driving)

Anytime host calls over live model exports (no init constraint, unlike the
SPI/I2C taps). Presence-gated where the chip lacks silicon (CAN/DAC/TIM
throw on F401/F411 instead of sinking into benign-0 holes):

- ADC: `setAdcChannel(peripheral, channel, value)` /
  `clearAdcChannel(...)` (global override table; synthetic temp/Vref/Vbat/
  random default without it), `takeAdcDma()` (EOC-DMA staged samples).
- CAN: `canInject(id, dlc, data)` (11-bit, via `emu.canInject`),
  `canInjectFd(id, data, brs?)`.
- TIM: `timInjectCapture(timer, ch)` (host TIx edge),
  `timPwmPulseUs(timer, ch, clockHz)`, `timOcMode(timer, ch)`.
- DAC: `dacTrigger(ch, src, dmaStaged?)`, `dacUnderrun(ch)`.
- Audio: `takeSpeakerSamples()` (Float32 drain of the I2S capture FIFO).
- USB FS: `usbInjectSetup(bytes)`, `usbInjectOut(ep, bytes)`,
  `usbTakeIn(ep)`, `usbReset()`, `usbEnumerated()`; HS twins
  `usbHsInjectSetup/usbHsInjectOut/usbHsTakeIn/usbHsReset/usbHsEnumerated`
  (F407/F429-only window).
- ETH: TX captured by the create-time `onTx(frame, meta)` tap (no
  post-create hook — the MAC emits TX polls, not events);
  `ethInject(frame)` drives RX (netsim/gateway/pcap path).
- QSPI/SDIO/DCMI images bind at create (same rule as SPI taps — the model
  clones at construction): `ext_devices.qspi` (or `qspiImage` sugar),
  `ext_devices.sdio` (or `sdioBlocks` sugar), `ext_devices.camera` (or
  `cameraFrame` sugar).
- ITM: `takeItm(port)`, `itmPending(port)` (firmware `printf` path).
- FSMC: `takeFsmc(bank)`, `pushFsmc(bank, values)` (needs
  `ext_devices.fsmcDevices` at create — same rule as SPI).
- RTC/regfile: `regfileGet(peripheral, offset)` /
  `regfileSet(peripheral, offset, value)` (needs `ext_devices.regfile`
  or the `rtc` shorthand at create).
- Live views (same handles the browser panels read; null unless enabled
  at create): `mcu.oled`, `mcu.tft`, `mcu.rtc`, `mcu.buzzer`,
  `mcu.camera` (with `feed/stop/start/frames`).

## DMA (`dmaController` / `DMAStream` / `Display`)

- `mcu.dmaController(1|2)` → `DMAController` (DMA1 @0x40026000, DMA2
  @0x40026400): `lisr()`/`hisr()`, `tcif(s)`/`htif(s)`, `cr(s)`,
  `ndtr(s)`, `par(s)`, `m0ar(s)`, `m1ar(s)`, `fcr(s)`, `stream(s)`.
- `mcu.dma.stream(i)` — legacy flat helper (controller 1 only);
  `mcu.dma.controller(ctl)` — same as `dmaController(ctl)`.
- `DMAStream.tcif()`/`htif()`/`cr()`/`ndtr()` read the live controller;
  `pendingCount()`/`setCompleted()` drive the model's DMA queue.

## Display surface (`mcu.display`, "DRM")

Read-only live framebuffers for platform screen widgets (no canvas/DOM
dependency — works headless):

- `display.oled` → `{w:128, h:64, fb, frame}` (needs `ext_devices.oled`).
- `display.tft` → `{w:240, h:320, fb, frame}` RGB565 big-endian (needs
  `ext_devices.tft`).
- `display.ltdc()` → `{w, h, pf, fb, addr, pitch}` from the live LTDC
  layer0 regs + guest RAM (pf 0 ARGB8888 / 2 RGB565; null when off, or
  when the chip has no LTDC silicon like F401/F411).
- All return `null` when the device was not enabled at create.

## Debug, power, memory

- SWD/JTAG are an **honest shim** (no DP/MEM-AP on the F4 model):
  `swdHalted()` false, `swdHalt/Resume` no-ops, `swdStep()` steps once,
  `swdAddWatch` records, `swdTakeTrip()` empty, DP/AP reads 0,
  `swdRegRead(0-16)` reads the live register file, `jtagIdcode()` reads
  the live DBGMCU IDCODE. Call points exist for platform code; behavior
  is documented, not silicon.
- `addJsPeripheral(base, size, read, write)` records a JS region (no
  model-side MMIO hook table exists — documented shim, not silicon).
- `pwrMode()` → 0 RUN / 2 STOP / 3 STANDBY from PWR_CR; `pwrEstimate()`
  → rough µA (4 / 400 STANDBY/STOP classes; RUN scaled by chip clock —
  F407 30000, F401 15000, F411 17857, F429 32143 — uncalibrated).
- `memRead32()` / `memWriteBytes()` (probe-style via the `uc` shim —
  bypasses flash protection + MPU, throws out-of-range like the
  edge-case contract), `periphRead/periphWrite`, `getPc/getSp`,
  `faultInfo/takeFault`, `setSymbols/resolveSymbol` (pure-JS map symbols).

## Fault harness + scope probes (platform scripting)

1:1 mirrors of the wasm harness exports (same names, same args) — drive
fault arms with no guest cooperation:

- UART: `uartSetCts/uartFaultRx/uartIdle/uartLinBreak/uartBreakPending/
  uartTxLen/uartMuted` (per-base).
- SPI: `spiFaultCrc/spiFaultModf/spiSlaveSelect/spiSlaveClock/spiSlaveGate`.
- I2C: `i2cArmArbLoss/i2cArmSmbusAlert` + `i2cSlaveAddress/Write/Read/
  Stop/Status` (slave-mode guest harness).
- TIM: `timMoe/timEncoderStep/timBreakInput` (+ gated `timPwmPulseUs/
  timOcMode/timInjectCapture`).
- Misc: `sdioFaultDataCrc`, `rccInjectFailure`, `flashRdpLevel/
  flashSetRdp`, `rngSeedEntropy/rngEntropyAvail`, `rtcTamperPin/
  rtcTimestamp`, `setIntrPending/hasPendingInterrupt`.
- Scope probes (read-only): `adcDualLatched`, `ethPpsCount/ethPpsLevel/
  ethLinkUp/ethSetLink`, `ltdcScanline/ltdcFrameCount` (LTDC-gated),
  `audioRemaining/audioClear`, `dcmiSync`, `qspiMmapLive/qspiMmapRead`,
  `sdioBusWidth/sdioCardBlocks/sdioReadBlock`.

## Tests

`site/test_stm32f4_api.mjs` (factories, GPIO/USART, result shapes, symbols,
callbacks-null, debug/power/DMA/display/memWriteBytes, 4-chip boot) and
`site/test_stm32f4_periph.mjs` (SPI/I2C taps), both in `npm test`.

## Chip support matrix (SVD census, `site/vendor/*.svd`)

Bases are identical wherever the peripheral exists; presence differs:

| Peripheral | F401 | F411 | F407 | F429 | Facade behavior |
|---|---|---|---|---|---|
| USART1/2/6 | ✅ | ✅ | ✅ | ✅ | live slots |
| USART3/UART4/UART5 | ❌ | ❌ | ✅ | ✅ | `null` slots on F401/F411 |
| UART7/UART8 | ❌ | ❌ | ✅ | ✅ | `usart7/8` slots, `null` on F401/F411 |
| SPI1/2/3 | ✅ | ✅ | ✅ | ✅ | always live |
| SPI4 | ✅ | ✅ | ✅ | ✅ | live slots (chip-gated) |
| SPI5 | ❌ | ✅ | ✅ | ✅ | `null` on F401 |
| SPI6 | ❌ | ❌ | ✅ | ✅ | `null` on F401/F411 |
| I2C1/2/3 | ✅ | ✅ | ✅ | ✅ | always live |
| UART7/8, I2S2/3ext | ❌/✅ | ❌/✅ | ✅ | ✅ | model-level only (no facade slot; use `periphRead`) |
| SAI1 | ❌ | ❌ | ✅(SAI1) | ✅(SAI) | model-level only |
| TIM1-5,8-11 | ✅ | ✅ | ✅ | ✅ | polled |
| TIM6/7/12/13/14 | ❌ | ❌ | ✅ | ✅ | skipped in TIM poll on F401/F411 |
| DMA1/DMA2, EXTI, PWR, RTC, IWDG, WWDG, ADC1, FLASH, SYSCFG | ✅ | ✅ | ✅ | ✅ | always live |
| CAN1/CAN2 | ❌ | ❌ | ✅ | ✅ | polls iterate `chip.can` (empty = skip) |
| DAC, LTDC | ❌ | ❌ | ✅ | ✅ | `ltdc()` null without silicon |
| GPIO | A-F | A-F | A-K | A-K | `gpio.pin()` rejects past-bank pins |
| USB | FS | FS | FS+HS | FS+HS | `usb_*` FS all chips; `usbHs*` F407/F429 window |
| SDIO, CRC | ✅ | ✅ | ✅ | ✅ | harness methods live on all chips |
| DCMI, CRYP, HASH, RNG, FSMC | ❌ | ❌ | ✅ | ✅ | methods live (model boots the block regardless) |
| DMA2D | ❌ | ❌ | ❌ | ✅ | `chip.dma2d` presence flag |
| QSPI | — | — | — | — | model extension (no SVD): `qspiImage` binds on all chips |
