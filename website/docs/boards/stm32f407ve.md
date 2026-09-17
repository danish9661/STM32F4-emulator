---
title: Board STM32F407VE/ZE
---

# STM32F407VE/ZE — support notes

Black F407VE / F407ZE boards: same die as the VG (identical silicon and
peripherals), 512K flash package, 192K RAM. Everything on
[STM32F407](stm32f407.md) applies; only flash size and board wiring differ.

## Board wiring (emulator rig)

- Black F407VE — LED **PA6**, `Serial` = **USART1**
- Black F407ZE — LED **PF10**, `Serial` = **USART1**

## Demos on this chip (66 presets)

The 64 shared F407 presets (see [STM32F407](stm32f407.md)) that fit 512K,
plus the VE/ZE builds: `blinky_f407ve`, `blinky_f407ze`,
`arduino_black_f407ve`, `arduino_black_f407ze`,
`arduino_test_black_f407ve`, `arduino_test_black_f407ze`,
`crypto_test_black_f407ve`, `crypto_test_black_f407ze`,
`echo_test_black_f407ve`, `echo_test_black_f407ze`,
`edge_test_black_f407ve`, `edge_test_black_f407ze`,
`hal_test_black_f407ve`, `hal_test_black_f407ze`,
`periph_test_black_f407ve`, `periph_test_black_f407ze`,
`timer_test_black_f407ve`, `timer_test_black_f407ze`. (Full list:
`adc_demo`, `arduino_test`, `audio_play_test`, `audio_test`,
`blink_serial`, `blinky`, `buzzer_test`, `can_demo`, `can_host_rx`,
`can_test`, `comprehensive_test`, `crypto_deep_test`, `crypto_test`,
`dac_demo`, `dcmi_test`, `deep_periph_test`, `deep_sleep_demo`,
`echo_test`, `edge_test`, `eth_irq_test`, `exti_test`, `flash_test`,
`fpu_irq_test`, `fpu_test`, `freertos_test`, `fsmc_test`, `hal_test`,
`i2s_sai_test`, `ltdc_test`, `mpu_test`, `new_periph_test`, `oled_test`,
`periph_test`, `pwm_demo`, `qspi_test`, `rtc_test`, `rx_crypto_test`,
`rx_interrupt_test`, `spi_flash_test`, `spi_tft_test`, `test_firmware`,
`tft_test`, `tim_capture_demo`, `timer_test`, `usb_cdc_test`,
`watchdog_demo`, `wwdg_demo`, `wwdg_window_demo`.)

## Verification

- `node site/test_arduino_boards.mjs` — VE + ZE builds PASS
- `node site/test_board_matrix.mjs` — all 66 entries green

## Feature matrix (silicon vs emulator)

Same die as the VG, so this table is row-for-row the [F407
table](stm32f407.md) — reproduced here so the page stands alone. Same
statuses: **Full** = proven by a passing preset; **Partial** = named gap;
**Missing** = silicon has it, the model doesn't; **Absent** = no such
silicon. Depth details: [PERIPHERALS](../peripherals.md); Ethernet depth:
[NETWORKING](../networking.md).

| Peripheral | Silicon | Status | Modeled | NOT modeled |
|---|---|---|---|---|
| CPU Cortex-M4F Thumb-2 | Yes | Full | GAS-verified decoder, exact exception entry/return, faults | M0+ is a separate project; UNDEFINSTR/BKPT halt loudly by policy |
| VFPv4-SP FPU | Yes | Full | arithmetic, conversions, single-rounding fused MLA, lazy stacking | — |
| MPU | Yes | Full | region/AP/XN/subregion enforcement, MemManage faults with MMFSR/MMFAR | — |
| NVIC + STIR | Yes | Full | priority grouping, BASEPRI/FAULTMASK, preemption, tail-chaining | — |
| SysTick | Yes | Full | LOAD/VAL/CTRL, periodic IRQ | CALIB reads 0 (no calibration data, by design) |
| SCB | Yes | Full | CPUID, VTOR, AIRCR, SHCSR, CFSR/HFSR, MMFAR/BFAR, CPACR | FPB/ETM/TPIU unmapped-benign (no debugger attached) |
| DWT | Yes | Full | CYCCNT clock, EXCCNT, FOLDCNT | CPI counter (would be fake precision in an interpreter) |
| ITM | Yes | Full | all 32 STIMn ports live (TER-gated): port 0 → UART console, ports 1-31 queue per-port streams (`itm_take_port`/`itm_port_pending`) | no SWO trace sink (bytes stay in-model for the driver) |
| GPIOA, B, C, D, E, F, G, H, I | Yes | Full | MODER/ODR/IDR/BSRR/AFR, EXTI edge path, ETH pin mirrors | — |
| EXTI | Yes | Full | IMR/RTSR/FTSR/PR, lines → IRQ 6–10/23/40 | — |
| SYSCFG | Yes | Full | MEMRMP/PMC/EXTICR/CMPCR storage; PMC RMII/MII select honored by ETH pin mirrors; EXTICR drives the EXTI line→port map; CMPCR COMP-ready handshake | reserved-bit behavior only (masks documented) |
| PWR | Yes | Full | CR/CSR storage, WUF/SBF on wake (STOP/STANDBY via emulator sleep path), PVD live (PVDO vs PVDE), VOS settle window, F429 overdrive/under-drive handshake (ODEN→ODRDY/ODSWRDY/UDRDY), STOP regulator state live (LPDS/FPDS hold VOSRDY 0 until wake+settle — mock `t_pwr`) | analog rail pinned healthy (PVDO=0 whenever PVDE set) |
| RCC | Yes | Full | HSE/PLL ready delays, real SYS/AHB/APB freq math, CSR reset-cause bits | fail-inject via `rcc_inject_failure` with HSI fallback (mock `t_rcc`); sources lock by default |
| FLASH | Yes | Full | unlock sequence, program + sector erase via the driver | WRP/RDP option bits stored; error flags WRPERR/PGSERR/PGAERR + OPTLOCK/OPTSTRT (mock `t_flash_err`) |
| DMA1, DMA2 | Yes | Full | 8 streams each, FIFO, double-buffer, mem-to-mem inline | peripheral side staged through the JS driver |
| USART1, USART2, USART3, USART6 | Yes | Full | async TX/RX, 64 B RX FIFO, RXNE/ORE, TXE/TC, IRQs | LIN/SC/IrDA enable bits stored + probed, async path unchanged (mock `t_usart_modes`); HW flow control + FE/PE live (mock `t_usart_flow_err`) |
| UART4, UART5 | Yes | Full | same USART model | LIN/SC/IrDA stored-only (mock `t_usart_modes`) |
| SPI1, SPI2, SPI3 + I2S1–3 | Yes | Full | master 8/16-bit, CS callbacks, flash/tap slaves, WAV-backed audio + TX capture | slave-select stored + probed, master path runs (mock `t_spi_slave`); HW CRC live + OVR/MODF/FRE/BSY (mock `t_spi_crc_err`) |
| I2C1, I2C2, I2C3 | Yes | Full | master state machine, EEPROM/regfile/tap slaves, IRQs | GCALL/ALERT/ARP answers + host-notify (mock `t_smbus_addr`), PEC + ARLO + TIMEOUT (mock `t_i2c_smbus`); single-master otherwise (mock `t_i2c_multi`) |
| SDIO | Yes | Full | card state machine, CMD0/2/3/5/7/8/9/10/13/16/17/18/41/55, block reads, IRQ49 | ACMD + wide-bus + DAT1 IRQ live (mock `t_sdio_acmd`) |
| TIM1, TIM8 (advanced) | Yes | Full | up/down/center, OC/PWM, input capture + host injection, DBGMCU freeze, TRGO trigger routing (MMS reset/update → ITR slaves), UIF/CC IRQs | encoder counting live via `tim_encoder_step` (SMS/polarity/wrap — mock `t_tim_enc`) |
| TIM2–TIM5 (general-purpose) | Yes | Full | same model (32-bit TIM2/5) + TRGO routing | encoder live (mock `t_tim_enc`); else same gaps minus trigger routing |
| TIM6, TIM7 (basic) | Yes | Full | time-base + UIF IRQ, DBGMCU freeze, TRGO routing | — |
| TIM9–TIM14 | Yes | Full | time-base + capture/compare + IRQs | — |
| ADC1, ADC2, ADC3 | Yes | Full | SWSTART conversion, SMPR timing, EOC/OVR/AWD IRQs, ADC_Common CSR/CDR mirror, EOC-triggered DMA staging (`adc_take_dma`), temp/Vref/Vbat canned | LCG samples (deterministic); dual-simultaneous latch via CDR + `adc_dual_latched` probe (mock `t_adc_dual`) |
| DAC | Yes | Full | triggers, LFSR noise + triangle waveforms, DOR readback, sink-sample probe `sink_sample` (mock `t_dac_sink`) | DOR readback IS the sink value (no analog pin layer) |
| DCMI | Yes | Full | JS-fed frames, 4-deep FIFO, VSYNC/LINE/FRAME/ERR/OVR + IRQ78, MIS masked status, CROP window, JPEG word-pack mode (+ESS), pin-sync harness gating (VSYNC-arm `dcmi_set_sync`, HSYNC blanking hold, PCLK divider — mock `t_dcmi`) | sensor pixel values always come from the JS feed (no analog sensor) |
| FSMC | Yes | Full | 4 banks, BCR/BTR/BWTR + NAND PCR/SR/PMEM/PATT/ECCR with real ECC engine (order-sensitive 24-bit parity, ECCEN-rise reset), JS bank taps (address + value), wait-state timing live (DATAST+ADDSET stamp `busy_until`, SR BUSY bit 5 — mock `t_fsmc`), NAND backing array via `fsmc_bind_nand` (program 1→0, erase restores, mock `t_fsmc`) | vendor Hamming matrix proprietary (codes differ bit-for-bit — contract, not bits); unwired+unbound banks read 0 |
| ETH MAC/MMC/PTP/DMA + PHY | Yes | Full | PHY/MDIO, TX csum insert, RX csum status, hash + perfect filters, VLAN, PTP timebase/drift/target/snapshots, WOL magic + 4 filters, wire pacing, deferral/collision reports, real LwIP sockets | PPS pin live on PB5 IDR mirror (level = counter path, mock `t_pps`); nibble data + 25/50 MHz clocks electrical-only, level mirrors are the contract (mock `t_nibble`); ULPI rate report 480/12 in GUSBCFG + scope probes (mock `t_ulpi_rate`); addend is rate-only |
| MII/RMII pin mirrors | Pins | Full | TX_EN / CRS_DV / RXD / COL levels readable in IDR | nibble data + 25/50 MHz clocks stay electrical-only |
| USB OTG FS (device) | Yes | Full | EP0–3 control + bulk, byte-exact echo, IN+OUT STALL handshake (CTL bit 21, `usb_out_status`), IRQ67 | no isochronous/host (HPTXFSIZ=0); SOF 1 kHz + HS 8x microframes/EOPF, ULPI rate 480/12, VBUS sense, internal-DMA accounting |
| USB OTG HS (FS mode) | Yes | Full | same device core as FS at 0x40040000, IRQ 77, HS ENUMSPD on ENUMDNE, own `usb_hs_*` host API; e2e = full CDC enum + 2x echo + STALL on the HS block | ULPI rate report 480 (mock `t_ulpi_rate`); microframes + EOPF (mock `t_hs_uframe`); host mode |
| CAN1, CAN2 | Yes | Full | two-node arbitration (lowest ID wins), filters, host injection, IRQs | TEC/REC live + bus-off/EPVF/EWGF/LEC via `can_note_error` (mock `t_can_err`); CAN FD + FDCAN timing NBTP/DBTP (mock `t_canfd`/`t_fdcan_timing`) |
| RTC + backup registers | Yes | Full | BCD time, alarms A/B, wakeup exits STOP, temperature pair | WUT + timestamp + tamper + smooth cal live (mock `t_rtc_wut_ts`) |
| CRC | Yes | Full | real CRC-32 | — |
| RNG | Yes | Full | DRDY + IRQ, regen-on-tick (RNGEN-gated) | host entropy pool + SECS fallback (mock `t_entropy`); unseeded LCG deterministic by design |
| CRYP | Yes | Full | AES-128/192/256 ECB/CBC/CTR, DES/3DES, GCM, CCM via real crates | — |
| HASH | Yes | Full | SHA-1 / MD5 / SHA-256 via real crates | HMAC live (MODE/LKEY, RFC 4231 mock `t_hmac_sha1`) |
| IWDG, WWDG | Yes | Full | prescalers, window semantics, early-wakeup IRQ, expiry reboots the guest | — |
| DBGMCU | Yes | Full | IDCODE 0x10006411; APBx freeze honored — halted+frozen TIM2-14/WWDG/IWDG neither count nor fire, I2C START/STOP sequencing holds (SMBUS bits 21-23); `dbgmcu_set_halt`, no catch-up burst | no debugger attached, so halt is a test/driver state |
| DMA2D | No | Absent | model exists — run these demos on F429 | — |
| LTDC | No | Absent | model exists — run on F429 | — |
| SAI1 | No | Absent | model exists — run on F429 | — |
| GPIOK | No | Absent | model exists — run on F429 | — |
| GPIOJ | No | Absent | — | — |
| QSPI | No | Absent | model exists — run on F429 | — |
| SPI4, SPI5, SPI6 | No | Absent | shared SPI model unclaimed on this map | — |
| UART7, UART8 | No | Absent | shared USART model unclaimed on this map | — |
