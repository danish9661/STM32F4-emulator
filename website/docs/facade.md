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
- `usart1..6` (+ `usart` = USART1 alias, `usarts` map). `send`/`sendData`
  injects RX per-USART (SVD-verified bases). **TX is one shared model
  buffer: only `usart1.onData`/`output` ever fire.** `usart2-6.send()`
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
| `onItmByte(port,byte)` | `itm_take_port` drain | Firmware `printf` path. |
| `onFsmcAccess(bank,off,write,size,val)` | FSMC bank tap drain | Needs `ext_devices.fsmcDevices` at create (same rule as SPI). |

Deliberately absent (no model source — a fake event would be worse than
none): `onAdcDone`/`onDacWrite`/`onCrcResult`/`onRtcAlarm`/`onHostTx`/
`onHostRx`/`onI2cAlert`. Poll `read32()` or use the bus taps instead.

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
  layer0 regs + guest RAM (pf 0 ARGB8888 / 2 RGB565; null when off).
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
  → rough µA (4 / 400 / 30000 classes — uncalibrated).
- `memRead32()` / `memWriteBytes()` (probe-style via the `uc` shim —
  bypasses flash protection + MPU, throws out-of-range like the
  edge-case contract), `periphRead/periphWrite`, `getPc/getSp`,
  `faultInfo/takeFault`, `setSymbols/resolveSymbol` (pure-JS map symbols).

## Tests

`site/test_stm32f4_api.mjs` (factories, GPIO/USART, result shapes, symbols,
callbacks-null, debug/power/DMA/display/memWriteBytes) and
`site/test_stm32f4_periph.mjs` (SPI/I2C taps), both in `npm test`.
