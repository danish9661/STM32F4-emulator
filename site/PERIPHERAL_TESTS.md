# Peripheral tests — what they do and what to expect

Every peripheral test boots a small bare-metal firmware into the emulator
(the Rust CPU core + the WASM peripheral model) and checks two things:

1. **UART markers** — lines the firmware prints as it progresses.
2. **Model-side state** — registers, framebuffers, capture FIFOs, model
   counters read from JS.

A test prints `PASS` (exit 0) or `FAIL` (exit 1) at the end.

## Running them

```bash
# one test
node site/test_oled.mjs

# all 16
for t in site/test_*.mjs; do echo "-- $t"; node $t >/dev/null && echo PASS || echo FAIL; done

# Rust unit tests of the peripheral model itself
cd stm32-periph-wasm && cargo test --release
```

Browser smoke (all five device firmwares on the console page, headless
Chrome on port 9223, site on 8123; restarts Chrome between boots to dodge
mem_map pressure, see AGENTS.md §11):
`node /tmp/opencode/devices_smoke.mjs`

---

## Network / Ethernet

### test_flow.mjs — full DHCP → TCP → HTTP client round (eth_http)
The big one. Boots `eth_http` (a real TCP web client), feeds it via the
netsim (canned DHCP Offer/Ack, TCP SYN-ACK, HTTP response), and expects the
firmware to complete **two consecutive rounds**.
- Exercises: RCC, GPIO, USART, DMA, ETH MAC/DMA, timers, DHCP, TCP state
  machine, HTTP parsing.
- Expected UART (tail) includes, in order:
  ```
  === ETH HTTP GET Test ===
  DHCP Ack IP=192.168.004.002 OK
  TCP connected
  Hello from openhw HTTP server
  !CONN
  === HTTP 0b === ... (round markers)
  ```
- Pass: `=== ETH HTTP GET Test ===` + `DHCP Ack IP=192.168.004.002 OK` +
  `TCP connected` + `Hello from openhw HTTP server` + `!CONN` and ≥ 2 rounds.
- Also prints `[TX] <n>B` per transmitted frame and netsim stats.

### test_eth_irq.mjs — interrupt-driven Ethernet (eth_irq_test)
The firmware uses NVIC ETH IRQ 61 + DMAIER; the interrupt pump runs the real
`ETH_IRQHandler`. The test TXs a `PING` frame, injects a `PONG` reply, and
expects both directions to complete through the ISR.
- Exercises: NVIC, DMA (descriptors), ETH MAC/DMA, the guest ISR.
- Expected UART:
  ```
  TX done via IRQ
  RX via IRQ len=60
  PONG TX via IRQ
  ETH IRQ Test: done
  ```
- Pass: those four markers + 2 TX frames captured (`PING` then `PONG`
  payloads at bytes 14+).

### test_rx_interrupt.mjs — USART1 RX with interrupt pump
Sends `Hello\n` via `emu.sendUart` and requires the guest `USART1_IRQHandler`
(RXNEIE) to chew the bytes — two firmwares, two markers:
- `rx_interrupt_test` → `CRC=` (polynomial CRC of the line)
- `rx_crypto_test` → `DONE` (XOR cipher; prints `CRC=EFE8B569` /
  `INT CRC matches polling`)
- Pass: each firmware prints its marker.

---

## Clocks, GPIO, interrupts

### test_blinky.mjs — GPIO LED toggle (blinky)
No Ethernet. The firmware prints `tick N LED=ON/OFF` and toggles PA5 via
GPIOA ODR every ~100 ms of emulated time.
- Exercises: RCC, GPIOA MODER/ODR, USART1.
- Expected UART:
  ```
  === Blinky ===
  No ethernet required
  tick 0 LED=ON
  tick 0 LED=OFF
  tick 1 LED=ON
  ...
  ```
- Pass: banner + `No ethernet required` + exactly one `tick 0 LED=ON` and
  one `tick 0 LED=OFF` + one `tick 1 LED=ON` + ≥ 2 ODR toggles observed
  from JS (writes to 0x40020014, bit 5).
- Console prints `led_toggles=N ledOn=M ledOff=K` + the UART tail.

### test_exti.mjs — EXTI0 rising edge (exti_test)
The driver raises PA0 (`gpio_set_input`), the model pends IRQ6, the pump
runs the guest `EXTI0_IRQHandler`, and the firmware's wait loop sees the
count increment. Two edges, the second after a low period.
- Exercises: EXTI, NVIC, GPIO input.
- Expected UART (each check prints `  PASS <name>`):
  ```
  === EXTI test ===
  PASS EXTI0 fired once
  PASS EXTI0 fired on 2nd edge
  EXTI TEST DONE
  ```
- Pass: `EXTI TEST DONE`, no `FAIL `, exactly 2 edges raised.

### test_dma.mjs — DMA2 mem-to-mem copy (comprehensive_test)
Runs the DMA section of comprehensive_test: DMA2 Stream0 copies SRAM→SRAM
with the interrupt pump on.
- Exercises: DMA2, NVIC (IRQ 56), memory.
- Expected UART:
  ```
  PASS DMA2 NDTR=0
  PASS DMA2 IRQ56 ISR executed
  ```
- Pass: both `PASS DMA2 ...` lines; also prints the `PASS: <hex>` summary
  counter (a `FAIL: 00000000` in the summary is a known 0-count artifact,
  not a failure).

---

## Storage

### test_flash.mjs — FLASH program/erase (flash_test)
The firmware unlocks FLASH, erases sector 5 (0x08020000), programs 4 words,
re-erases, relocks. The JS driver applies program writes and erase fills
via `flash_is_programming` / `flash_take_erase`.
- Exercises: FLASH controller, RCC.
- Expected UART: per-check `  PASS <name>` lines (unlock, erase, program,
  verify, relock) ending in `FLASH TEST DONE`.
- Pass: `FLASH TEST DONE` + no `FAIL `.

### test_spi_flash.mjs — SPI NOR flash (spi_flash_test)
A Winbond-style 2 MB flash (JEDEC 0xEF4015) on SPI3 with software CS PB12.
Firmware writes a page, reads it back, erases a 4k sector, checks.
- Exercises: SPI3, GPIO CS gating, the flash model (WEL tracking, program
  on CS deassert, sector erase).
- Expected UART: per-check `  PASS <name>` lines (`jedec manufacturer EF`,
  `jedec device`, `WEL set (0x02)`, `readback matches payload`, `erased to
  0xFF`, ...) ending in `SPI FLASH TEST DONE`.
- Pass: `SPI FLASH TEST DONE` + no `FAIL `.

---

## Displays

### test_oled.mjs — SSD1306 OLED over I2C (oled_test)
Firmware drives a 128×64 SSD1306 over I2C1 @ 0x3C: init sequence, a text
page ("F407 OLED"), empty rows, a solid bottom bar. The JS parser
(`processOled`) builds the framebuffer from I2C command/data groups
(START/STOP boundary events).
- Exercises: I2C1 (START/STOP/address/DR), the tap, the OLED parser.
- Expected UART:
  ```
  OLED test
  OLED init done
  OLED draw done
  ```
- Pass: those three markers + text pixels > 30 on page 0 (the "F407 OLED"
  glyphs), bar = exactly 128×8 lit pixels on page 7, pages 1/2/4/6 empty.
- Console prints `textPixels(page0)=N barPixels(page7)=M onTotal=K emptyOK=bool`.

### test_tft.mjs — ILI9341 TFT over SPI (tft_test)
Firmware initializes a 240×320 ILI9341 on SPI2 (CS PB12, DC PB11) and fills
four color quadrants (R/G/B/W). The JS parser (`processTft`) decodes the
RGB565 byte stream into `emu.tft.fb` (big-endian).
- Exercises: SPI2, GPIO CS/DC gating, the TFT parser.
- Expected UART:
  ```
  TFT ILI9341 test
  TFT init done
  TFT fill done
  ```
- Pass: those markers + `emu.tft.frame() >= 319` (the fill finished all 320
  rows) + quadrant pixels read back exactly:
  `px(10,10)=0xF800` (red), `px(230,10)=0x07E0` (green),
  `px(10,310)=0x001F` (blue), `px(230,310)=0xFFFF` (white).
- Console prints the frame count + the four pixel values.

### test_ltdc.mjs — LTDC scanout (ltdc_test)
Firmware configures the LTDC (64×32 layer-0 ARGB8888 framebuffer pinned at
0x20002000), waits for the frame-end flag, checksums the gradient pixels.
- Exercises: LTDC (GCR/L1CR/PFCR/WHPCR/WVPCR/CFBAR/CFBLR/CFBLNR), IRQ 88,
  model scanout counters.
- Expected UART:
  ```
  === LTDC Test ===
  scanout started
  LTDC pixels OK
  === LTDC Test: done ===
  ```
- Pass: those markers + `ltdc_get_frame_count() >= 2` (scanout ran at least
  two frames after the firmware finished), scanline != 0xFFFF, and JS
  re-reads of the gradient pixels from guest RAM match
  `pixel(x,y) = 0xFF000000 | x<<16 | y<<8 | (x+y)` at (0,0),(1,0),(63,0),
  (63,31),(7,5).

---

## Audio

### test_audio.mjs — WAV DMA replay + capture (audio_test)
Phase A: JS loads a generated 64-sample PCM16 WAV into the model
(`audio_load_wav`); the firmware DMA1 Stream0 reads 64 samples from
I2S1_DR, and its printed checksum must equal the JS-computed sum.
Phase B: the firmware writes 16 words (1000..1015) to I2S1_DR; the model's
capture FIFO (`audio_take_capture`) must return exactly those values.
- Exercises: I2S1 (SPI1 block), DMA1 (PINC/PSIZE), the WAV source, the
  capture FIFO.
- Expected UART:
  ```
  === Audio Test ===
  DMA RX OK
  RX n=64 sum=93C40        (sum of (i*300+7)&0xFFFF for i=0..63, 8-digit hex)
  TX n=16 OK
  === Audio Test: done ===
  ```
- Pass: those markers + `sum=93C40` (or the JS-computed equivalent) +
  capture FIFO is exactly `[1000..1015]` + no `FAIL`.

### test_audio_play.mjs — I2S speaker (audio_play_test)
Firmware streams a 256-sample sine table through I2S1 TX continuously; the
JS side drains the capture FIFO (`emu.takeSpeakerSamples`).
- Exercises: I2S1 TX, the capture FIFO.
- Expected UART:
  ```
  Audio play test
  I2S1 TX sine 256 samples
  I2S ready
  ```
- Pass: `Audio play test` + `I2S ready` + > 2048 drained samples, > 90% of
  them non-zero, amplitude beyond ±0.5 (the sine is ~±0.915), and enough
  zero crossings for a 256-sample periodic wave.
- Console prints `samples=N nonzero=M min=… max=… zeroCrossings=K`.

---

## CAN bus

### test_can.mjs — arbitration + loopback (can_test)
Phase 1: loopback (BTR LBKM) TX of id 0x123 `CANLOOP!` self-echo.
Phase 2: CAN2 stages id 0x200 while CAN1 stages id 0x300 back-to-back; the
model's bus arbitration must let the lowest ID win first and both nodes
drain 2 frames each.
- Exercises: CAN1/CAN2 mailboxes, TSR/TIR, filters, arbitration, loopback.
- Expected UART:
  ```
  === CAN Test ===
  CAN loopback OK: id=0x123 data=CANLOOP!
  both TX done
  CAN arbitration OK
  === CAN Test: done ===
  ```
- Pass: those four markers + no `FAIL`.

---

## Device peripherals (browser panels)

### test_buzzer.mjs — TIM2 PWM buzzer (buzzer_test)
Firmware programs TIM2 CH1 PWM (PSC=83 → 1 MHz tick, 50 % duty) and plays a
melody by rewriting ARR/CCR1: C4(262) D4(294) E4(330) F4(349) G4(392)
A4(440) B4(494) C5(523) rest(0) G4 A4 C5. The JS side reads the modeled
TIM2 registers and reports the freq/duty changes.
- Exercises: TIM2 (CR1/CCER/PSC/ARR/CCR1), GPIO AF.
- Expected UART:
  ```
  Buzzer test
  Buzzer melody
  BUZZ 262 Hz
  BUZZ 294 Hz
  ... (one per note, with duration)
  BUZZ 523 Hz
  Buzzer done
  ```
- Pass: `Buzzer test` + `Buzzer melody` + `BUZZ 262 Hz` + `BUZZ 523 Hz` +
  `Buzzer done` + the observed freq sequence contains 262±3, 523±3 and 0.
- Console prints the UART and the observed `freqs:` sequence
  (e.g. `262,294,330,349,392,440,494,523,0,...`).

---

### test_rtc.mjs — DS3231 RTC over I2C (rtc_test)
Firmware writes pointer 0x00 + 7 BCD bytes (sec 30/min 45/hr 10/dow 3/
day 15/mon 7/yr 26) in one I2C transaction, then reads all 7 back with a
pointer-then-streaming-read transaction (register pointer persists across
transactions), verifies them, and reads the temperature pair 0x11/0x12.
The model's `I2cRegFile` (pointer-addressed, auto-increment, `% size`
clamp) backs the device; the JS side decodes the BCD registers live into
`emu.rtc.time`/`temp`.
- Exercises: I2C1 master flow (START → addr → SR1/SR2 latch → TXE/RXNE),
  register-file tap, BCD decode, signed 0.25 °C temp.
- Expected UART:
  ```
  RTC set done
  RTC read done
  RTC time=10:45:30 DOW=3 15/07/26
  RTC verify OK
  RTC temp=27.50
  RTC test done
  ```
- Pass: those markers + `emu.rtc.time` BCD-decodes to
  `{sec:30,min:45,hour:10,dow:3,day:15,mon:7,year:26}` and `temp === 27.5`.
- Register-file seed: BCD time 0x00-0x06 (10:45:30 dow3 15/07/26), temp
  MSB/LSB 0x11/0x12 = 0x1B/0x80 (27.50 C). The guest overwrites time; the
  temp regs stay at the seed (read-only on real silicon).

---

## FPU — VFPv4-SP (2026-09-09)

### test_fpu.mjs — hard-float firmware (fpu_test)
Bare-metal firmware compiled with `-mfpu=fpv4-sp-d16 -mfloat-abi=hard`
(real GCC output, not hand-written asm): the disassembly contains `vfma`
(plain `a*b+c` contracts too), `vcmpe`, `vcvt.s32.f32`, `vsqrt`, D=1
high-register arithmetic (`vadd s15,s13,s15`), and CPACR enable — the
first instruction faults (UsageFault NOCP) without it, so the gate itself
is under test. Results print as raw hex bits with per-check PASS/FAIL.
- Exercises: CPACR gate, vadd/vsub/vmul/vdiv/vsqrt, fused vfma (the
  `0x28800000` result appears only with single-rounding fusion),
  vcmp/vcmpe + NaN-unordered, vcvt int/float both directions.
- Expected UART:
  ```
  === FPU Test ===
  CPACR ok
  ADD 40700000 PASS
  SUB C0600000 PASS
  MUL 40900000 PASS
  DIV 40600000 PASS
  SQRT 3FB504F3 PASS
  FMA 28800000 PASS
  CMP OK
  CVT OK
  SPILL 43528000 PASS
  VLDM OK
  VLDR-D OK
  DSP OK
  FPU all PASS
  FPU done
  ```
- Pass: every marker above, no `FAIL`, no CPU fault.
- Gotchas the firmware caught while being written: (1) `usad8` takes 3
  operands — the 4-operand accumulate form is `usada8` (GAS rejects the
  former loudly); (2) inline-asm outputs written before later inputs are
  consumed need `=&r` early-clobbers, or GCC aliases them (observed:
  qadd8's result landed in usada8's Ra slot, printing 527 instead of 20
  — the decoder faithfully executed the garbage, which is how the alias
  was identified).
- Also wired into `test_browser.mjs` (`FPU all PASS`) and the headed sweep.
- Gotcha the wiring caught: the firmware dropdown is TWO hardcoded lists
  (native `<option>` + custom `data-value` divs in index.html) —
  `?fw=` boots eth_http silently when the custom div is missing (empty
  UART, no error). Every new preset needs all three: native option,
  custom div, `firmware.js` entry.

### test_fpu_irq.mjs — FPU state across SysTick IRQs (fpu_irq_test)
Main seeds S0–S3 + FPSCR=0 (setting CONTROL.FPCA), enables SysTick every
2000 instructions; the handler does its own float work on other S-regs
and dirties FPSCR (0.0/0.0 → IOC). Pass = S0–S3 intact, FPSCR clean after
~18 IRQs — only possible with entry reserve + first-use stacking + full
restore on return (lazy FP stacking end-to-end).
- Exercises: CPACR gate, CONTROL.FPCA, FPCCR/FPCAR/LSPACT lifecycle,
  SysTick delivery with `enable_irqs`, EXC_RETURN with FType=0.
- Expected UART:
  ```
  === FPU IRQ Test ===
  CPACR ok
  IRQ count 18
  S0 11111111 ok
  S1 22222222 ok
  S2 33333333 ok
  S3 44444444 ok
  FPSCR 00000000 ok
  FPU IRQ all PASS
  FPU IRQ done
  ```
- Pass: every marker above, IRQ count ≥ 10, no `FAIL`, no CPU fault.

### test_mpu.mjs — MPU enforcement (mpu_test)
Bare-metal firmware programs five regions (R0 FLASH 1MB RX RO-both, R1 SRAM
128KB RW-priv/XN, R2 peripherals 512MB FULL/XN, R3 1KB scratch FULL, R4 32B
no-access), sets SHCSR.MEMFAULTENA + CTRL.ENABLE|PRIVDEFENA, then probes:
legal SRAM R/W, no-access store (DACCVIOL+MMFAR), XN branch (IACCVIOL, resume
via stacked LR), unprivileged FLASH-RO read (pass), unpriv SRAM-priv store
(DACCVIOL), unpriv PPB read (DACCVIOL, no exemption), unpriv scratch store
(pass), SVC back to privileged MSP.
- Exercises: MPU region/AP/XN/subregion checks, privilege + HFNMI context,
  deferred data faults vs precise fetch faults, MemManage/HardFault routing
  via SHCSR, MSTKERR/MUNSTKERR/MLSPERR pre-validation, MMFSR/MMFAR latching.
- Expected UART:
  ```
  === MPU Test ===
  REGIONS OK
  MPU enabled CTRL=00000005 R4ASR=00000009
  LEGAL OK
  NOACC 00000001 PASS
  XNEXEC 00000001 PASS
  UPRIV-RO OK
  UPRIV-W 00000001 PASS
  UPRIV-PPB 00000001 PASS
  UPRIV-FULL OK
  MPU all PASS
  MPU done
  ```
- Pass: every marker above, no `FAIL`, no CPU fault, emulator never stops.
- Gotchas the firmware caught while being written: (1) RAM write32 splits
  into 4x write8 — the fault channel must be first-wins or MMFAR reports the
  last byte; (2) stale .bin (missing SHCSR write) escalates everything to
  HardFault — always `touch` + rebuild after editing; (3) GCC sinks the
  volatile resume-pointer store below the faulting access AND misplaces
  `&&label` values after block reordering — the handler resumes data faults
  implicitly (deferred PC already past) and XN via stacked LR, no labels;
  (4) the handler's own `push` shifts SP — capture the frame pointer in a
  naked trampoline before any push; (5) PSP must sit strictly inside its
  region (0x20001400 is past a 0x20001000+1KB region's end — even [sp,#4]
  spills   fault). See AGENTS.md §25.

---

## USB OTG FS (2026-09-10)

### test_usb.mjs — USB CDC-ACM echo (usb_cdc_test)
Bare-metal polling firmware (no interrupts) enumerates as a CDC-ACM
device (VID:PID 0483:5740) against a scripted host — this file plays USB
host through the model's `usb_*` exports (reset, enum-done, SETUP/OUT
inject, IN take), the netsim pattern applied to control transfers.
- Exercises: OTG FS init (PHY power, FDMOD, CSRST + FIFO flushes, FSIZ
  programming, GINTMSK, EP0 setup), bus reset + ENUMDNE, EP0 control
  transfers (GET_DESCRIPTOR device/config/string, SET_ADDRESS,
  SET_CONFIGURATION, GET_STATUS, SET/CLEAR_FEATURE, CDC
  SET_CONTROL_LINE_STATE + SET_LINE_CODING with 7-byte OUT stage +
  GET_LINE_CODING), EP1 bulk OUT/IN echo with TX-FIFO flush discipline
  (word-padded tails must not leak into the next transfer — caught live
  during bring-up: config desc readback shifted by 2 stale bytes).
- Expected UART:
  ```
  === USB CDC Test ===
  USB init done
  USBRST
  ENUMDNE
  REQ 0680 ... (one per SETUP)
  USB enum done
  USB echo 1
  USB echo 2
  USB echo OK
  USB done
  ```
- Pass: every marker above + both echo payloads byte-equal + no `USB FAIL`,
  no CPU fault. Model notes: IN completes synchronously at EPENA
  (whole-blob; the host slices by MPSIZ), OUT completes on short packet
  or drained XFRSIZ, EP0 needs no arming, EPENA clears on complete,
  NPTXFE/DTXFSTS derive from programmed FSIZ (program first, like
  silicon). Out of scope: host mode, OTG_HS, SOF/suspend, VBUS, DMA.
- Browser: `?fw=usb_cdc_test` runs the same script via `site/usbhost.js`
  (one frame-step per rAF); `test_browser.mjs` asserts `USB echo OK`.

| test_usb | usb_cdc_test | USB_OTG_FS | `USB echo OK`, `USB done`, byte-equal echoes |
| test_blinky_f401/411/f407g/nucleo/f429 | blinky_* | per-board SVD | board banner + LED ODR toggles on the board's pin |

## Board variants (2026-09-11)

All listed boards are Cortex-M4F — one shared CPU core, per-board SVD +
flash/RAM sizes + firmware (`site/boards.js`). Since 2026-09-11 the
coverage is the full matrix, not just blinky: `site/test_board_matrix.mjs`
(in `npm test`, 138/138) boots every portable demo build on every
compatible map and asserts its completion markers, and only passing pairs
become presets (`BOARDS_OF_FIRMWARE`, ~190 in the bundle). Family builds
come from `tools/build_family.mjs` (same sources, family link script +
`-DSTACK_TOP`) plus per-sketch Arduino FQBN rebuilds; `expectFail`
entries assert honest incompatibilities (absent silicon, 2 known model
gaps) and alert if they ever pass. Bring-up per board started as a blinky
(own link script + real LED pin) asserting banner + ODR toggles through
that board's SVD map:

| Preset | Board | SVD | Sizes | LED |
|---|---|---|---|---|
| blinky_f401 | BlackPill F401CC | stm32f401 | 256K/64K | PC13 |
| blinky_f411 | BlackPill F411CE | stm32f411 | 512K/128K | PC13 |
| blinky_f407g | Discovery F407VG | stm32f407 | 1M/192K | PD12 |
| blinky_nucleo_f401 | Nucleo-F401RE | stm32f401 | 256K/64K | PA5 |
| blinky_nucleo_f411 | Nucleo-F411RE | stm32f411 | 512K/128K | PA5 |
| blinky_f429 | Discovery F429ZI | stm32f429 | 2M/256K | PG13 |
| blinky_f407ve/ze | F407VE/ZE black | stm32f407 | 512K/192K | markers only (pinout varies) |

---

## DOOM (2026-08-14)

### test_doom.mjs — DOOM boot → menu → E1M1 gameplay + save/load (doom)
Boots the doomgeneric F407 port with the 4.2 MB shareware WAD in
`extra_mem` (0xB8000000), drives the retail menu sequence (New Game →
episode → skill 3) with change-gated key taps, holds W + fires + turns in
E1M1, then runs the **save flow**: F6 quick-save (0xC0) → Enter on the
save-slot menu → name char 'a' (0x61) + Enter → asserts the firmware's
`SAVE ok slot=0 bytes=NNNN` UART print and the ABI `saveFlag == 1`.
- Exercises: guest↔driver ABI ring (keyWr/keyRd), EXTRAM save staging
  (fd 0x7f00 → 0xC0080000), newlib `rename` = `_link`+`_unlink` commit
  hook, CMAP256 framebuffer + BGRA palette, I2S mixer audio.
- Key ABI asserts: `save: flag=1 size>0 … qss=0` and the EXTRAM blob
  contains the save archive (level state, ~25 KB).
- Key-ring gotcha: the guest drains ~1 (D,U) pair per frame, so the
  harness re-asserts held W sparingly (every 25 iters) and taps
  turn/fire every 40 — per-iteration spam overflows the 256-byte ring
  and clobbers the queued save-menu events (keyRd freezes, menu opens
  at SaveDef, sse=0, qss=-2 forever).
- Pass: `Z_Init` + `adding doom1.wad` + `I_InitGraphics` boot markers,
  palette non-zero, fb at 0xC0700008, `phase === 'play'`, `SAVE ok slot=0`.

---

## Expected PASS summary

| Test | Firmware | Peripherals | Key output markers |
|---|---|---|---|
| test_flow | eth_http | ETH/DMA/USART/TIM | `TCP connected`, `Hello from openhw HTTP server`, `!CONN`, ≥2 rounds |
| test_eth_irq | eth_irq_test | NVIC/DMA/ETH | `TX done via IRQ`, `ETH IRQ Test: done` |
| test_doom | doom | ABI ring/EXTRAM/I2S | `SAVE ok slot=0 bytes=…`, fb changes ≥ 20, audio peak > 0 |
| test_rx_interrupt | rx_interrupt_test, rx_crypto_test | USART1/NVIC | `CRC=`, `DONE` |
| test_blinky | blinky | GPIOA/RCC/USART | `tick 0 LED=ON`, `No ethernet required` |
| test_exti | exti_test | EXTI/NVIC/GPIO | `EXTI TEST DONE` |
| test_dma | comprehensive_test | DMA2/NVIC | `PASS DMA2 NDTR=0` |
| test_flash | flash_test | FLASH/RCC | `FLASH TEST DONE` |
| test_spi_flash | spi_flash_test | SPI3/flash/GPIO | `SPI FLASH TEST DONE` |
| test_oled | oled_test | I2C1/tap | `OLED draw done`, bar=1024 px |
| test_tft | tft_test | SPI2/GPIO/tap | `TFT fill done`, px=0xF800/0x07E0/0x001F/0xFFFF |
| test_ltdc | ltdc_test | LTDC/NVIC | `LTDC pixels OK`, frames≥2 |
| test_audio | audio_test | I2S1/DMA1 | `RX n=64 sum=93C40`, `TX n=16 OK` |
| test_audio_play | audio_play_test | I2S1 | `I2S ready`, samples>2048 |
| test_can | can_test | CAN1/CAN2 | `CAN arbitration OK`, `CAN loopback OK` |
| test_buzzer | buzzer_test | TIM2/GPIO | `BUZZ 262 Hz`, `BUZZ 523 Hz`, `Buzzer done` |
| test_rtc | rtc_test | I2C1/regfile | `RTC verify OK`, `RTC time=10:45:30 DOW=3 15/07/26`, temp=27.5 |
| test_fpu | fpu_test | VFPv4-SP/CPACR | `FPU all PASS`, `FMA 28800000 PASS`, `FPU done` |
| test_fpuirq | fpu_irq_test | VFPv4-SP/SysTick/NVIC | `FPU IRQ all PASS`, `S0 11111111 ok`, `FPSCR 00000000 ok` |
| test_mpu | mpu_test | MPU/SCB | `MPU all PASS`, `MPU done`, exact MMFSR/MMFAR per probe |

Plus the Rust unit suite: `cargo test` (135 tests — CPU/decoder incl. FPU
encoding/semantics/stacking tests and 36 interrupt/system-fidelity tests
(BASEPRI/FAULTMASK, priority order, nesting, tail-chain, SLEEPONEXIT, DWT,
SVC/UsageFault escalation, PRIGROUP split, SEV/WFE event, CPS, STKALIGN,
SHCSR/ICSR, LDRT, UNALIGN_TRP, DIV_0_TRP, FPCCR.USER, CONTROL, USERSETMPEND,
BusFault, NONBASETHRDENA, ITM, EXCCNT, Device-unaligned, SEVONPEND,
STIR, FOLDCNT),
plus CAN,
SPI/I2C taps, DCMI, WAV,
LTDC, register files — 135/135 green).

## Gotchas

- `comprehensive_test` prints `FAIL: 00000000` for the 0-count counter —
  known artifact, not a failure (test_dma checks only the `PASS DMA2` lines).
- Tests are step-count driven; they stop early on their success marker, so a
  PASS usually takes only a few seconds.
- The site tests are fresh-process runs; the browser smoke boots all five
  device firmwares by navigation (restarting Chrome between boots) and can
  hit mem_map errors on a memory-starved box (see AGENTS.md §11 Gotchas) —
  restart Chrome with a fresh `--user-data-dir` if boots start failing.
