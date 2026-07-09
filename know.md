$ arduino-cli compile --fqbn "STMicroelectronics:stm32:GenF4:pnum=GENERIC_F407VGTX" "C:\Users\Danish\Documents\stm32-emu\crypto_test" --output-dir "C:\Users\Danish\Documents\stm32-emu\crypto_test\build" 2>&1 && cd "C:\Users\Danish\Documents\stm32-emu\stm32-periph-wasm\pkg" && node cli.mjs "../../crypto_test/build/crypto_test.ino.bin" 20000000 2>&1


$ cd "C:\Users\Danish\Documents\stm32-emu\stm32-periph-wasm\pkg" && node cli.mjs "../../deep_periph_test/build/deep_periph_test.ino.bin" 20000000 2>&1


$ cd "C:\Users\Danish\Documents\stm32-emu\stm32-periph-wasm" && cargo build --release 2>&1 | Select-String "error"
$ cd "C:\Users\Danish\Documents\stm32-emu\stm32-periph-wasm" && wasm-pack build --release 2>&1 | Select-String "error"






















Per-Pheral Detailed Audit
#	Peripheral	File	Lines	Status	Issues Found
1	ADC	adc.rs	151	MINOR_ISSUES	_ => {} silently ignores writes at line 148. No interrupt support (ADC has EOC/OVR interrupts in real HW). DMA not signaled. Uses deterministic pseudo-random values instead of real analog simulation. No temperature sensor calibration. No injected group support beyond basic JDR storage.
2	CAN	can.rs	163	MINOR_ISSUES	_ => {} in inner match arms (lines 116, 138). TX completes immediately (line 119: "complete immediately with success") -- no bus arbitration, error frames, or bit timing simulation. No interrupt signaling from filters or error conditions. Missing IER-based interrupt generation -- interrupt enable bits are stored but never actually trigger NVIC.
3	CRC	crc.rs	39	COMPLETE	Minimal but correct. CRC32 calculation, IDR storage, CR reset. _ => {} catch-all is fine.
4	CRYP	cryp.rs	628	COMPLETE	Most complex and complete peripheral. AES-128/192/256, DES, TDES. ECB, CBC, CTR, GCM, CCM modes. Proper FIFO, interrupt signaling (IRQ 79). GHASH implementation. _ => {} on mode/phase match arms is appropriate. One comment: "simplified: just use IV as nonce" (line 193) for CCM -- minor.
5	DAC	dac.rs	100	MINOR_ISSUES	_ => {} at line 97. No wave generation (WAVE[1:0] bits in CR -- the DAC can generate noise/triangle waves in real HW). No DMA support. update_dor1() uses hardcoded DHR12R1 regardless of alignment mode (line 40: "Just pick the most recently written data register" -- comment acknowledges incompleteness).
6	DBGMCU	dbgmcu.rs	37	MINOR_ISSUES	_ => {} at line 34. Reads hardcoded IDCODE (0x10006411). No debug stub functionality implemented (registers stored but no actual debug behavior).
7	DCMI	dcmi.rs	68	STUB	_ => {} (line 65). Purely synthetic data generation (self.pattern = self.pattern.wrapping_add(0x01020304)). No real camera interface. No interrupt signaling (IER register bits exist but never trigger NVIC). No DMA support. Data register reads generate incrementing pattern -- not realistic.
8	DMA	dma.rs	210	MINOR_ISSUES	_ => {} at lines 69, 190. Interrupt signaled immediately on enable (line 177-178) rather than after transfer completion. Transfer queued to system (sys.queue_dma_transfer) but completion signaling is synchronous/immediate. 8 streams supported (correct for STM32F4).
9	EXTI	exti.rs	45	STUB	_ => {} (line 42). Critical: No interrupt generation. PR bits are stored and SWIER triggers PR, but EXTI never calls set_intr_pending() on NVIC. The entire purpose of EXTI is to generate interrupts -- this is non-functional.
10	FLASH	flash.rs	80	COMPLETE	_ => {} at line 77. Key lock/unlock sequence implemented. ACR, SR, CR, OPTCR registers modeled. No actual flash memory emulation (reads from flash address space go through system memory, not this peripheral).
11	FSMC	fsmc.rs	111	COMPLETE	_ => {} at line 106. Routes to external devices. 4 banks supported. Register file is minimal (BCR/BTR only). No NAND/PC Card support (only NOR/PSRAM).
12	GPIO	gpio.rs	229	COMPLETE	_ => {} at line 226. MODER, OTYPER, OSPEEDR, PUPDR, IDR, ODR, BSRR, LCKR, AFR. Callback system for external devices. Ports A-I (9 ports instead of real A-K=11). No GPIO interrupt support (EXTI integration is in SYSCFG/EXTI). Bit set/reset via BSRR.
13	HASH	hash.rs	164	COMPLETE	_ => {} at lines 92, 161. SHA-1, MD5, SHA-256 support with proper digest computation. Interrupt signaling implemented (IRQ 80). DINIE interrupt. Context registers (CSR) stored.
14	I2C	i2c.rs	161	MINOR_ISSUES	_ => {} at lines 155, 158. Master mode only (no slave). No multi-master arbitration. No clock stretching. State machine implemented (Idle -> StartSent -> AddrSent -> Active). Interrupts: SB flag set but no NVIC firing.
15	I2S extended	i2s.rs	49	STUB	_ => {} (line 46). Minimal implementation -- same register layout as SPI. ready_toggle alternates SR value 0/3. No actual audio streaming. No interrupt generation.
16	IWDG	iwdg.rs	70	COMPLETE	_ => {} at lines 62, 67. Key-protected register access (0x5555 for PR/RLR, 0xAAAA refresh, 0xCCCC enable). Counter decrements based on instruction count. Watchdog reset triggers request_watchdog_reset().
17	LTDC	ltdc.rs	123	MINOR_ISSUES	_ => {} at lines 105, 120. Register model complete (SSCR, BPCR, AWCR, TWCR, GCR, SRCR, BCCR, IER, ISR, LIPCR + 2 layer configs). Interrupt status register stored (ISR) but interrupts never fired to NVIC. CPSR/CDSR return fixed values.
18	NVIC	nvic.rs	218	COMPLETE	_ => {} at line 207. Full NVIC implementation: ISER/ICER/ISPR/ICPR/IABR/PRI registers. System exceptions (PendSV, SysTick). pending tracking via 128-bit bitmap.
19	PWR	pwr.rs	40	STUB	_ => {} at line 37. Only CR stored (with mask). CSR write is no-op. No power state simulation (sleep/stop/standby). No voltage regulator scaling. No PVD.
20	RCC	rcc.rs	220	COMPLETE	_ => {} at line 217. Full clock tree: CR, PLLCFGR, CFGR, CIR, AHB/APB enable/reset/lpenr. HSE/PLL/LSE/LSI ready timing simulation. system_clock_hz(), ahb_freq(), apb1_freq(), apb2_freq() all functional. is_peripheral_enabled() works.
21	RNG	rng.rs	78	MINOR_ISSUES	_ => {} at line 75. Deterministic pseudo-random (LSFR-based). SR handling correct (DRDY, SEIS, CEIS). Missing interrupt support -- IE bit in CR is stored but never triggers NVIC interrupt.
22	RTC	rtc.rs	103	MINOR_ISSUES	_ => {} at line 100. Time counting works (BCD increment). Alarm registers stored but no alarm comparison or interrupt. No wakeup timer. Key registers unprotected (WPR stored but no write protection -- real RTC must write 0xCA then 0x53 to unlock). unsafe { std::mem::zeroed() } for Rtc::default().
23	SAI	sai.rs	77	STUB	_ => {} at lines 35, 74. Two blocks (A/B) modeled with CR1/CR2/FRCR/SLOTR/IM/SR/CLRFR/DR. No audio data generation. SR returns hardcoded 0x08. Interrupts never fired.
24	SCB	scb.rs	130	COMPLETE	_ => {} at line 127. VTOR, ICSR (with pend/clear for PendSV and SysTick), AIRCR (with SYSRESETREQ), SCR, CCR, SHPR, SHCSR, CFSR, HFSR, DFSR, MMFAR, BFAR, AFSR, CPACR. CPACR for FPU.
25	SDIO	sdio.rs	129	MINOR_ISSUES	_ => {} at lines 96, 126. SD state machine (Idle->Ready->Ident->Stby->Tran). Response for CM0,2,3,5,7,8,9,10,13,16,17,18,41,55. Hardcoded responses, no real SD protocol. Data transfer counts bytes but no real data movement. Interrupts not fired via NVIC.
26	SPI	spi.rs	101	COMPLETE	_ => {} at line 98. Full-duplex with external device routing. CS-based device selection via GPIO callbacks. SR emulation with ready_toggle.
27	Software SPI	sw_spi.rs	105	COMPLETE	N/A (not a Peripheral -- uses GPIO callbacks). Bit-banged SPI via external pin callbacks. Proper clock-edge sampling.
28	SYSCFG	syscfg.rs	52	MINOR_ISSUES	_ => {} at line 49. MEMRMP (boot remap), PMC, EXTICR[0-3] stored. COMP AC current mode read flag. No real memory remap effect.
29	SysTick	systick.rs	55	COMPLETE	_ => {} at line 52. CSR/RVR/CVR. Period-based interrupt via NVIC systick_period and maybe_set_systick_intr_pending(). Proper ENABLE+COUNTFLAG handling.
30	TIM	tim.rs	239	COMPLETE	_ => {} at line 236. Up/down/center-aligned counting. Prescaler. Update events with UIF interrupt. Output compare with CCxIF interrupts. PWM duty cycle calc. DMAR/DCR. Supports TIM1-14. IRQ mapping.
31	USART	usart.rs	107	MINOR_ISSUES	_ => {} at line 104. TX via get_uart_output(). Interrupt handling (TCIE, TXEIE, RXNEIE). No RX input mechanism (no way to inject received data into DR). No baud rate effect on timing. No modem/CTS/RTS flow control.
32	WWDG	wwdg.rs	81	COMPLETE	_ => {} at line 78. Counter decrement, early wakeup interrupt (EWI, IRQ 0), reset on underflow. Prescaler (WDGTB). Proper refresh logic.
3. SVD Peripherals vs. Implemented Peripherals
Peripherals listed in the SVD (stm32f407.svd) that are NOT implemented as Peripheral types:

SVD Peripheral	Notes
ADC_Common	Common ADC registers (not handled separately)
Ethernet_MAC	NOT IMPLEMENTED
Ethernet_MMC	NOT IMPLEMENTED
Ethernet_PTP	NOT IMPLEMENTED
Ethernet_DMA	NOT IMPLEMENTED
OTG_FS_GLOBAL	NOT IMPLEMENTED
OTG_FS_HOST	NOT IMPLEMENTED
OTG_FS_DEVICE	NOT IMPLEMENTED
OTG_FS_PWRCLK	NOT IMPLEMENTED
OTG_HS_GLOBAL	NOT IMPLEMENTED
OTG_HS_HOST	NOT IMPLEMENTED
OTG_HS_DEVICE	NOT IMPLEMENTED
OTG_HS_PWRCLK	NOT IMPLEMENTED
CAN1	Implemented (CAN2 also, but not in SVD)
FPU	Partially handled via SCB CPACR
MPU	NOT IMPLEMENTED (not present on F407 per SVD)
NVIC_STIR	NOT IMPLEMENTED
FPU_CPACR	Partially handled via SCB
SCB_ACTRL	NOT IMPLEMENTED