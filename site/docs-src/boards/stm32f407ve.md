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
| ITM | Yes | Partial | port-0 stimulus → UART console when TCR.ITMENA + TER[0] | other stimulus ports, no trace sink |
| GPIOA, B, C, D, E, F, G, H, I | Yes | Full | MODER/ODR/IDR/BSRR/AFR, EXTI edge path, ETH pin mirrors | — |
| EXTI | Yes | Full | IMR/RTSR/FTSR/PR, lines → IRQ 6–10/23/40 | — |
| SYSCFG | Yes | Partial | MEMRMP/PMC/EXTICR/CMPCR storage; PMC RMII/MII select honored | nothing beyond stored registers |
| PWR | Yes | Partial | CR/CSR storage, WUF on wake; STOP sleep handled at emulator level | voltage scaling / regulator states |
| RCC | Yes | Full | HSE/PLL ready delays, real SYS/AHB/APB freq math, CSR reset-cause bits | clock-failure injection (sources always lock) |
| FLASH | Yes | Full | unlock sequence, program + sector erase via the driver | WRP/RDP option bits stored, not enforced |
| DMA1, DMA2 | Yes | Full | 8 streams each, FIFO, double-buffer, mem-to-mem inline | peripheral side staged through the JS driver |
| USART1, USART2, USART3, USART6 | Yes | Full | async TX/RX, 64 B RX FIFO, RXNE/ORE, TXE/TC, IRQs | LIN, Smartcard, IrDA, HW flow control |
| UART4, UART5 | Yes | Full | same USART model | same gaps |
| SPI1, SPI2, SPI3 + I2S1–3 | Yes | Full | master 8/16-bit, CS callbacks, flash/tap slaves, WAV-backed audio + TX capture | SPI slave mode; HW CRC regs stored, not computed |
| I2C1, I2C2, I2C3 | Yes | Full | master state machine, EEPROM/regfile/tap slaves, IRQs | multi-master arbitration; SMBus/PEC |
| SDIO | Yes | Full | card state machine, CMD0/2/3/5/7/8/9/10/13/16/17/18/41/55, block reads, IRQ49 | bus-width switching; ACMDs; SDIO-card interrupts |
| TIM1, TIM8 (advanced) | Yes | Full | up/down/center, OC/PWM, input capture + host injection, UIF/CC IRQs | encoder-mode counting; timer-to-timer trigger routing |
| TIM2–TIM5 (general-purpose) | Yes | Full | same model (32-bit TIM2/5) | same gaps |
| TIM6, TIM7 (basic) | Yes | Full | time-base + UIF IRQ | — |
| TIM9–TIM14 | Yes | Full | time-base + capture/compare + IRQs | — |
| ADC1, ADC2, ADC3 | Yes | Full | SWSTART conversion, SMPR timing, EOC/OVR IRQs, temp/Vref/Vbat canned | sample values are deterministic LCG pseudo-random; AWD thresholds stored without IRQ; no DMA requests |
| DAC | Yes | Full | triggers, LFSR noise + triangle waveforms, DOR readback | output has no physical sink (register model) |
| DCMI | Yes | Partial | JS-fed frames, 4-deep FIFO, VSYNC/LINE/FRAME/OVR + IRQ78 | pin-sync sampling; JPEG mode; crop-window regs stored |
| FSMC | Yes | Partial | 4 banks, BCR/BTR/PCR, JS bank taps (address + value) | access timings / wait states; NAND ECC not computed; untapped banks read 0 |
| ETH MAC/MMC/PTP/DMA + PHY | Yes | Full | PHY/MDIO, TX csum insert, RX csum status, hash + perfect filters, VLAN, PTP timebase/drift/target/snapshots, WOL magic + 4 filters, wire pacing, deferral/collision reports, real LwIP sockets | PPS pin itself (the edge counter is the sink); addend is rate-only |
| MII/RMII pin mirrors | Pins | Full | TX_EN / CRS_DV / RXD / COL levels readable in IDR | nibble data + 25/50 MHz clocks stay electrical-only |
| USB OTG FS (device) | Yes | Full | EP0–3 control + bulk, byte-exact echo, IRQ67 | isochronous unproven; SOF/suspend; VBUS sensing; internal DMA; host mode |
| USB OTG HS | Yes | Missing | SVD entries dropped (benign 0) | the whole HS controller — use FS |
| CAN1, CAN2 | Yes | Full | two-node arbitration (lowest ID wins), filters, host injection, IRQs | bus-off / error-passive states simplified; CAN FD n/a |
| RTC + backup registers | Yes | Full | BCD time, alarms A/B, wakeup exits STOP, temperature pair | tamper pins; smoothing calibration not applied (CALIBR stored) |
| CRC | Yes | Full | real CRC-32 | — |
| RNG | Yes | Full | DRDY + IRQ | LCG pseudo-random (deterministic, not true entropy) |
| CRYP | Yes | Full | AES-128/192/256 ECB/CBC/CTR, DES/3DES, GCM, CCM via real crates | — |
| HASH | Yes | Full | SHA-1 / MD5 / SHA-256 via real crates | HMAC mode |
| IWDG, WWDG | Yes | Full | prescalers, window semantics, early-wakeup IRQ, expiry reboots the guest | — |
| DBGMCU | Yes | Partial | IDCODE 0x10006411 | APBx freeze bits stored but timers ignore them |
| DMA2D | No | Absent | model exists — run these demos on F429 | — |
| LTDC | No | Absent | model exists — run on F429 | — |
| SAI1 | No | Absent | model exists — run on F429 | — |
| GPIOK | No | Absent | model exists — run on F429 | — |
| GPIOJ | No | Absent | — | — |
| QSPI | No | Absent | model exists — run on F429 | — |
| SPI4, SPI5, SPI6 | No | Absent | shared SPI model unclaimed on this map | — |
| UART7, UART8 | No | Absent | shared USART model unclaimed on this map | — |
