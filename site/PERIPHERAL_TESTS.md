# Peripheral tests — what they do and what to expect

Every peripheral test boots a small bare-metal firmware into the emulator
(Unicorn + the WASM peripheral model) and checks two things:

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

## Expected PASS summary

| Test | Firmware | Peripherals | Key output markers |
|---|---|---|---|
| test_flow | eth_http | ETH/DMA/USART/TIM | `TCP connected`, `Hello from openhw HTTP server`, `!CONN`, ≥2 rounds |
| test_eth_irq | eth_irq_test | NVIC/DMA/ETH | `TX done via IRQ`, `ETH IRQ Test: done` |
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

Plus the Rust unit suite: `cargo test --release` (19 tests covering CAN
arbitration, filters, FIFOs, loopback, SPI taps/CS, I2C taps, DCMI frames,
WAV parsing, LTDC scanout, register files — 19/19 green).

## Gotchas

- `comprehensive_test` prints `FAIL: 00000000` for the 0-count counter —
  known artifact, not a failure (test_dma checks only the `PASS DMA2` lines).
- Tests are step-count driven; they stop early on their success marker, so a
  PASS usually takes only a few seconds.
- The site tests are fresh-process runs; the browser smoke boots all five
  device firmwares by navigation (restarting Chrome between boots) and can
  hit mem_map errors on a memory-starved box (see AGENTS.md §11 Gotchas) —
  restart Chrome with a fresh `--user-data-dir` if boots start failing.
