/* tslint:disable */
/* eslint-disable */

export class WasmCpu {
    free(): void;
    [Symbol.dispose](): void;
    fault_len(): number;
    /**
     * Packed fault detail: op1 | op2<<16 | len<<... (see fault_op2/fault_len).
     */
    fault_op1(): number;
    fault_op2(): number;
    /**
     * Fault program counter, or 0xFFFF_FFFF when running clean.
     */
    fault_pc(): number;
    /**
     * Fill a flash range with 0xFF (erase applied by the JS flash driver
     * after `flash_take_erase`; clamped to mapped flash, then completed
     * with `flash_erase_applied`). Debugger/firmware images use `load_firmware`.
     */
    flash_fill_erase(start: number, len: number): void;
    /**
     * FPSCR (cumulative flags, RMode, FZ/DN).
     */
    get_fpscr(): number;
    get_ipsr(): number;
    get_pc(): number;
    get_primask(): number;
    get_regs(): Uint32Array;
    get_sp(): number;
    /**
     * Raw S0-S31 file (f32 bits; Dd aliases S(2d)/S(2d+1)). Debugger and
     * driver visibility for the VFPv4-SP unit (nothing else reads these).
     */
    get_sregs(): Uint32Array;
    get_xpsr(): number;
    /**
     * Load firmware bytes (writes through flash protection).
     */
    load_firmware(data: Uint8Array, base: number): void;
    /**
     * Last unmapped-memory access address, or 0xFFFF_FFFF when none.
     */
    mem_fault(): number;
    mem_read(addr: number, len: number): Uint8Array;
    mem_write(addr: number, data: Uint8Array): void;
    constructor(sp: number, pc: number, flash_size: number, ram_size: number);
    read32(addr: number): number;
    read8(addr: number): number;
    reset_cpu(sp: number, pc: number): void;
    /**
     * Enable/disable inline guest exception delivery (NVIC SysTick, ETH,
     * USART RX, SVC, PendSV...). Off by default (polling-only: pending
     * model IRQs never stop execution).
     */
    set_deliver_irqs(v: boolean): void;
    /**
     * Debugger poke for FPSCR, under the same write mask the guest VMSR
     * uses (NZCVQC + AHP/DN/FZ/RMode + enables/flags; STRIDE/LEN/reserved
     * stay zero).
     */
    set_fpscr(v: number): void;
    /**
     * Debugger poke for one S register (out of range is ignored).
     */
    set_sreg(i: number, v: number): void;
    /**
     * True while halted in WFI/WFE (low-power). The driver advances virtual
     * time and calls `wake()` once an interrupt is pending.
     */
    sleeping(): boolean;
    step(budget: number): number;
    take_trace(): Uint32Array;
    /**
     * PC-trace control for differential debugging (see cpu::trace_*).
     */
    trace_start(): void;
    trace_stop(): void;
    wake(): void;
    write32(addr: number, v: number): void;
    write8(addr: number, v: number): void;
}

/**
 * Remove a channel override, reverting it to the synthetic default.
 */
export function adc_clear_channel_value(peripheral: string, channel: number): void;

/**
 * Whether the last ADC CDR read returned a latched simultaneous pair
 * (dual regular-simultaneous mode with both sides conversion-ready).
 * Harness scope probe for dual-mode simultaneity.
 */
export function adc_dual_latched(): boolean;

/**
 * Force an ADC channel's next conversion(s) to return `value` (clamped to
 * 12-bit) instead of the synthetic temp/vref/vbat/random default. Unlike
 * spi_tap/i2c_register_slave this can be called any time, including after
 * init() — it's a global override table, not per-instance device wiring
 * (see docs/components.md).
 */
export function adc_set_channel_value(peripheral: string, channel: number, value: number): void;

/**
 * Drain ADC samples staged by EOC-triggered DMA requests (CR2 DMA bit).
 * Each entry is one 12-bit conversion result, oldest first; empty when no
 * conversion with DMA enabled has completed since the last drain. The JS
 * DMA driver calls this after servicing a DMA stream aimed at an ADC DR.
 */
export function adc_take_dma(): Uint16Array;

export function add_i2c_eeprom(peripheral: string, address: number, data: Uint8Array): void;

/**
 * Register a software SPI device. Must be called before init().
 */
export function add_software_spi(name: string, cs: string | null | undefined, clk: string, miso: string, mosi: string): void;

/**
 * Add an SPI flash device. Must be called before init().
 */
export function add_spi_flash(peripheral: string, jedec_id: number, data: Uint8Array, cs?: string | null): void;

/**
 * Reset the audio source and capture FIFO.
 */
export function audio_clear(): void;

/**
 * Load a WAV file (PCM 16-bit) as the I2S/SAI sample source. DR reads then
 * consume samples from it. Returns an error string on malformed input.
 */
export function audio_load_wav(bytes: Uint8Array): void;

/**
 * Remaining source samples (0 when no WAV is loaded or it is exhausted).
 */
export function audio_source_remaining(): number;

/**
 * Drain the I2S/SAI TX capture FIFO (all DR writes since the last call).
 */
export function audio_take_capture(): Uint16Array;

/**
 * Read one CAN FD payload byte for (`base`, `fifo`, `slot`, `idx`) —
 * the harness path into the FD window (firmware uses the MMIO window).
 */
export function can_fd_byte(base: number, fifo: number, slot: number, idx: number): number;

/**
 * FDCAN wire-time cost (virtual instructions) for a frame: arbitration
 * at the nominal rate, FD payload at the data rate iff BRS. Firmware
 * pacing TX-to-TX gaps observes exactly this contract.
 */
export function can_fd_cost(base: number, fd: boolean, brs: boolean, payload_bytes: number): bigint;

/**
 * Valid CAN FD payload length for (`base`, `fifo`, `slot`).
 */
export function can_fd_len(base: number, fifo: number, slot: number): number;

/**
 * Inject a CAN frame from an external transmitter onto the shared bus. The
 * frame is delivered to every CAN node (CAN1/CAN2) whose accept filters pass
 * it, so the guest sees it exactly as if another node sent it. `data` is up
 * to 8 bytes; `dlc` caps the length. Standard 11-bit frames.
 */
export function can_inject(id: number, dlc: number, data: Uint8Array): void;

/**
 * Inject a CAN FD frame (FDF set, up to 64 bytes) from an external
 * transmitter. Same filter + FIFO path as classic (arbitration on the ID
 * is identical); the payload lands in the FD window (`can_fd_read`),
 * TDTR reports DLC + FDF. `brs` marks bit-rate-switch (flag only).
 */
export function can_inject_fd(id: number, data: Uint8Array, brs: boolean): void;

/**
 * Harness = the wire fault: one error event on the CAN node at `base`
 * (TEC +8, LEC latched 0-7; BOFF at TEC > 255). `recover=true` models
 * 128x11 recessive bits (counters + LEC cleared, bus recovered).
 */
export function can_note_error(base: number, lec: number, recover: boolean): void;

export function clear_watchdog_reset_flags(): void;

/**
 * Forget any fed frame (stop the camera).
 */
export function dcmi_clear(): void;

/**
 * Provide the next camera frame to the DCMI controller (8-bit pixels,
 * row-major, width x height). The next CAPTURE start consumes it.
 */
export function dcmi_feed_frame(w: number, h: number, pixels: Uint8Array): void;

/**
 * Harness = the camera sync lines: drive VSYNC/HSYNC levels and the PCLK
 * divider. A rising VSYNC edge loads an armed capture (CAPTURE set, frame
 * fed); HSYNC low holds pixels (horizontal blanking); PCLK div scales
 * pixels-per-tick (16/div, min 1). Defaults (true, true, 1) = free-run.
 */
export function dcmi_set_sync(vsync: boolean, hsync: boolean, pclk_div: number): void;

/**
 * Blend FG over BG ("over" operator) into the output mode. `fg_alpha`
 * supplies the alpha for formats without one (FGPFCCR.ALPHA).
 */
export function dma2d_blend(fg_cm: number, bg_cm: number, out_cm: number, _w: number, _h: number, fg_alpha: number, fg: Uint8Array, bg: Uint8Array): Uint8Array;

/**
 * Convert a line-packed pixel buffer between color modes
 * (0 ARGB8888, 1 RGB888, 2 RGB565). Pure function of its inputs.
 */
export function dma2d_convert(fg_cm: number, out_cm: number, _w: number, _h: number, px: Uint8Array): Uint8Array;

/**
 * Complete the staged DMA2D transfer (TCIF + IRQ56 when TCIE is set).
 */
export function dma2d_job_done(): void;

/**
 * Take the staged DMA2D transfer for the JS driver: 16 words
 * [mode, w, h, fg_addr, fg_cm, fg_off, bg_addr, bg_cm, bg_off,
 *  out_addr, out_cm, out_off, ocolr, 0, 0, 0], or empty when idle.
 * The driver gathers source lines, converts/blends them, scatters the
 * output lines (honoring the OR line offsets), then calls dma2d_job_done.
 */
export function dma2d_take_job(): Uint32Array;

export function dma_get_pending(index: number): Uint32Array;

export function dma_get_pending_count(): number;

/**
 * DMA peripheral-side chunked read: read `size` bytes from peripheral
 * `addr` (4-byte-aligned, tail chunk partial). Replaces the JS per-chunk
 * periph_read loop (one WASM call instead of size/4).
 */
export function dma_periph_read(addr: number, size: number, pinc: boolean, psize: number): Uint8Array;

/**
 * DMA peripheral-side chunked write: write `bytes` to peripheral `addr` in
 * 4-byte chunks (tail chunk partial). Replaces the JS per-chunk periph_write
 * loop (one WASM call instead of size/4).
 */
export function dma_periph_write(addr: number, bytes: Uint8Array): void;

export function dma_set_completed(stream_idx: number, success: boolean): void;

/**
 * Arm a single-node collision for the next TX completion (consumed once;
 * the driver reports EC + CC=15 when the MAC is half-duplex).
 */
export function eth_arm_collision(): void;

/**
 * Wake-on-LAN inspection of a received frame. Returns bit 0 on a magic
 * packet (latches MPR when MPE is set, pends IRQ 62 when PMTIM is set).
 * Wakeup-frame CRC matching is not modeled (RWKPR never sets).
 */
export function eth_check_wol(frame: Uint8Array): number;

/**
 * Clear the RX poll flag (call after processing descriptors).
 */
export function eth_clear_rx_poll(): void;

/**
 * Clear the TX poll flag (call after processing descriptors).
 */
export function eth_clear_tx_poll(): void;

/**
 * Forward checksum-bad frames (FEF) or drop-disable (DTCEFD); else drop.
 */
export function eth_fwd_csum_bad(): boolean;

/**
 * Current MACCR (FES/DM/LM/ROD checks for pacing + loopback).
 */
export function eth_get_maccr(): number;

/**
 * Get the RX descriptor list address for the current poll.
 */
export function eth_get_rx_desc_addr(): number;

/**
 * Get the TX descriptor list address for the current poll.
 */
export function eth_get_tx_desc_addr(): number;

/**
 * IPCO (MACCR[10]) gates the RX checksum status.
 */
export function eth_ipco_on(): boolean;

/**
 * Check if an Ethernet RX poll is pending (firmware wants to receive a packet).
 */
export function eth_is_rx_poll(): boolean;

/**
 * Check if an Ethernet TX poll is pending (firmware wants to send a packet).
 */
export function eth_is_tx_poll(): boolean;

/**
 * Wire link currently up.
 */
export function eth_link_up(): boolean;

/**
 * Loopback active (MACCR LM). The driver re-injects TX into RX.
 */
export function eth_loopback_tx(): boolean;

/**
 * MAC accept filtering for a received frame (perfect slots + hash table +
 * broadcast/multicast/promiscuous + VLAN tag). The driver drops rejected
 * frames before writing any descriptor.
 */
export function eth_mac_accept(frame: Uint8Array): boolean;

/**
 * TX jabber completion (over the WD limit): TJTS.
 */
export function eth_note_jabber(): void;

/**
 * RX queue-full drop: missed-frame counter + ROS.
 */
export function eth_note_missed(): void;

/**
 * Accepted-RX delivery for the MMC good-unicast counter.
 */
export function eth_note_rx(frame: Uint8Array): void;

/**
 * Delivery deferred on a CPU-owned head (silicon RBUS).
 */
export function eth_note_rx_stall(): void;

/**
 * MMC counting hooks for TX completions / accepted RX / queue-full
 * drops, plus RX-stall (RBUS) and jabber (TJTS) status.
 */
export function eth_note_tx(collided: boolean): void;

/**
 * RX flow-control step: true when the frame is a pause frame for us
 * (arms the stall, terminates the frame — never delivered/counted).
 */
export function eth_pause_rx(frame: Uint8Array): boolean;

/**
 * PPS edge count: the observable sink for the PPS output pin (frequency
 * 2^n Hz from PTPPPSCR, gated by TSE). Test harnesses read this like a
 * scope probe; the guest itself cannot see it, like silicon.
 */
export function eth_pps_count(): number;

/**
 * PPS pin level (square wave at the PTPPPSCR rate, 50% duty). The
 * readable model of the PPS output — sample it like a logic analyzer.
 */
export function eth_pps_level(): boolean;

/**
 * PTP current seconds / subseconds for TDES6/7 + RDES6/7 snapshots.
 */
export function eth_ptp_sec(): number;

export function eth_ptp_sub(): number;

/**
 * PTP timestamping enabled (PTPTSCR TSE). Gates RX/TX snapshots.
 */
export function eth_ptp_tse(): boolean;

/**
 * RX checksum status for descriptor bits: bit 0 = has IPv4, bit 1 = IP
 * header OK, bit 2 = has TCP/UDP/ICMP, bit 3 = L4 OK. Maps to RDES0
 * IPHCE (bit 7) / PCE (bit 0).
 */
export function eth_rx_csum_status(frame: Uint8Array): number;

/**
 * Signal to the peripheral that RX descriptor processing is complete.
 * Call this after writing received data into RX buffers.
 */
export function eth_rx_done(): void;

/**
 * Clear a latched RX stall (delivery succeeded).
 */
export function eth_rx_stall_clear(): void;

/**
 * Arm RX wire pacing for a delivered frame (RS waits the wire time).
 */
export function eth_rx_wire_busy(len: number): void;

/**
 * Set the wire link state (test-harness peer control).
 */
export function eth_set_link(up: boolean): void;

/**
 * Re-arm the RX poll flag from JS (used when more packets are pending in gwRxQueue).
 */
export function eth_signal_rx_poll(desc_addr: number): void;

/**
 * Re-arm the TX poll flag from JS (used when more TX descriptors are pending).
 */
export function eth_signal_tx_poll(desc_addr: number): void;

/**
 * Station address (MACA0) packed as u64 (48 bits used) for SARC insert.
 */
export function eth_station_addr(): bigint;

/**
 * Take a pending armed collision (one-shot, false when none armed).
 */
export function eth_take_collision(): boolean;

/**
 * Take a pending pause-frame emission ((1<<31)|quanta, 0 when none).
 */
export function eth_take_pause_tx(): number;

/**
 * True when a TX completing now must report deferral (half-duplex while
 * a receive still occupies the wire).
 */
export function eth_tx_deferred(): boolean;

/**
 * Signal to the peripheral that TX descriptor processing is complete.
 * Call this after walking TX descriptors and sending the packet.
 */
export function eth_tx_done(): void;

/**
 * Immediate error completion (dead-wire NC / jabber JT): TS raises on
 * the next tick with no wire wait (the driver already wrote the error
 * status into the descriptor).
 */
export function eth_tx_done_now(): void;

/**
 * TX jabber limit from MACCR WD (2048, or 16383 with WD set).
 */
export function eth_tx_jabber_limit(): number;

/**
 * SARC mode (MACCR[29:28]): 0/1 off, 2 insert-if-present, 3 replace.
 */
export function eth_tx_sarc(): number;

/**
 * Arm TX wire pacing for a `len`-byte frame: TS completion waits until the
 * frame has left the wire at the MACCR FES speed (168 MHz virtual clock).
 */
export function eth_tx_wire_busy(len: number): void;

/**
 * Called by the JS driver after it applied the queued erase to guest memory;
 * clears BSY/EOP so the firmware's busy-wait can proceed.
 */
export function flash_erase_applied(): void;

/**
 * True when the FLASH peripheral is unlocked with PG set and !BSY — the
 * JS driver applies program writes to guest memory when this is true.
 */
export function flash_is_programming(): boolean;

/**
 * FLASH readout-protection level from OPTCR RDP (0/1/2).
 */
export function flash_rdp_level(): number;

/**
 * Harness = the option-byte programmer: set the RDP byte (respects
 * OPTLOCK like the register path).
 */
export function flash_set_rdp(level_byte: number): void;

/**
 * Consume a completed erase request (start, len) the JS driver must apply
 * to guest memory (all bytes 0xFF). Empty vec = nothing pending.
 */
export function flash_take_erase(): Uint32Array;

/**
 * Harness = the NAND flash array: bind an erased (0xFF) backing array of
 * `size` bytes to FSMC bank `bank` (0-3). Untapped data accesses go to
 * this array (program clears bits, reads return stored bytes).
 */
export function fsmc_bind_nand(bank: number, size: number): void;

/**
 * Erase `len` bytes at `offset` in FSMC bank `bank` (restore 0xFF) —
 * the silicon block-erase firmware runs before reprogram.
 */
export function fsmc_nand_erase(bank: number, offset: number, len: number): void;

/**
 * Queue values the JS device answers on subsequent bank reads, oldest
 * first. An exhausted queue reads back 0.
 */
export function fsmc_push_data(bank: number, values: Uint32Array): void;

/**
 * Drain all FSMC tap events for a bank since the last call (2 words per
 * access, see `fsmc_tap`).
 */
export function fsmc_take_events(bank: number): Uint32Array;

/**
 * Register a protocol-agnostic tap on an FSMC memory bank (0 = BANK1, the
 * 0x6000_0000 window). Must be called before init(): the Fsmc peripheral
 * binds its banks' devices once at construction and never rescans.
 *
 * Every data-space access to the bank is queued for JS as TWO words —
 * header then value — where the header is `1<<31 | offset` for a write and
 * `offset` for a read. The offset matters: memory-mapped displays in
 * 8080 mode decode an address line as RS/DC, so the address is the only
 * thing separating a command write from a pixel write.
 */
export function fsmc_tap(bank: number): void;

export function get_next_pending_interrupt(): number;

/**
 * Collect UART output since last call.
 */
export function get_uart_output(): string;

export function gpio_read_input(port: number, pin: number): boolean;

export function gpio_read_output(port: number, pin: number): boolean;

export function gpio_set_input(port: number, pin: number, value: boolean): void;

/**
 * Check if any interrupt is pending (non-consuming).
 */
export function has_pending_interrupt(): boolean;

/**
 * Harness = the other master: arm arbitration loss on the next address
 * phase of the I2C block at `base` (one-shot; ARLO latches, bus lost).
 */
export function i2c_arm_arb_loss(base: number): void;

/**
 * Harness = the SMBus alerting device (host-notify source): arm the
 * address returned in DR on the next Alert-Response-Address read.
 */
export function i2c_arm_smbus_alert(base: number, addr: number): void;

/**
 * Current PEC accumulator of the I2C block at `base` (scope probe).
 */
export function i2c_pec(base: number): number;

/**
 * Queue bytes the tapped I2C slave answers on master reads.
 */
export function i2c_push_rx(peripheral: string, bytes: Uint8Array): void;

/**
 * Read one register of the first matching regfile on a peripheral.
 */
export function i2c_regfile_get(peripheral: string, offset: number): number;

/**
 * Write one register of the first matching regfile on a peripheral
 * (JS-side poke, e.g. temperature coming from outside the guest).
 */
export function i2c_regfile_set(peripheral: string, offset: number, value: number): void;

/**
 * Register a pointer-addressed register file (DS3231 RTC style) on an I2C
 * peripheral. Must be called before init(). The first write byte of each
 * transaction is the register pointer, subsequent bytes land at `ptr++`;
 * reads return `regs[ptr++]` (pointer persists across address matches).
 */
export function i2c_register_regfile(peripheral: string, address: number, size: number, init: Uint8Array): void;

/**
 * Register a protocol-agnostic I2C slave on a peripheral. Must be called
 * before init(). The address is ACKed like any other registered slave;
 * master writes queue for JS (`i2c_take_tx`) and JS-pushed bytes are
 * returned on master reads (`i2c_push_rx`).
 */
export function i2c_register_slave(peripheral: string, address: number): void;

/**
 * Drain all events for a tapped I2C slave since the last call. Each entry
 * is a u32: bit31 = START/STOP boundary event (bit30 = 1 START / 0 STOP),
 * otherwise the low byte is one byte the master wrote to the slave.
 */
export function i2c_take_events(peripheral: string): Uint32Array;

/**
 * Initialize the emulator with hardcoded peripheral map.
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init(): void;

/**
 * Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init_svd(svd_xml: string): void;

export function is_watchdog_reset_requested(): boolean;

/**
 * Queued backlog (bytes) for one ITM stimulus port.
 */
export function itm_port_pending(port: number): number;

/**
 * Drain one ITM stimulus port's queued trace bytes (oldest first).
 * Port 0 sinks into the UART console instead and always drains empty here;
 * ports 1-31 queue per-port streams for the JS driver.
 */
export function itm_take_port(port: number): Uint8Array;

export function iwdg_reset_flag(): boolean;

/**
 * Frames completed by the LTDC scanout since enable.
 */
export function ltdc_get_frame_count(): number;

/**
 * Current LTDC scanline (0xFFFF when the controller is disabled).
 */
export function ltdc_get_scanline(): number;

export function periph_read(addr: number, width: number): number;

export function periph_write(addr: number, width: number, value: number): void;

/**
 * Record STANDBY entry (PDDS=1 + SLEEPDEEP WFI): sets SBF (CSR bit 1).
 * The driver calls this when it observes the guest enter WFI sleep with
 * PDDS set, before advancing virtual time.
 */
export function pwr_enter_standby(): void;

/**
 * Mark the PWR peripheral as having woken from a low-power (WFI/WFE) state.
 * The emulator calls this when the core resumes after a sleep halt so firmware
 * can read PWR->CSR WUF to confirm the wakeup source.
 */
export function pwr_wakeup(): void;

/**
 * Wakeup from STANDBY: set WUF (CSR bit 0), keep SBF until the guest
 * clears it via CR CSBF.
 */
export function pwr_wakeup_standby(): void;

/**
 * Register an external QSPI flash image for the named QUADSPI peripheral.
 * Must be called before init(): the model binds its flash backend once at
 * construction and never rescans. `data` is the raw flash contents (e.g. a
 * W25Q-style image); indirect read/write transfers are serviced from it.
 */
export function qspi_register_flash(name: string, data: Uint8Array): void;

/**
 * Harness = the failing oscillator: mark RCC HSE (bit 0) / PLL (bit 1)
 * dead or alive. Dead sources read RDY 0 and SWS falls back to HSI.
 */
export function rcc_inject_failure(src_mask: number, dead: boolean): void;

/**
 * Clear all process-lifetime globals so a NEW emulator instance starts
 * clean.  Must be called before registering that instance's devices.
 * Without it, `ExtDevices` accumulates and a second instance silently binds
 * to the FIRST instance's devices (see system::reset_globals).
 */
export function reset_state(): void;

/**
 * Host entropy words currently pooled (0 = LCG fallback active).
 */
export function rng_entropy_avail(): number;

/**
 * Push host entropy words into the RNG pool (true-noise samples from JS
 * `crypto.getRandomValues` or equivalent). Consumed FIFO, one word per
 * regen; when the pool drains the model falls back to the deterministic
 * LCG (SR SECS reports fallback-active).
 */
export function rng_seed_entropy(words: Uint32Array): void;

/**
 * Harness = the tamper pin: latch RTC TAMP1F (IRQ 2 when TAMPIE).
 */
export function rtc_tamper(): void;

/**
 * Harness = the tamper pin with physics (TAMP1E/TRG/FLT-gated, erases
 * backup registers, optional timestamp via TAMPTS).
 */
export function rtc_tamper_pin(level: boolean): void;

/**
 * Harness = the timestamp pin event: capture TR/DR/SSR, latch TSF.
 */
export function rtc_timestamp(): void;

/**
 * Current SDIO bus-width select (0 = 1-bit, 1 = 4-bit, 2 = 8-bit).
 */
export function sdio_bus_width(): number;

/**
 * Harness = the card's DAT1 interrupt line: latch/clear SDIOIT.
 */
export function sdio_card_irq(set: boolean): void;

/**
 * Harness = the bad card: next CMD17/18 completion latches DCRCFAIL.
 */
export function sdio_fault_data_crc(): void;

/**
 * Set a pending interrupt in the NVIC. Negative `irq` values select system
 * exceptions (SVC = -5, PENDSV = -2, SYSTICK = -1) and are always deliverable.
 * Used by the FreeRTOS path: the Rust core synthesizes these exceptions
 * with exact inline entry/return.
 */
export function set_intr_pending(irq: number): void;

/**
 * Harness = the faulty peer: corrupt the RX CRC so the next CRCNEXT
 * compare on the SPI block at `base` mismatches (latches CRCERR).
 */
export function spi_fault_crc(base: number): void;

/**
 * Harness = the NSS pin fault: latch MODF on the SPI block at `base`.
 */
export function spi_fault_modf(base: number): void;

/**
 * Debug: flash state summary [wel, status1, cs_state, dummy_pending, pending_program_len]
 */
export function spi_flash_debug(peripheral: string): Uint32Array;

/**
 * Push bytes the JS device answers on the MISO line (read transactions).
 */
export function spi_push_miso(peripheral: string, bytes: Uint8Array): void;

/**
 * Harness = the SPI master clock: shift one frame through the slave at
 * `base` (returns the MISO word). No-op when the gate is closed.
 */
export function spi_slave_clock(base: number, mosi: number): number;

/**
 * Slave gate state of the SPI block at `base` (scope probe).
 */
export function spi_slave_gate(base: number): boolean;

/**
 * Harness = the SPI master: drive the slave's NSS level (true = asserted).
 */
export function spi_slave_select(base: number, asserted: boolean): void;

/**
 * Drain all SPI tap events for a peripheral since the last call.
 */
export function spi_take_events(peripheral: string): Uint32Array;

/**
 * Register a protocol-agnostic tap on an SPI peripheral. Must be called
 * before init(). `cs` optionally names the GPIO pin used as chip select
 * ("PA4"); when given, CS edges are reported in the event stream. `dc`
 * optionally names a data/command pin; its level is reported in bit 29 of
 * each byte event (1 = data) so the JS device can parse TFT-style traffic.
 */
export function spi_tap(peripheral: string, cs?: string | null, dc?: string | null): void;

export function tick(): void;

/**
 * Same as tick() but accounts for `delta` instructions at once. Timer
 * peripherals are instruction-count driven, so batching ticks with a
 * delta is semantically identical to one tick per instruction.
 */
export function tick_n(delta: number): void;

/**
 * Run one peripheral-model tick WITHOUT advancing the instruction clock.
 * The CPU core publishes its executed count itself while stepping, so the
 * post-step driver tick must not add the budget a second time.
 */
export function tick_peripherals(): void;

/**
 * Host/JS-driven quadrature step on an encoder-mode timer: one TI edge
 * (`ti` 0 = TI1, 1 = TI2; `rising` = edge polarity). Counts per the
 * SMS/polarity rules; no-op outside encoder modes 1-3.
 */
export function tim_encoder_step(name: string, ti: number, rising: boolean): void;

/**
 * Host/JS-driven TIM input-capture edge. Simulate a TIx edge on timer `name`
 * channel `ch` and latch the live counter into its capture register (only if
 * the channel is configured for input capture via CCxS). Mirrors
 * `can_inject`: tests have no external signal source, so the edge is injected
 * from the driver. `name` is e.g. "TIM3"; `ch` is 0..3.
 */
export function tim_inject_capture(name: string, ch: number): void;

/**
 * Harness = the noisy wire: arm framing (FE) / parity (PE) faults on the
 * next received byte of the USART at `base`. PE needs PCE enabled.
 */
export function uart_fault_rx(base: number, fe: boolean, pe: boolean): void;

/**
 * Harness = the IR transmitter: inject a byte with a pulse class
 * (low_power selects the 1.6µs class, else the 3/16 class).
 */
export function uart_irda_rx(base: number, byte: number, low_power: boolean): void;

/**
 * Pulse class of the last TX byte (scope probe for IrDA mode).
 */
export function uart_irda_tx_class(base: number): number;

/**
 * Harness = the LIN master: deliver a break frame to the USART at `base`.
 */
export function uart_lin_break(base: number): void;

/**
 * Inject a received byte into the UART at the given peripheral base address.
 * Returns true if a peripheral was found at that address.
 */
export function uart_rx_byte(addr: number, byte: number): boolean;

/**
 * Harness = the smartcard: NACK the next transmitted byte.
 */
export function uart_sc_nack(base: number): void;

/**
 * Smartcard retry counter of the USART at `base` (scope probe).
 */
export function uart_sc_retries(base: number): number;

/**
 * Harness = the CTS peer: drive the CTS input of the USART at `base`.
 * With CTSE set, deasserted CTS holds TX (TXE/TC clear, byte dropped).
 */
export function uart_set_cts(base: number, asserted: boolean): void;

/**
 * Queued TX length of the USART at `base` (scope probe for CTSE-hold).
 */
export function uart_tx_len(base: number): number;

/**
 * Internal-DMA progress on the FS block: [in_bytes, out_bytes] moved
 * while GAHBCFG DMAEN was set (firmware polls the EP DMA registers;
 * the harness reads these counters directly).
 */
export function usb_dma_progress(ep: number): BigUint64Array;

/**
 * Simulate enumeration-done at full speed (ENUMDNE + FS speed in DSTS).
 */
export function usb_enumerated(): void;

/**
 * Internal-DMA progress on the HS block.
 */
export function usb_hs_dma_progress(ep: number): BigUint64Array;

/**
 * Simulate enumeration-done on the HS block (ENUMDNE + HS speed in DSTS).
 */
export function usb_hs_enumerated(): void;

/**
 * HS IN transfer status: 0 none, 1 data ready, 2 STALL handshake.
 */
export function usb_hs_in_status(ep: number): number;

/**
 * Inject an OUT data packet to an HS endpoint.
 */
export function usb_hs_inject_out(ep: number, data: Uint8Array): void;

/**
 * Inject an 8-byte SETUP packet to HS EP0.
 */
export function usb_hs_inject_setup(data: Uint8Array): void;

/**
 * HS OUT transfer status: 0 none, 2 STALL handshake.
 */
export function usb_hs_out_status(ep: number): number;

/**
 * USB OTG HS (FS-mode personality) host-side test API: same semantics as
 * the FS exports above, driven against the HS block (0x40040000, IRQ 77).
 * Each falls back to the FS block when no HS slot exists.
 * Simulate a USB bus reset on the HS block.
 */
export function usb_hs_reset(): void;

/**
 * Harness = the cable on the HS block.
 */
export function usb_hs_set_vbus(present: boolean): void;

/**
 * Drain a completed HS device-to-host IN blob.
 */
export function usb_hs_take_in(ep: number): Uint8Array;

/**
 * HS microframe index for the HS block.
 */
export function usb_hs_uframe(): number;

/**
 * ULPI rate for the HS block.
 */
export function usb_hs_ulpi_rate(): number;

/**
 * IN transfer status: 0 none, 1 data ready, 2 STALL handshake.
 */
export function usb_in_status(ep: number): number;

/**
 * Inject an OUT data packet to an endpoint.
 */
export function usb_inject_out(ep: number, data: Uint8Array): void;

/**
 * Inject an 8-byte SETUP packet to EP0.
 */
export function usb_inject_setup(data: Uint8Array): void;

/**
 * OUT transfer status: 0 none, 2 STALL handshake (OUT has no data-ready
 * slot — reception completes via the endpoint interrupt).
 */
export function usb_out_status(ep: number): number;

/**
 * USB OTG FS host-side test API (the harness plays USB host; see
 * peripherals/usb.rs). Drive reset -> enum-done -> SETUP/OUT inject,
 * and drain device-to-host IN blobs with usb_take_in.
 * Simulate a USB bus reset: fresh device session, USBRST latched.
 */
export function usb_reset(): void;

/**
 * Harness = the cable: plug/unplug VBUS on the FS block (default present).
 * Unplug suspends the device, stops SOF, latches SEDET + BSVLD clear.
 */
export function usb_set_vbus(present: boolean): void;

/**
 * Drain a completed device-to-host IN blob (empty = none pending;
 * check usb_in_status first to tell ZLP apart).
 */
export function usb_take_in(ep: number): Uint8Array;

/**
 * HS microframe index (DSTS FNSOF low 3 bits, 0..7) for the FS block
 * (always 0 — FS has 1 ms frames, no microframes).
 */
export function usb_uframe(): number;

/**
 * ULPI PHY packet wire rate in Mbit/s (480 HS / 12 FS) for the FS block.
 */
export function usb_ulpi_rate(): number;

export function wwdg_reset_flag(): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly __wbg_wasmcpu_free: (a: number, b: number) => void;
    readonly adc_clear_channel_value: (a: number, b: number, c: number) => void;
    readonly adc_dual_latched: () => number;
    readonly adc_set_channel_value: (a: number, b: number, c: number, d: number) => void;
    readonly adc_take_dma: (a: number) => void;
    readonly add_i2c_eeprom: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly add_software_spi: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly add_spi_flash: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly audio_clear: () => void;
    readonly audio_load_wav: (a: number, b: number, c: number) => void;
    readonly audio_source_remaining: () => number;
    readonly audio_take_capture: (a: number) => void;
    readonly can_fd_byte: (a: number, b: number, c: number, d: number) => number;
    readonly can_fd_cost: (a: number, b: number, c: number, d: number) => bigint;
    readonly can_fd_len: (a: number, b: number, c: number) => number;
    readonly can_inject: (a: number, b: number, c: number, d: number) => void;
    readonly can_inject_fd: (a: number, b: number, c: number, d: number) => void;
    readonly can_note_error: (a: number, b: number, c: number) => void;
    readonly clear_watchdog_reset_flags: () => void;
    readonly dcmi_clear: () => void;
    readonly dcmi_feed_frame: (a: number, b: number, c: number, d: number) => void;
    readonly dcmi_set_sync: (a: number, b: number, c: number) => void;
    readonly dma2d_blend: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
    readonly dma2d_convert: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly dma2d_job_done: () => void;
    readonly dma2d_take_job: (a: number) => void;
    readonly dma_get_pending: (a: number, b: number) => void;
    readonly dma_get_pending_count: () => number;
    readonly dma_periph_read: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly dma_periph_write: (a: number, b: number, c: number) => void;
    readonly dma_set_completed: (a: number, b: number) => void;
    readonly eth_arm_collision: () => void;
    readonly eth_check_wol: (a: number, b: number) => number;
    readonly eth_clear_rx_poll: () => void;
    readonly eth_clear_tx_poll: () => void;
    readonly eth_fwd_csum_bad: () => number;
    readonly eth_get_maccr: () => number;
    readonly eth_get_rx_desc_addr: () => number;
    readonly eth_get_tx_desc_addr: () => number;
    readonly eth_ipco_on: () => number;
    readonly eth_is_rx_poll: () => number;
    readonly eth_is_tx_poll: () => number;
    readonly eth_link_up: () => number;
    readonly eth_loopback_tx: () => number;
    readonly eth_mac_accept: (a: number, b: number) => number;
    readonly eth_note_jabber: () => void;
    readonly eth_note_missed: () => void;
    readonly eth_note_rx: (a: number, b: number) => void;
    readonly eth_note_rx_stall: () => void;
    readonly eth_note_tx: (a: number) => void;
    readonly eth_pause_rx: (a: number, b: number) => number;
    readonly eth_pps_count: () => number;
    readonly eth_pps_level: () => number;
    readonly eth_ptp_sec: () => number;
    readonly eth_ptp_sub: () => number;
    readonly eth_ptp_tse: () => number;
    readonly eth_rx_csum_status: (a: number, b: number) => number;
    readonly eth_rx_done: () => void;
    readonly eth_rx_stall_clear: () => void;
    readonly eth_rx_wire_busy: (a: number) => void;
    readonly eth_set_link: (a: number) => void;
    readonly eth_signal_rx_poll: (a: number) => void;
    readonly eth_signal_tx_poll: (a: number) => void;
    readonly eth_station_addr: () => bigint;
    readonly eth_take_collision: () => number;
    readonly eth_take_pause_tx: () => number;
    readonly eth_tx_deferred: () => number;
    readonly eth_tx_done: () => void;
    readonly eth_tx_done_now: () => void;
    readonly eth_tx_jabber_limit: () => number;
    readonly eth_tx_sarc: () => number;
    readonly eth_tx_wire_busy: (a: number) => void;
    readonly flash_erase_applied: () => void;
    readonly flash_is_programming: () => number;
    readonly flash_rdp_level: () => number;
    readonly flash_set_rdp: (a: number) => void;
    readonly flash_take_erase: (a: number) => void;
    readonly fsmc_bind_nand: (a: number, b: number) => void;
    readonly fsmc_nand_erase: (a: number, b: number, c: number) => void;
    readonly fsmc_push_data: (a: number, b: number, c: number) => void;
    readonly fsmc_take_events: (a: number, b: number) => void;
    readonly fsmc_tap: (a: number) => void;
    readonly get_next_pending_interrupt: () => number;
    readonly get_uart_output: (a: number) => void;
    readonly gpio_read_input: (a: number, b: number) => number;
    readonly gpio_read_output: (a: number, b: number) => number;
    readonly gpio_set_input: (a: number, b: number, c: number) => void;
    readonly has_pending_interrupt: () => number;
    readonly i2c_arm_arb_loss: (a: number) => void;
    readonly i2c_arm_smbus_alert: (a: number, b: number) => void;
    readonly i2c_pec: (a: number) => number;
    readonly i2c_push_rx: (a: number, b: number, c: number, d: number) => void;
    readonly i2c_regfile_get: (a: number, b: number, c: number) => number;
    readonly i2c_regfile_set: (a: number, b: number, c: number, d: number) => void;
    readonly i2c_register_regfile: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly i2c_register_slave: (a: number, b: number, c: number) => void;
    readonly i2c_take_events: (a: number, b: number, c: number) => void;
    readonly init: () => void;
    readonly init_svd: (a: number, b: number) => void;
    readonly is_watchdog_reset_requested: () => number;
    readonly itm_port_pending: (a: number) => number;
    readonly itm_take_port: (a: number, b: number) => void;
    readonly iwdg_reset_flag: () => number;
    readonly ltdc_get_frame_count: () => number;
    readonly ltdc_get_scanline: () => number;
    readonly periph_read: (a: number, b: number) => number;
    readonly periph_write: (a: number, b: number, c: number) => void;
    readonly pwr_enter_standby: () => void;
    readonly pwr_wakeup: () => void;
    readonly pwr_wakeup_standby: () => void;
    readonly qspi_register_flash: (a: number, b: number, c: number, d: number) => void;
    readonly rcc_inject_failure: (a: number, b: number) => void;
    readonly reset_state: () => void;
    readonly rng_entropy_avail: () => number;
    readonly rng_seed_entropy: (a: number, b: number) => void;
    readonly rtc_tamper: () => void;
    readonly rtc_tamper_pin: (a: number) => void;
    readonly rtc_timestamp: () => void;
    readonly sdio_bus_width: () => number;
    readonly sdio_card_irq: (a: number) => void;
    readonly sdio_fault_data_crc: () => void;
    readonly set_intr_pending: (a: number) => void;
    readonly spi_fault_crc: (a: number) => void;
    readonly spi_fault_modf: (a: number) => void;
    readonly spi_flash_debug: (a: number, b: number, c: number) => void;
    readonly spi_push_miso: (a: number, b: number, c: number, d: number) => void;
    readonly spi_slave_clock: (a: number, b: number) => number;
    readonly spi_slave_gate: (a: number) => number;
    readonly spi_slave_select: (a: number, b: number) => void;
    readonly spi_take_events: (a: number, b: number, c: number) => void;
    readonly spi_tap: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly tick: () => void;
    readonly tick_n: (a: number) => void;
    readonly tick_peripherals: () => void;
    readonly tim_encoder_step: (a: number, b: number, c: number, d: number) => void;
    readonly tim_inject_capture: (a: number, b: number, c: number) => void;
    readonly uart_fault_rx: (a: number, b: number, c: number) => void;
    readonly uart_irda_rx: (a: number, b: number, c: number) => void;
    readonly uart_irda_tx_class: (a: number) => number;
    readonly uart_lin_break: (a: number) => void;
    readonly uart_rx_byte: (a: number, b: number) => number;
    readonly uart_sc_nack: (a: number) => void;
    readonly uart_sc_retries: (a: number) => number;
    readonly uart_set_cts: (a: number, b: number) => void;
    readonly uart_tx_len: (a: number) => number;
    readonly usb_dma_progress: (a: number, b: number) => void;
    readonly usb_enumerated: () => void;
    readonly usb_hs_dma_progress: (a: number, b: number) => void;
    readonly usb_hs_enumerated: () => void;
    readonly usb_hs_in_status: (a: number) => number;
    readonly usb_hs_inject_out: (a: number, b: number, c: number) => void;
    readonly usb_hs_inject_setup: (a: number, b: number) => void;
    readonly usb_hs_out_status: (a: number) => number;
    readonly usb_hs_reset: () => void;
    readonly usb_hs_set_vbus: (a: number) => void;
    readonly usb_hs_take_in: (a: number, b: number) => void;
    readonly usb_hs_uframe: () => number;
    readonly usb_hs_ulpi_rate: () => number;
    readonly usb_in_status: (a: number) => number;
    readonly usb_inject_out: (a: number, b: number, c: number) => void;
    readonly usb_inject_setup: (a: number, b: number) => void;
    readonly usb_out_status: (a: number) => number;
    readonly usb_reset: () => void;
    readonly usb_set_vbus: (a: number) => void;
    readonly usb_take_in: (a: number, b: number) => void;
    readonly usb_uframe: () => number;
    readonly usb_ulpi_rate: () => number;
    readonly wasmcpu_fault_len: (a: number) => number;
    readonly wasmcpu_fault_op1: (a: number) => number;
    readonly wasmcpu_fault_op2: (a: number) => number;
    readonly wasmcpu_fault_pc: (a: number) => number;
    readonly wasmcpu_flash_fill_erase: (a: number, b: number, c: number) => void;
    readonly wasmcpu_get_fpscr: (a: number) => number;
    readonly wasmcpu_get_ipsr: (a: number) => number;
    readonly wasmcpu_get_pc: (a: number) => number;
    readonly wasmcpu_get_primask: (a: number) => number;
    readonly wasmcpu_get_regs: (a: number, b: number) => void;
    readonly wasmcpu_get_sp: (a: number) => number;
    readonly wasmcpu_get_sregs: (a: number, b: number) => void;
    readonly wasmcpu_get_xpsr: (a: number) => number;
    readonly wasmcpu_load_firmware: (a: number, b: number, c: number, d: number) => void;
    readonly wasmcpu_mem_fault: (a: number) => number;
    readonly wasmcpu_mem_read: (a: number, b: number, c: number, d: number) => void;
    readonly wasmcpu_mem_write: (a: number, b: number, c: number, d: number) => void;
    readonly wasmcpu_new: (a: number, b: number, c: number, d: number) => number;
    readonly wasmcpu_read32: (a: number, b: number) => number;
    readonly wasmcpu_read8: (a: number, b: number) => number;
    readonly wasmcpu_reset_cpu: (a: number, b: number, c: number) => void;
    readonly wasmcpu_set_deliver_irqs: (a: number, b: number) => void;
    readonly wasmcpu_set_fpscr: (a: number, b: number) => void;
    readonly wasmcpu_set_sreg: (a: number, b: number, c: number) => void;
    readonly wasmcpu_sleeping: (a: number) => number;
    readonly wasmcpu_step: (a: number, b: number) => number;
    readonly wasmcpu_take_trace: (a: number, b: number) => void;
    readonly wasmcpu_trace_start: (a: number) => void;
    readonly wasmcpu_trace_stop: (a: number) => void;
    readonly wasmcpu_wake: (a: number) => void;
    readonly wasmcpu_write32: (a: number, b: number, c: number) => void;
    readonly wasmcpu_write8: (a: number, b: number, c: number) => void;
    readonly wwdg_reset_flag: () => number;
    readonly __wbindgen_export: (a: number, b: number, c: number) => void;
    readonly __wbindgen_export2: (a: number, b: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
