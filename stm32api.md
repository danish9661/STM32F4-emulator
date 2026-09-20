# stm32api.md — STM32F1 (`stm32f1-emu@3.1.0`) API + OpenHW gap spec

Probed from `/tmp/emu/stm32` (`pkg/*.js`, `*.d.ts`, `package.json`, `site/board_pins.json`).
Status claims (`~70M IPS`, `736 checks`, `39/39 firmware`) are README claims, not re-verified here.
All API names below were dumped live.

## 1. Package layout

- `package.json`: name `stm32f1-emu`, v3.1.0, ESM (`"type": "module"`), Node ≥18.
- Exports: `.` → `pkg/stm32f1.js`, `./emulator` → `pkg/emulator.js`, `./gdb` → `pkg/gdbstub.mjs`, `./wasm` → raw wasm-pack glue, `./cli` → `pkg/cli.mjs`; bins `stm32f1-emu` / `bluepill-emu`.
- Files: `pkg/emulator.js` (low-level bridge), `pkg/stm32f1.js` (ergonomic wrapper), `pkg/stm32_bluepill_wasm.js` + `_bg.wasm` (Rust core), `pkg/gdbstub.mjs`, `pkg/cli.mjs`, `pkg/ws-server.mjs`, `site/board_pins.json`, `svd/`.

## 2. High-level API (`pkg/stm32f1.js` → `pkg/stm32f1.d.ts`)

`import { STM32F1 } from 'stm32f1-emu'`

Factories: `STM32F1.create(opts)`, `fromELF(buf, opts?)`, `fromBin(buf, opts?)`, `fromHex(text, opts?)`; instance `loadELF/loadBin/loadHex`, `_reload`, `reset()` (recreates emulator).

Tick: `execute(cycles) → {instCount, stopped}`, `step(cycles) → {pc, instCount, stopped}`, `stop()`, `close()`. Event drain (`_drain_events`) runs after every `execute/step`, dispatching flat i32 events (discriminants 1–22: SPI/I2C/UART/EXTI/ADC/TIM/DAC/CRC/RTC/WDOG/CAN×2/FSMC/USB/I2CAlert/HostTx/HostRx/ITM).

- `gpio: GPIO` → `gpio.pin('A'|'B'|'C'|0|1|2, 0..15) → GPIOPin { on('change',cb)→unsub, read()->0|1 (output), readInput(), setInput(high), setAnalog(0..4095) }`
- `usart1/2/3` (+`usart{1,2,3}`): `{ onData(byte)->void|null, send(string|Uint8Array|number[]), output (accumulated TX string), _tx(byte) internal }`. Top-level `uartRx(byte)` / `uartOutput` are USART1-only shortcuts.
- `spi1/2/3`: `{ onTransfer(ch,tx[],rx[])->void|null, injectMiso(bytes) }`
- `i2c1/2/3`: `{ onStart(addr), onWrite(byte), onRead(), onStop(), injectRx(bytes) }`
- Top-level callbacks (set directly): `onExtiEdge(line)`, `onAdcDone(adc,chan)`, `onTimUpdate(tim)`, `onDacWrite(chan,val)`, `onCrcResult(v)`, `onRtcAlarm(a)`, `onWdogReset(1=IWDG|2=WWDG)`, `onCanTx/Rx(can,id,len,data[8])`, `onTimCapture(tim,ch,val)`, `onFsmcAccess(bank,off,write,size,val)`, `onUsbIn(ep,data[])`, `onI2cAlert(ch,asserted)`, `onHostTx(ch,ep,setup,data[])`, `onHostRx(ch,ep,len)`, `onItmByte(port,byte)`.
- Misc: `onPeriphWrite(fn)→unsub`, `setSymbols(mapText)`, `resolveSymbol(pc)`, `fsmcWriteByte/fsmcReadByte(name,off,val)`.

Re-exports: `parseElf, parseIntelHex, parseSymbolMap`.

## 3. Low-level API (`pkg/emulator.js` → `pkg/emulator.d.ts`)

`import { createEmulator, CHIPS, chipInfo } from 'stm32f1-emu/emulator'`
`createEmulator(opts?: CreateEmulatorOptions) → Promise<BluepillEmulator>`.

Options: `firmware (Uint8Array|ArrayBuffer|HEX string)`, `flash_size` (def 64K), `ram_size` (def 20K), `vector_table` (def 0x08000000), `svd`, `chip` (name|{…}|svd), `js_peripherals[] {base,size,read,write}`, `uart_addr` (def USART1 0x40013800), `ext_devices {spi_flash,i2c_eeprom,i2c_oled,lcd,touchscreen,software_spi,fsmc_bank}`, `verbose`, `batch_size` (overrides adaptive 20K/50K).

`BluepillEmulator` methods (verbatim, sorted):

```
adcSetInternal, adcSetRcTau, addJsPeripheral, bootloaderEnable, bootloaderGoAddr,
canInjectMessage, close, dmaPending, drainEvents, fsmcReadByte, fsmcWriteByte,
getPc, getRegisters (-> R0..R12,SP,LR,PC,xPSR), getSp, getSymbolCount, getUartOutput,
gpioReadInput, gpioReadOutput, gpioSetAnalog, gpioSetInput, i2cInjectAlert,
i2cInjectRead, i2cInjectRx, i2cInjectStart, i2cInjectStop, i2cInjectWrite,
i2cOledFb, jtagAp, jtagDp, jtagIdcode, jtagIr, jtagReset, lcdFb, memRead32,
memWriteBytes (probe: bypasses flash protection+MPU), onPeriphWrite->unsub,
onPinChange(fn(port,pin,level))->unsub, otgBusReset, otgDetach, otgHostAttach,
otgHostFeedIn, otgInjectOut, otgInjectSetup, periphRead(addr,width?), periphWrite,
pwmDuty(addr,ch?), pwrEstimate (µA), pwrMode (0=RUN,1=SLEEP,2=STOP,3=STANDBY),
read32, resolveSymbol, run(maxInstructions?)->{totalSteps,instCount,stopped},
rxPending, setPc, setReg(i,v), setSimAdc, setSymbols, setTouch, spiInjectMiso,
step(maxBatch?)->{pc,instCount,stopped}, stop, swdAddWatch, swdApRead, swdApWrite,
swdDpRead, swdDpWrite, swdHalt, swdHalted, swdRegRead, swdRegWrite, swdRemoveWatch,
swdResume, swdStep, swdTakeTrip, takeFault()->[pc,op]|null, takePinEvents,
uartInjectBreak, uartRx, uartRxAddr, uartRxBytes, usbBusReset, usbDetach,
usbInjectOut, usbInjectSetup, write32
```

`CHIPS` (8): `stm32f103c8, stm32f103cb, maple_mini, nucleo_f103rb, stm32f103rc, gd32f103c8, gd32f103cb, gd32f103rb` (flash/RAM/DBGMCU IDCODE/label each; `chipInfo(name?)`).

Raw WASM (`pkg/stm32_bluepill_wasm.d.ts`): 119 `export function` incl. `init/init_svd`, `process_batch/step_batch/step/tick`, `rustcpu_*` (init/load/run/regs/mem/fault/dma/tap), `dma_*`, `gpio_*`, `uart_rx_byte/uart_rx_pending/uart_inject_break/get_uart_output`, `spi_inject_miso`, `i2c_inject_*`, `can_inject_message`, `swd_*`, `usb_*/otg_*`, `fsmc_*`, `lcd_fb/i2c_oled_fb`, `pwr_*/rcc_*`, `raise_fault`, `register_js_peripheral`. Do not call directly — `emulator.js` owns init/batch policy.

## 4. Servers / debug

- `serveGdb({firmware?,chip?,flash_size?,ram_size?,vector_table?,ext_devices?,port?=1234,chunk?=20000}) → {port,emu,close}` — RSP `target remote :port`, BKPT Z0, watchpoints Z2/Z3/Z4, step/continue, target.xml.
- `pkg/cli.mjs`: parses ELF (`7F 45 4C 46`→regions+symbols), HEX (`:`→base+data), else raw bin @0x08000000; YAML config; uses `process_batch/dma/rustcpu_* /init`.
- `pkg/ws-server.mjs <firmware.elf> [--port]`: static `site/` + WS in (`uart_rx {addr,byte}`, `gpio_set {port,pin,high}`, `can_inject {...}`), `hello` welcome frame, 20K-batch loop. Closest existing template for an OpenHW bridge.
- `site/board_pins.json`: Arduino aliases per board (maple_mini, nucleo_f103rb, … from STM32duino 2.12.0).

## 5. OpenHW gap (what's missing today)

- No `openhw-stm32-bluepill` key in frontend `LOGIC_REGISTRY`/`COMPONENT_PINS`; emulator package exports it (`src/components/index.ts:11`), stub logic exists (`openhw-stm32-bluepill/logic.ts`: PC13 active-low LED, irq counters) — runner never instantiates it.
- `createRunnerForBoard` (`frontend/src/worker/execute.ts:3-18`) routes only pico→RP2040Runner else AVRRunner. No STM32 branch. `runners/` has only avr/rp2040. The `BackendProxyRunner`/QEMU/Renode plan in `openhw-studio-docs/{esp and stm32.md,architecture/stm32.md}` is stale — file doesn't exist.
- `board-profiles.ts` (UNO+PICO only), `backend/src/compiler/boardRegistry.js` (uno+pico only) need STM32 entries (F103C8: ELF/HEX artifacts, GenF1 FQBN).

## 6. Runner mapping checklist (BoardRunner contract `component-registry.ts:472-501`)

1. Load: `STM32F1.fromELF/fromBin/fromHex` per compile artifact; `chip` from board variant; `reset()`→reload.
2. Tick: `execute(budget)` per frame (batch policy internal); `stop()` on halt; `getPc/getRegisters/takeFault` for debug snapshot; `readMem` via `memRead32`/`periphRead` windows.
3. GPIO: `gpio.pin(port,pin).on('change')` → propagateBoardPin; `setInput/setAnalog` ← UI buttons/sliders; EXTI via `onExtiEdge`.
4. UART: `usartN.onData` → serial TX frames; `send()` ← `serialRx/serialRxByte`; baud via `setSerialBaudRate` passthrough (emulator has no baud API — record only).
5. I2C/SPI/devices: `onStart/onWrite/onRead/onStop` + `injectRx` → ComponentSignalAPI `onI2CWrite/onI2CRead`; `onTransfer` + `injectMiso` → `onSPIByte/onSPIBuffer`; `ext_devices` at create for flash/EEPROM/OLED/LCD; `lcdFb/i2cOledFb` → display descriptors (keep VRAM SAB path).
6. Debug: SWD (`swdHalt/Resume/Step/AddWatch/TakeTrip`, `swdRegRead/Write`) → debug service HALT/STEP/WATCH/READ_MEM; `setSymbols/resolveSymbol` for PC symbols; `serveGdb` optional out-of-band.
7. SAB publish: `uiState {i:builtInLed}` exists on stub; extend per runner `forceEmitState` (pins bitfield + slots + telemetry ring), never per-instruction WASM calls.
