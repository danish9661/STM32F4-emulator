# Peripheral implementation matrix

Every peripheral in the Rust model (`stm32-periph-wasm/src/peripherals/`),
its register coverage, behavior level, and how it's exercised. Line counts
from `wc -l` (all files, as of 2026-08-09).

**Levels**

- **Detailed** — real register semantics and real behavior (data movement,
  state machines, interrupts, real algorithms).
- **Partial** — registers modeled but with simplified or missing behavior
  (constants, canned data, no hardware path).

**Legend** — IRQ: generates NVIC interrupts; tick: implements `fn tick()`;
ext: connects to external devices (SPI flash, EEPROM, display, ...).

## Summary

| | Count |
|---|---|
| Modules | 33 |
| Detailed | 28 |
| Partial | 5 |
| Stub / passthrough-only | 0 |

## The matrix

| Peripheral | Loc | Level | What's implemented | IRQ | tick | ext |
|---|---|---|---|---|---|---|
| ADC | 183 | Detailed | SR/CR1/CR2/SMPR1-2/JOFR/HTR/LTR/SQR1-3/JSQR/JDR/DR; SWSTART-gated conversion, real sampling-time logic (SMPR lookup), channel values (16/17 = temp/Vref, 18 = Vbat, others LCG pseudo-random), EOC/OVR flags | Y (EOC/OVR → 18/47) | N | N |
| CAN | 494 | Detailed | MCR/MSR/TSR/RF0R/RF1R/IER/ESR/BTR; 3 TX + 2 RX mailboxes, filter banks (FMR/FM1R/FS1R/FFA1R/FA1R + 56 filter words), TX request → TXOK/RQCP, RX FIFO release decrements FMP, INIT/SLEEP transitions; **two-node bus arbitration** (lowest ID wins, ties by node/mailbox), winner broadcasts to every RX passing its filters (including itself), BTR LBKM loopback delivers only to the sender | Y (TX/RX/SCE → 19/63) | N | N |
| CRC | 39 | Detailed | Real CRC-32 (poly 0x04C11DB7, MSB-first), accumulated on DR writes; DR/IDR/CR, RESET → 0xFFFFFFFF | N | N | N |
| CRYP | 628 | Detailed | Full crypto core via real `aes`/`des` crates: AES-128/192/256 ECB/CBC/CTR, DES + 3DES (EDE3), GCM (GHASH + GF(2¹²⁸) + CTR + tag), CCM (CBC-MAC + CTR); 64-byte FIFO with IFEM/IFNF/OFNE/OFFU/BUSY, datatype byte-swap | Y (79: OFNE/IFNF) | N | N |
| DAC | 135 | Detailed | CR/SWTRIGR/all DHR regs/DOR1-2/SR; DOR update on trigger + writes, LFSR noise, triangle-waveform counter/direction, MAMP masks | N | N | N |
| DBGMCU | 37 | Partial | IDCODE constant 0x10006411; CR/APB1FZ/APB2FZ stored/masked | N | N | N |
| DCMI | 81 | Partial | CR/SR/RIS/IER/ICR/ESCR/ESUR/CWSTRT/CWSIZ/DR; consumes a JS-fed camera frame with VSYNC/LINE/FRAME/OVR semantics and a 4-deep FIFO. Source: `ext_devices.camera` (pumped every step) or `emu.camera.feed(w,h,pixels)`; guest-side coverage: `dcmi_test` | Y (78) | N | N |
| DMA | 212 | Detailed | LISR/HISR/IFCR + 8 streams (CR/NDTR/PAR/M0AR/M1AR/FCR); EN queues a `DmaTransfer` (copy done by JS via `dma_get_pending`/`dma_set_completed`), TCIF/HTIF, stream IRQ, double-buffer M1AR, dir/mem2mem | Y (11-18, 56-63) | N | N |
| ETH | 311 | Detailed | All 4 blocks (MAC/MMC/PTP/DMA) full register maps; PHY emulation (MIIAR read → fixed PHY regs), write-1-clear DMASR, DMAIER masking, AIS/NIS, TX/RX done → TS/RS bits, DMABMR soft-reset, DMAOMR ST/SR → `eth_signal_tx/rx_poll` atomics | Y (61) | Y | N |
| EXTI | 70 | Detailed | IMR/EMR/RTSR/FTSR/SWIER/PR with correct line→IRQ mapping; **GPIO edge-trigger path**: `scan_lines` per tick compares GPIO line levels vs `last_state`, RTSR/FTSR gating, line→IRQ map (0-4→6-10, 5-9→23, 10-15→40) | Y (6-10/23/40) | N | N |
| FLASH | 80 | Detailed | ACR, KEYR two-step unlock (0x45670123→0xCDEF89AB), OPTKEYR, SR write-1-clear, CR + LOCK/PSIZE, OPTCR1; program (PG) and sector-erase (SER) dispatch into the emulated flash backing buffer via `flash_erase_applied` (JS driver gated on `syncFlashProtection`) | N | N | N |
| FSMC | 100 | Partial | 4 banks 0x60000000-0xA0001000; BCR/BTR/PCR stored per bank; data-space accesses forward to a JS device via the bank tap (`fsmc_tap` / `ext_devices.fsmcDevices`), which reports the ACCESS ADDRESS as well as the value so an 8080-mode display can decode its RS/DC line. An untapped bank reads 0; guest-side coverage: `fsmc_test` | N | N | N |
| GPIO | 229 | Detailed | MODER/OTYPER/OSPEEDR/PUPDR/IDR/ODR/BSRR/LCKR/AFRL/AFRH; ODR/BSRR drive output_state + write callbacks, MODER→input clears output, IDR = input_state + read callbacks, 11 ports (A-K) | N | N | Y |
| HASH | 164 | Detailed | Real SHA-1/MD5/SHA-256 (sha1/md5/sha2 crates) with NBLW/length handling; CR/INIT, DIN FIFO, STR, HR + HASH_HR, IMR/SR, 54 CSR words | Y (80) | N | N |
| I2C | 204 | Detailed | Full master state machine (Idle→Start→Addr→Active read/write), START/STOP, SWRST, address match vs attached EEPROMs (NACK/AF on miss), byte R/W, SR1/SR2 ordering semantics, event/buffer/error IRQ masking | Y (31-34, 72-73) | N | Y |
| I2S | 150 | Detailed | CR1/CR2/SR/DR/CRC/I2SCFGR/I2SPR; DR reads consume WAV-backed PCM16 audio (`audio_load_wav`) with fallback to a synthetic generator, DR writes push into a capture FIFO (`audio_take_capture`), SR flags | Y (35/36/51) | N | N |
| IWDG | 70 | Detailed | KR keys (0x5555 write-enable, 0xAAAA reload, 0xCCCC start), PR prescaler, RLR; instruction-count driven; underflow → `request_watchdog_reset()`; SR PVU/RVU | reset (not IRQ) | N (self-timed) | N |
| LTDC | 260 | Detailed | Global regs + 2 layers (CR/whpcr/wvpcr/ckcr/pfcr/cacr/dccr/bfcr/cfbar/cfblr/cfblnr/clutwr); real scanline/frame advance from SSCR/BPCR/AWCR geometry (`ltdc_get_scanline`/`ltdc_get_frame_count`), LIF at LIPCR + frame-end F flag, IRQ 88; browser console renders layer0 (ARGB8888/RGB565) to a canvas | Y (88) | N | N |
| NVIC | 226 | Detailed | ISER/ICER/ISPR/ICPR/IABR/priority; u128 pending mask, enable/active arrays; `set_intr_pending` sets pending only (no auto-enable — pending is delivered only when ISER is set, and disabled pending stays set until taken or ICPR); `get_and_clear_next_intr_pending` delivers the highest-priority enabled IRQ, skips disabled ones without clearing; `has_pending` reflects deliverable-only; ICSR VECTPENDING returns the exception vector number; SysTick periodic pending; `in_interrupt` | controller | via System::tick | N |
| PWR | 47 | Partial | CR/CSR masked storage, WUF on valid wakeup write pattern | N | N | N |
| RCC | 221 | Detailed | CR/PLLCFGR/CFGR/CIR/reset-enable/low-power/BDCR/CSR/SSCG/PLLI2SCFGR/PLLSAI/DCKCFGR/CKGATENR/DCKCFGR2; HSE/PLL ready after instruction-count delays, HSIRDY, SWS mirrors SW, real freq math (system/pll/ahb/apb1/apb2), enable-bit gating map, LSE/LSI ready timing | N | N | N |
| RNG | 85 | Detailed | CR/SR/DR; LCG pseudo-random 32-bit values regenerated every 40 inst, DRDY, error flags + IRQ | Y (80) | N | N |
| RTC | 198 | Detailed | TR/DR/CR/ISR/PRER/WUTR/CALIBR/ALRMAR/ALRMBR/WPR/SSR/SHIFTR/timestamp/CALR/TAFCR/ALRMASSR/ALRMBSSR + 20 backup regs; BCD time advances via PRER prescaler vs instruction count, alarm A/B matching with don't-care masks | Y (43) | N (advances on access) | N |
| SAI | 106 | Detailed | GCR + 2 blocks (CR1/CR2/FRCR/SLOTR/IM/SR/CLRFR/DR); shares the WAV-backed audio path with I2S (block routes DR to audio when I2SMOD is set), SR flags on DR access, masked interrupt | Y (87) | N | N |
| SCB | 130 | Detailed | CPUID (Cortex-M4 r0p1), ICSR set/clear pending (PendSV/SysTick) + pending-vector report, AIRCR VECTKEY + SYSRESETREQ → `request_watchdog_reset()`, VTOR, SCR/CCR/SHPR/SHCSR/CFSR/HFSR/DFSR/MMFAR/BFAR/AFSR/CPACR | indirect (PendSV/SysTick) | N | N |
| SDIO | 143 | Detailed | Full register set + emulated SD card state machine (Idle→Ident→Stby→Tran), canned responses for CMD0/2/3/5/7/8/9/10/13/16/17/18/41/55, RCA matching, data-transfer simulation (DCOUNT/FIFOCNT, CMD17/18), status flags + ICR/MASK | Y (49) | N | N |
| SPI | 157 | Detailed | CR1/CR2/SR/DR/RXCRC/TXCRC/I2SCFGR/I2SPR; full-duplex 8/16-bit transfers to attached device with CS selection via GPIO, I2S mode audio generation, TXE/RXNE toggling; **CS edges delivered via GPIO write callbacks** (`register_cs_callbacks`, sw_spi pattern) so attached-device CS deassert is observed immediately | Y (35/36/51) | N | Y |
| SW_SPI | 105 | Detailed | Bit-banged SPI via GPIO callbacks (CS/CLK/MOSI/MISO): shift register, 8-bit framing, forwards bytes to attached device; CS edge resets | N | N | Y |
| SYSCFG | 52 | Partial | MEMRM/PMC/EXTICR[4]/CMPCR masked storage; CMPCR read toggles COMP bit | N | N | N |
| SYSTICK | 55 | Detailed | CSR/RVR/CVR/CALIB; enabling programs `nvic.systick_period` → periodic SYSTICK pending from `System::tick`; CVR write resets trigger point | indirect (SYSTICK) | N | N |
| TIM | 243 | Detailed | CR1/CR2/SMCR/DIER/SR/EGR/CCMR1-3/CCER/CNT/PSC/ARR/CCR1-6/RCR/DCR/DMAR/OR; instruction-count counter with PSC prescaler, up/down/center-aligned, UIF+UIE IRQ on overflow, CC match IRQs, UG update event, PWM duty | Y (per-timer) | **Y** | N |
| USART | 126 | Detailed | SR/DR/BRR/CR1-3/GTPR; TX pushes to global UART output buffer (drained by `get_uart_output`), RX via `rx_byte` (JS `uart_rx_byte`) with 64-byte RX buffer, RXNE/ORE, TXE/TC always set; IRQ on RXNEIE/TCIE/TXEIE | Y (37/38/39/52/53/71/82/83) | N | N |
| WWDG | 81 | Detailed | CR/CFR/SR; countdown at 256×prescaler instructions, early-wakeup flag + IRQ (EWI), underflow + WDGA → `request_watchdog_reset()` | Y (0) | N (self-timed) | N |

## External devices (`stm32-periph-wasm/src/ext_devices/`)

`ext_devices/{lcd,touchscreen,usart_probe,display}.rs` (protocol-specific
device models) were removed in commit `13d7cdb` in favor of two generic,
protocol-agnostic bus taps — real device *behavior* now lives in JS on top
of them (`site/emulator.js`'s `oled`/`tft`/`rtc` blocks implement the
SSD1306/ILI9341/DS3231 protocols client-side; see [components.md](components.md)
for the public attachment API built on the same taps).

| Device | Lines | Behavior |
|---|---|---|
| SpiFlash | 399 | SPI NOR flash: JEDEC ID (0x9F), device ID (0x90), status regs (0x05/0x35), ReadData (0x03)/FastRead (0x0B) streaming, WriteEnable (0x06), PageProgram (0x02), SectorErase4k (0x20) **with CS-deassert commit** (program ANDs bits, erase sets 0xFF, WEL auto-clears after program/erase like real W25Q). MISO timing: `dummy_pending` returns 0 while the command+address bytes are clocked in (one dummy per written byte), so the first real data byte appears only on the byte after the address phase |
| I2cEeprom | 78 | I²C EEPROM: 1/2-byte address phase, sequential byte read/write into RAM copy |
| I2cRegfile | 139 | Pointer-addressed I²C register file (DS3231 RTC): auto-increment pointer, JS seeds/reads registers via `i2c_regfile_get`/`set` |
| SpiTap | 54 | Generic protocol-agnostic SPI slave: CS/DC edges + bytes queued for JS (`spi_take_events`), JS answers reads via `spi_push_miso` — the oled/tft/custom-device bridge |
| I2cTap | 38 | Generic protocol-agnostic I²C slave: START/STOP + bytes queued for JS (`i2c_take_events`), JS answers reads via `i2c_push_rx` |

## How the matrix is exercised

- **UART + GPIO + CRC + HASH + CRYP**: `echo_test`, `blink_serial`,
  `rx_interrupt_test`, `rx_crypto_test`, `crypto_test`, `crypto_deep_test`
  — all pass headless with real interrupt handlers (`node site/test_rx_interrupt.mjs`).
- **TIM + RCC + NVIC + IWDG**: `timer_test`, `hal_test`, `periph_test`,
  `deep_periph_test`, `new_periph_test`.
- **SPI + FSMC + SDIO + DCMI + LTDC + I2C + CAN + ADC + DAC + RNG + RTC**:
  `spi_tft_test`, `comprehensive_test`, `edge_test`, `saturn`/`monox`
  printer firmwares (upstream heritage).
- **ETH**: `eth_http`, `eth_dhcp`, `eth_test` — full DHCP/TCP/HTTP flows
  (see [benchmarks.md](benchmarks.md)).

Probe all firmware binaries with `node site/probe_firmwares.mjs`; each
prints a banner over UART when it boots.

## Known gaps / model shortcuts

- NVIC pending is set by peripherals regardless of ISER (hardware-accurate);
  the pump only ever runs enabled IRQs.
- USART ignores its ext-device argument (uses the global UART buffer);
  the UsartProbe device is wired through the SPI lookup instead.
- DMA peripheral-side accesses are chunked in Rust (`dma_periph_read`/
  `dma_periph_write` — one WASM call per transfer instead of size/4);
  RAM-to-RAM copies still run in the JS driver because guest memory lives
  in Unicorn, not in the Rust model.
- FLASH programs/erases emulated flash; DCMI consumes JS-fed camera frames
  (no pixel source). LTDC scanout, SAI/I2S WAV-backed audio, and CAN
  two-node bus arbitration are implemented (see rows above).
- Timers/ADC/RNG/RTC/IWDG/WWDG are instruction-count driven, not
  wall-clock driven (deterministic across machines).
