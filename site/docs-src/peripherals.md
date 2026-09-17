# Peripheral implementation matrix

Every peripheral in the Rust model (`stm32-periph-wasm/src/peripherals/`),
its register coverage, behavior level, and how it's exercised. Line counts
from `wc -l` (all files).

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
| Modules | 41 |
| Detailed | 41 |
| Partial | 0 |
| Stub / passthrough-only | 0 |

## The matrix

| Peripheral | Loc | Level | What's implemented | IRQ | tick | ext |
|---|---|---|---|---|---|---|
| ADC | 624 | Detailed | SR/CR1/CR2/SMPR1-2/JOFR/HTR/LTR/SQR1-3/JSQR/JDR/DR; SWSTART-gated conversion, real sampling-time logic (SMPR lookup), channel values (16/17 = temp/Vref, 18 = Vbat, others LCG pseudo-random), EOC/OVR flags; analog watchdog (AWDEN/AWDSGL/AWDCH + HTR/LTR compare, AWD flag + IRQ on excursion); DMA request on EOC (CR2 DMA bit stages samples for `adc_take_dma`); shared **ADC_Common @ 0x40012300** — CSR mirrors EOC1/2/3+OVR1/2/3 (read-only), CDR = ADC1.DR\|ADC2.DR<<16, CCR DUAL mode + CDR simultaneous latch (both-ready peek, `adc_dual_latched` probe — mock `t_adc_dual`); `C_ADC` Keil alias unified so the F429 map gets the block | Y (EOC/OVR/AWD → 18/47) | N | N |
| CAN | 844 | Detailed | MCR/MSR/TSR/RF0R/RF1R/IER/ESR/BTR; 3 TX + 2 RX mailboxes, filter banks (FMR/FM1R/FS1R/FFA1R/FA1R + 56 filter words), TX request → TXOK/RQCP, RX FIFO release decrements FMP, INIT/SLEEP transitions; **two-node bus arbitration** (lowest ID wins, ties by node/mailbox), winner broadcasts to every RX passing its filters (including itself), BTR LBKM loopback delivers only to the sender; TEC/REC live + bus-off/EPVF/EWGF/LEC via `can_note_error` (mock `t_can_err`); CAN FD via `can_inject_fd` (DLC table, FDF, 64B window 0x320/0x3A0, RFOM shift — mock `t_canfd`) + FDCAN bit-timing NBTP/DBTP @0x3E0/0x3E4 with nominal/data `can_fd_cost` split (mock `t_fdcan_timing`; bxCAN has no FD regs, protocol timing beyond rates unmodeled) | Y (TX/RX/SCE → 19/63) | N | N |
| CRC | 40 | Detailed | Real CRC-32 (poly 0x04C11DB7, MSB-first), accumulated on DR writes; DR/IDR/CR, RESET → 0xFFFFFFFF | N | N | N |
| CRYP | 629 | Detailed | Full crypto core via real `aes`/`des` crates: AES-128/192/256 ECB/CBC/CTR, DES + 3DES (EDE3), GCM (GHASH + GF(2¹²⁸) + CTR + tag), CCM (CBC-MAC + CTR); 64-byte FIFO with IFEM/IFNF/OFNE/OFFU/BUSY, datatype byte-swap | Y (79: OFNE/IFNF) | N | N |
| DAC | 150 | Detailed | CR/SWTRIGR/all DHR regs/DOR1-2/SR; DOR update on trigger + writes (register model real — mock `t_dac_sink`), LFSR noise, triangle-waveform counter/direction, MAMP masks; no analog pin layer (DOR readback IS the sink value + `sink_sample` probe — mock `t_dac_sink`) | N | N | N |
| DBGMCU | 164 | Detailed | IDCODE 0x10006411; CR/APB1FZ/APB2FZ stored/masked; freeze HONORED — `dbgmcu_set_halt()` + halted/frozen TIM2-14/WWDG/IWDG neither count nor fire, I2C START/STOP sequencing holds (SMBUS-timeout bits 21-23), no catch-up burst on resume; name alias unified (`DBGMCU` monox + `DBG` Keil DFP) so every map gets the block; cargo `dbgmcu_freeze_*` tests | N | N | N |
| DCMI | 545 | Detailed | CR/SR/RIS/IER/MIS/ICR/ESCR/ESUR/CWSTRT/CWSIZ/DR + JPEG/ESS/CROP bytes; full camera pipeline (VSYNC/LINE/FRAME/ERR/OVR, CROP window, JPEG word-pack, FIFO, IRQ78) fed by `dcmi_feed_frame` (JS camera) + pin-sync harness gating (VSYNC-arm `dcmi_set_sync`, HSYNC blanking hold, PCLK divider — mock `t_dcmi`); guest coverage: `dcmi_test` | Y (78) | Y | Y |
| DMA | 235 | Detailed | LISR/HISR/IFCR + 8 streams (CR/NDTR/PAR/M0AR/M1AR/FCR); EN queues a `DmaTransfer` (copy done by JS via `dma_get_pending`/`dma_set_completed`), TCIF/HTIF, stream IRQ, double-buffer M1AR, dir/mem2mem | Y (DMA1: 11-18, DMA2: 56-63) | N | N |
| DMA2D | 366 | Detailed | F429 Chrom-ART @0x4002B000: R2M/M2M/M2M+PFC/blend jobs staged at CR START for `dma2d_take_job()`; the JS driver converts via pure `convert_px`/`blend_px` (ARGB8888/RGB888/RGB565, over-operator) and scatters with OR offsets, then `dma2d_job_done()` sets TCIF + pends IRQ56 (TCIE); unsupported color mode raises CEIF; START clears on completion; guest coverage: `dma2d_test` | Y (56) | N | N |
| DWT | 119 | Detailed | CTRL+CYCCNT @0xE0001000: CYCCNT advances with the instruction clock while CTRL.CYCCNTENA + DEMCR.TRCENA are both set (DEMCR lives in the same file, own 4-byte slot @0xE000EDFC); EXCCNT exact exception-take count, FOLDCNT exact IT-skip count from the decoder; NOCYCCNT reads 0; CPICNT/SLEEPCNT/LSUCNT stay 0 (would be fabricated precision) | N | N | N |
| ETH | 2579 | Detailed | All 4 blocks (MAC/MMC/PTP/DMA) full register maps incl. WOL wakeup-filter CRC, addend drift, PPS counter/level, VLAN gate+invert, PTP snapshots, TX checksum insert, RX checksum status, accept filtering (perfect/hash/promiscuous/DAIF/ROD), wire pacing, deferral/collision reports, RBUS head-only RX; DP83848-style PHY/MDIO peer; MII/RMII pin mirrors in GPIO IDR; BSD sockets over real LwIP; PPS pin live on PB5 IDR mirror (same advance-then-read path as the counter — mock `t_pps`), nibble data + 25/50 MHz clocks electrical-only (level mirrors are the contract); ULPI rate report 480/12 in GUSBCFG + scope probes (mock `t_ulpi_rate`); guest coverage: `eth_feat_test` (60+ markers, both maps), `eth_http/dhcp/test/irq/pins`, `lwip_demo` | Y (61/62) | Y | Y |
| EXTI | 118 | Detailed | IMR/EMR/RTSR/FTSR/SWIER/PR with correct line→IRQ mapping; **GPIO edge-trigger path**: `scan_lines` per tick compares GPIO line levels vs `last_state`, RTSR/FTSR gating, line→IRQ map (0-4→6-10, 5-9→23, 10-15→40) | Y (6-10/23/40) | Y | N |
| FLASH | 348 | Detailed | ACR, KEYR two-step unlock (0x45670123→0xCDEF89AB), OPTKEYR, SR write-1-clear, CR + LOCK/PSIZE, OPTCR1; program (PG) and sector-erase (SER) dispatch into the emulated flash backing buffer via `flash_erase_applied`; error flags WRPERR (nWRP-protected sector/MER, locked OPTSTRT)/PGSERR (bad SNB, broken unlock sequence)/PGAERR (STRT while BSY), OPTLOCK/OPTSTRT option-byte sequence (mock `t_flash_err`); RDP levels L0/L1/L2 + MER timing window (mock `t_flash_rdp`) | N | N | N |
| FSMC | 538 | Detailed | 4 banks 0x60000000-0xA0001000; BCR/BTR/BWTR + NAND PCR/SR/PMEM/PATT/ECCR with a REAL ECC engine (24-bit order-sensitive parity over data-space writes while ECCEN set, reset on ECCEN rise, frozen while clear; vendor Hamming matrix proprietary so the code differs from silicon bit-for-bit — the round-trip contract is what firmware checks); data-space accesses forward to a JS device via the bank tap (`fsmc_tap` / `ext_devices.fsmcDevices`), which reports the ACCESS ADDRESS as well as the value so an 8080-mode display can decode its RS/DC line. wait-state timing live (DATAST+ADDSET `busy_until`, SR BUSY bit 5) + NAND backing array via `fsmc_bind_nand` (program 1→0, erase restores — mock `t_fsmc`); untapped+unbound banks read 0; FMC alias unified so the Keil F429 map gets the block; guest coverage: `fsmc_test` | N | N | Y |
| FPU | 67 | Detailed | System side @0xE000EF34 (SVD omits MVFR0-2, claimed explicitly): FPCCR/FPCAR/FPDSCR + M4F ID values (0x10110021/0x11000011/0x00000040); FPEXC.EN shadow — effective enable is CPACR-full && EN, EX derived live from LSPACT; S0-S31/FPSCR live in the CPU core with lazy stacking on exception entry | N | N | N |
| GPIO | 305 | Detailed | MODER/OTYPER/OSPEEDR/PUPDR/IDR/ODR/BSRR/LCKR/AFRL/AFRH; ODR/BSRR drive output_state + write callbacks, MODER→input clears output, IDR = input_state + read callbacks, 11 ports (A-K) | N | N | Y |
| HASH | 311 | Detailed | Real SHA-1/MD5/SHA-256 (sha1/md5/sha2 crates) with NBLW/length handling; CR/INIT, DIN FIFO, STR, HR + HASH_HR, IMR/SR, 54 CSR words; HMAC live (MODE/LKEY, FIPS-198 ipad/opad, RFC 2202/SHA-256 vectors — mock `t_hash`/`t_hmac_sha1`) | Y (80) | N | N |
| I2C | 558 | Detailed | Full master state machine (Idle→Start→Addr→Active read/write), START/STOP, SWRST, address match vs attached EEPROMs (NACK/AF on miss), byte R/W, SR1/SR2 ordering semantics, event/buffer/error IRQ masking; SMBus PEC (CRC-8, ENPEC-gated, PECERR — mock `t_i2c_smbus`), GCALL/ALERT/ARP model answers (GENCALL/SMBALERT, host-notify via `i2c_arm_smbus_alert` — mock `t_smbus_addr`), ARLO on armed loss (harness = other master), TIMEOUT after 32 Active ticks with SMBUS set; single-master only otherwise (bus always won — mock `t_i2c_multi`) | Y (31-34, 72-73) | Y | Y |
| I2S | 150 | Detailed | CR1/CR2/SR/DR/CRC/I2SCFGR/I2SPR; DR reads consume WAV-backed PCM16 audio (`audio_load_wav`) with fallback to a synthetic generator, DR writes push into a capture FIFO (`audio_take_capture`), SR flags | Y (35/36/51) | N | N |
| ITM | 163 | Detailed | 32 STIMn ports @0xE0000000 (absent from every SVD, registered explicitly), TER-gated: port 0 sinks to the UART console (the CMSIS ITM_SendChar idiom) when TCR.ITMENA + TER[0]; ports 1-31 queue per-port streams drained via `itm_take_port(n)` (`itm_port_pending(n)` backlog, 4 KB cap); TER-disabled writes dropped, STIM reads ready only when enabled | N | N | N |
| IWDG | 84 | Detailed | KR keys (0x5555 write-enable, 0xAAAA reload, 0xCCCC start), PR prescaler, RLR; instruction-count driven; underflow → `request_watchdog_reset()`; SR PVU/RVU | reset (not IRQ) | Y | N |
| LTDC | 295 | Detailed | Global regs + 2 layers (CR/whpcr/wvpcr/ckcr/pfcr/cacr/dccr/bfcr/cfbar/cfblr/cfblnr/clutwr); real scanline/frame advance from SSCR/BPCR/AWCR geometry (`ltdc_get_scanline`/`ltdc_get_frame_count`), LIF at LIPCR + frame-end F flag, IRQ 88; browser console renders layer0 (ARGB8888/RGB565) to a canvas | Y (88) | Y | N |
| MPU | 187 | Detailed | TYPE/RNR/RBAR/RASR @0xE000ED90: CTRL ENABLE/HFNMIENA/PRIVDEFENA latched sticky; every CPU access gated through `check_range` (privilege/AP/XN/subregions, highest-numbered-region wins; background priv-iff-PRIVDEFENA); violations raise MemManage (escalate to HardFault when MEMFAULTENA clear) with exact MMFSR/MMFAR; guest coverage: `mpu_test` | fault (MemManage/HardFault) | N | N |
| NVIC | 335 | Detailed | ISER/ICER/ISPR/ICPR/IABR/priority; u128 pending mask, enable/active arrays; `set_intr_pending` sets pending only (no auto-enable — pending is delivered only when ISER is set, and disabled pending stays set until taken or ICPR); `get_and_clear_next_intr_pending` delivers the highest-priority enabled IRQ, skips disabled ones without clearing; `has_pending` reflects deliverable-only; ICSR VECTPENDING returns the exception vector number; SysTick periodic pending; `in_interrupt` | controller | via System::tick | N |
| PWR | 276 | Detailed | CR/CSR masked storage (incl. F429 VOS/ODEN/ODSWEN/UDEN family fields), WUF/SBF via emulator wakeup path (CSR bit 0/1, cleared via CR CWUF/CSBF); PVD live (PVDO follows PVDE, pinned-healthy rail); VOS settle window (~1000 inst); F429 overdrive/under-drive handshake (ODEN→ODRDY, ODSWEN→ODSWRDY, UDEN→UDRDY after ~5000-inst windows, ready drops with enable) | N | N | N |
| QSPI | 429 | Detailed | QUADSPI @0xA0001000 (absent from the F407 SVD, registered explicitly at its conventional base): indirect read/write to a `qspi_register_flash()` image bound before init (mirrors the SPI/FSMC tap pattern); 32-byte data FIFO, BUSY/TC/FT flags; guest coverage: `qspi_test` (+ in-browser CDP smoke) | N | N | Y |
| RCC | 273 | Detailed | CR/PLLCFGR/CFGR/CIR/reset-enable/low-power/BDCR/CSR/SSCG/PLLI2SCFGR/PLLSAI/DCKCFGR/CKGATENR/DCKCFGR2; HSE/PLL ready after instruction-count delays (RDY clears at once when ON drops — mock `t_rcc`), HSIRDY, SWS mirrors SW, real freq math (system/pll/ahb/apb1/apb2), enable-bit gating map, LSE/LSI ready timing; fail-inject via `rcc_inject_failure` with HSI fallback (mock `t_rcc`) | N | N | N |
| RNG | 222 | Detailed | CR/SR/DR @0x50060800; LCG regenerated every 40 inst on tick (RNGEN-gated) + on DR read, DRDY, error flags + IRQ; host entropy pool via `rng_seed_entropy` (FIFO, one/regen, SECS reports LCG-fallback — mock `t_entropy`); unseeded = deterministic LCG by design (mock `t_rng`) | Y (80) | Y | N |
| RTC | 384 | Detailed | TR/DR/CR/ISR/PRER/WUTR/CALIBR/ALRMAR/ALRMBR/WPR/SSR/SHIFTR/timestamp/CALR/TAFCR/ALRMASSR/ALRMBSSR + 20 backup regs; BCD time advances via PRER prescaler vs instruction count, alarm A/B matching with don't-care masks, wakeup timer (WUTR reload + WUTE countdown latches WUTF + IRQ 2 — mock `t_rtc_wut_ts`), timestamp capture + overrun (TSF/TSOVF — mock `t_rtc_wut_ts`), tamper-pin physics (TAMP1E/TRG/FLT/PRCH/FREQ, BKPR erase, TAMPTS — mock `t_rtc_tamper_phys`), smooth calibration (CALR CALM/CALP rate — mock `t_rtc_wut_ts`) | Y (41) | N (advances on access) | N |
| SAI | 106 | Detailed | GCR + 2 blocks (CR1/CR2/FRCR/SLOTR/IM/SR/CLRFR/DR); shares the WAV-backed audio path with I2S (block routes DR to audio when I2SMOD is set), SR flags on DR access, masked interrupt | Y (87) | N | N |
| SCB | 195 | Detailed | CPUID (Cortex-M4 r0p1), ICSR set/clear pending (PendSV/SysTick) + pending-vector report, AIRCR VECTKEY + SYSRESETREQ → `request_watchdog_reset()`, VTOR, SCR/CCR/SHPR/SHCSR/CFSR/HFSR/DFSR/MMFAR/BFAR/AFSR/CPACR | indirect (PendSV/SysTick) | N | N |
| SDIO | 364 | Detailed | Full register set + emulated SD card state machine (Idle→Ident→Stby→Tran), canned responses for CMD0/2/3/5/7/8/9/10/13/16/17/18/41/55, RCA matching, ACMD prefix tracking (CMD55 latches APP_CMD; ACMD41 answers OCR-ready only under prefix, ACMD6 latches WIDBUS 1/4/8-bit, ACMD13/51 canned; bare ACMDs illegal — mock `t_sdio_acmd`), DAT1 card IRQ (SDIOIT + IRQ49, mock `t_sdio_acmd`), width-scaled data timing (4-bit beats 1-bit; DTIMER→DTIMEOUT, DCRCFAIL fault, RXOVERR/TXUNDERR on zero-length — mock `t_sdio_timing`), data-transfer simulation (DCOUNT/FIFOCNT, CMD17/18), status flags + ICR/MASK | Y (49) | N | N |
| SPI | 648 | Detailed | CR1/CR2/SR/DR/RXCRC/TXCRC/I2SCFGR/I2SPR; full-duplex 8/16-bit transfers to attached device with CS selection via GPIO, I2S mode audio generation, TXE/RXNE toggling; HW CRC (CRCPR poly, live RXCRCR/TXCRCR, CRCNEXT compare latches CRCERR) + OVR/MODF/FRE/BSY error flags with SR→DR clear (mock `t_spi_crc_err`); **CS edges delivered via GPIO write callbacks** (`register_cs_callbacks`, sw_spi pattern) so attached-device CS deassert is observed immediately; master path runs with MSTR=0 stored (`slave_selected` probe — mock `t_spi_slave`); slave gating live (NSS/SSM+SSI gate, DR preload, harness SCK `spi_slave_clock` — mock `t_spi_slave_gate`) | Y (35/36/51) | N | Y |
| STIR | 38 | Detailed | Software trigger @0xE000EF00 (absent from every SVD, registered explicitly), write-only: low 9 bits pend the target external IRQ (out-of-range ignored, deny-closed); unprivileged writes gated on CCR.USERSETMPEND like ICSR; reads return 0 | pends target IRQ | N | N |
| SW_SPI | 110 | Detailed | Bit-banged SPI via GPIO callbacks (CS/CLK/MOSI/MISO): shift register, 8-bit framing, forwards bytes to attached device; CS edge resets | N | N | Y |
| SYSCFG | 63 | Detailed | MEMRMP/PMC/EXTICR[4]/CMPCR masked storage; CMPCR COMP-ready handshake; PMC MII_RMII_SEL honored by the ETH pin mirrors; EXTICR drives the EXTI line→port map; single shared instance (named field, not a per-SVD slot) so monox + Keil DFP maps all see the same block; MEMRMP has no observable boot-alias effect (flat flash map) | N | N | N |
| SYSTICK | 56 | Detailed | CSR/RVR/CVR/CALIB; enabling programs `nvic.systick_period` → periodic SYSTICK pending from `System::tick`; CVR write resets trigger point | indirect (SYSTICK) | N | N |
| TIM | 681 | Detailed | CR1/CR2/SMCR/DIER/SR/EGR/CCMR1-3/CCER/CNT/PSC/ARR/CCR1-6/RCR/DCR/DMAR/OR; instruction-count counter with PSC prescaler, up/down/center-aligned, UIF+UIE IRQ on overflow, CC match IRQs, UG update event, PWM duty; master/slave trigger routing (CR2 MMS reset/update → ITR slaves SMS reset/gated/trigger, deferred end-of-tick drain); DBGMCU freeze stops advance while halted+frozen; encoder counting live via `tim_encoder_step` (SMS/polarity/wrap — mock `t_tim_enc`) | Y (per-timer) | Y | N |
| USART | 389 | Detailed | SR/DR/BRR/CR1-3/GTPR; TX pushes to global UART output buffer (drained by `get_uart_output`), RX via `rx_byte` (JS `uart_rx_byte`) with 64-byte RX buffer, RXNE/ORE, TXE/TC always set; HW flow control (CTSE gates TX on harness CTS, TXE/TC clear while held) + FE/PE fault injection (PCE-gated PE, DR-read clear) (mock `t_usart_flow_err`); IRQ on RXNEIE/TCIE/TXEIE; LIN break (LBD + RXNE/0x00, LBDIE IRQ — mock `t_usart_protocols`), Smartcard T=0 NACK retry loop (TC held, 8-cap, NE on exhaustion — mock `t_usart_protocols`), IrDA pulse-class envelope (NE on mismatch — mock `t_usart_protocols`); LIN/SC/IrDA enable bits stored + probed (`lin_active`/`sc_active`/`irda_active`), async path unchanged (mock `t_usart_modes`) | Y (37/38/39/52/53/71/82/83) | N | N |
| USB | 1229 | Detailed | OTG FS device @0x50000000 + OTG HS in FS mode @0x40040000 (IRQ 77, HS ENUMSPD, own window — same device core; the SVD OTG_FS_*/OTG_HS_* sub-entries stay dropped): GINTSTS+GINTMSK (USBRST/ENUMDNE/RXFLVL/NPTXFE/IEPINT/OEPINT, W1C), GRXSTSP(pop)/GRXSTSR(peek), device block, EP CTL/INT(XFRC+EPDISD, W1C)/TSIZ/DMA/DTXFSTS-from-programmed-FSIZ; IN completes synchronously at EPENA, OUT on short-packet/drained XFRSIZ, STALL handshake per-EP (CTL bit 21, `usb_in_status`/`usb_out_status`); IRQ67 when masked+live; host-driven via `usb_*` / `usb_hs_*` exports (`usbhost.js` `{hs:true}` in-page, `test_usb.mjs` + matrix `usb` script; HS e2e = full CDC enum + 2x echo + STALL on the HS block); SOF 1 kHz while enumerated (W1C, FNSOF — mock `t_usb_sof`), HS 8x microframes + EOPF + `usb_hs_uframe` (mock `t_hs_uframe`), ULPI rate report 480/12 in GUSBCFG + scope probes (mock `t_ulpi_rate`), VBUS sense via `usb_set_vbus` (BSVLD/SEDET/SUSPSTS — mock `t_usb_vbus`), internal-DMA accounting via DMAEN + `usb_dma_progress` (mock `t_usb_dma`); no isochronous/host (HPTXFSIZ=0), no suspend-resume IRQ path beyond RWUSIG (mock `t_usb_gaps`) | Y (67/77) | Y | Y |
| WWDG | 158 | Detailed | CR/CFR/SR; countdown at 256×prescaler instructions, early-wakeup flag + IRQ (EWI), underflow + WDGA → `request_watchdog_reset()`; DBGMCU freeze stops countdown while halted+frozen | Y (0) | Y | N |

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
  delivery only ever takes enabled IRQs.
- USART ignores its ext-device argument (uses the global UART buffer).
- DMA peripheral-side accesses are chunked in Rust (`dma_periph_read`/
  `dma_periph_write` — one WASM call per transfer instead of size/4);
  mem-to-mem copies complete inline in the Rust core (guest RAM is
  model-visible), peripheral transfers stage for the JS driver.
- FLASH programs/erases emulated flash; DCMI consumes JS-fed camera frames
  (no pixel source). LTDC scanout, SAI/I2S WAV-backed audio, and CAN
  two-node bus arbitration are implemented (see rows above).
- Timers/ADC/RNG/RTC/IWDG/WWDG are instruction-count driven, not
  wall-clock driven (deterministic across machines).
- USB STALL handshake: IN + OUT endpoints stall on CTL bit 21 (set latches
  the handshake, clear resumes; stalled transfers move no data and raise no
  XFRC), observable via `usb_in_status`/`usb_out_status` (2 = STALL) —
  covered by the native `stall_handshake_set_and_clear_both_directions`
  test AND end-to-end: `usb_cdc_test` stalls EP0 on GET_DESCRIPTOR string 9
  (`USB stall set` → host samples status 2 → `USB stall clear` → status 0),
  asserted in `site/test_usb.mjs` and mirrored in the browser `site/usbhost.js`
  script (`takeStall`/`waitClear` steps).
