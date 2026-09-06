use std::sync::atomic::{AtomicPtr, Ordering};
use wasm_bindgen::prelude::*;

mod system;
pub mod peripherals;
pub mod ext_devices;
pub mod cpu;

use system::WasmSystem;

// The process-wide system instance. This used to be a `OnceLock`, which
// accepts only the FIRST value: every later `init()` was silently discarded
// and a second emulator instance in the same process kept running on the
// FIRST instance's entire peripheral tree. That was the root cause of the
// old "one firmware per process" rule.
//
// It is now an AtomicPtr to a leaked Box, so `init()`/`init_svd()` can
// replace it while `sys()` still hands out a `&'static`. The previous
// WasmSystem is deliberately LEAKED rather than dropped: `sys()` has already
// handed out `&'static` references that may still be live on the stack (an
// in-flight MMIO callback re-entering from the CPU core), so freeing the old
// one would dangle. A system is a few hundred KB and instances are created
// once per firmware, so the leak is bounded by firmware count.
//
// An earlier attempt at this same change appeared to regress test_exti and
// test_audio. The real cause was on the JS side: site/emulator.js called
// `init_svd(svd)` and then `init()`, which is a no-op under OnceLock but
// clobbers the SVD-built system with the hardcoded map once SYS is
// replaceable. That call is now conditional; those two tests are the
// canaries for this code.
static SYS: AtomicPtr<WasmSystem> = AtomicPtr::new(std::ptr::null_mut());

pub(crate) fn sys() -> &'static WasmSystem {
    let p = SYS.load(Ordering::Acquire);
    assert!(!p.is_null(), "WasmSystem not initialized");
    // SAFETY: p came from Box::into_raw in set_sys and is never freed.
    unsafe { &*p }
}

/// Install a fresh system, leaking the previous one (see SYS above).
fn set_sys(s: WasmSystem) {
    SYS.store(Box::into_raw(Box::new(s)), Ordering::Release);
}

#[cfg(test)]
pub(crate) fn init_svd_for_test(s: WasmSystem) {
    set_sys(s);
}

/// Initialize the emulator with hardcoded peripheral map.
/// Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
#[wasm_bindgen]
pub fn init() {
    console_error_panic_hook::set_once();
    set_sys(WasmSystem::new());
}

/// Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
/// Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
#[wasm_bindgen]
pub fn init_svd(svd_xml: &str) {
    console_error_panic_hook::set_once();
    set_sys(WasmSystem::new_svd(svd_xml));
}

#[wasm_bindgen]
pub fn periph_read(addr: u32, width: u32) -> u32 {
    sys().p.read(&*sys(), addr, width as u8)
}

#[wasm_bindgen]
pub fn periph_write(addr: u32, width: u32, value: u32) {
    sys().p.write(&*sys(), addr, width as u8, value);
}

#[wasm_bindgen]
pub fn tick() {
    use std::sync::atomic::Ordering;
    system::INSTRUCTION_COUNT.fetch_add(1, Ordering::Relaxed);
    sys().tick();
}

/// Same as tick() but accounts for `delta` instructions at once. Timer
/// peripherals are instruction-count driven, so batching ticks with a
/// delta is semantically identical to one tick per instruction.
#[wasm_bindgen]
pub fn tick_n(delta: u32) {
    use std::sync::atomic::Ordering;
    system::INSTRUCTION_COUNT.fetch_add(delta as u64, Ordering::Relaxed);
    sys().tick();
}

/// Check if any interrupt is pending (non-consuming).
#[wasm_bindgen]
pub fn has_pending_interrupt() -> bool {
    sys().p.nvic.borrow().has_pending()
}

#[wasm_bindgen]
pub fn get_next_pending_interrupt() -> i32 {
    sys().p.nvic.borrow_mut().get_and_clear_next_intr_pending()
        .unwrap_or(-255)
}

/// Mark the PWR peripheral as having woken from a low-power (WFI/WFE) state.
/// The emulator calls this when the core resumes after a sleep halt so firmware
/// can read PWR->CSR WUF to confirm the wakeup source.
#[wasm_bindgen]
pub fn pwr_wakeup() {
    sys().p.pwr_wakeup();
}

/// Inject a CAN frame from an external transmitter onto the shared bus. The
/// frame is delivered to every CAN node (CAN1/CAN2) whose accept filters pass
/// it, so the guest sees it exactly as if another node sent it. `data` is up
/// to 8 bytes; `dlc` caps the length. Standard 11-bit frames.
#[wasm_bindgen]
pub fn can_inject(id: u32, dlc: u32, data: &[u8]) {
    crate::peripherals::can::can_inject(sys(), id, dlc, data, false, false);
}

/// Host/JS-driven TIM input-capture edge. Simulate a TIx edge on timer `name`
/// channel `ch` and latch the live counter into its capture register (only if
/// the channel is configured for input capture via CCxS). Mirrors
/// `can_inject`: tests have no external signal source, so the edge is injected
/// from the driver. `name` is e.g. "TIM3"; `ch` is 0..3.
#[wasm_bindgen]
pub fn tim_inject_capture(name: String, ch: u32) {
    crate::peripherals::tim::tim_inject_capture(sys(), &name, ch);
}

/// Set a pending interrupt in the NVIC. Negative `irq` values select system
/// exceptions (SVC = -5, PENDSV = -2, SYSTICK = -1) and are always deliverable.
/// Used by the FreeRTOS path in the JS driver, which detects `svc` in the CPU
/// hook and synthesizes the SVC exception here instead of letting Unicorn take
/// it natively (this WASM build cannot perform the Cortex-M exception return).
#[wasm_bindgen]
pub fn set_intr_pending(irq: i32) {
    sys().p.nvic.borrow_mut().set_intr_pending(irq);
}

#[wasm_bindgen]
pub fn dma_get_pending_count() -> u32 {
    sys().pending_dma_count() as u32
}

#[wasm_bindgen]
pub fn dma_get_pending(index: u32) -> Vec<u32> {
    sys().take_pending_dma_transfer(index as usize)
        .map(|t| t.to_u32_vec())
        .unwrap_or_default()
}

#[wasm_bindgen]
pub fn dma_set_completed(stream_idx: u32, success: bool) {
    sys().mark_dma_completed(stream_idx as usize, success);
}

/// DMA peripheral-side chunked read: read `size` bytes from peripheral
/// `addr` (4-byte-aligned, tail chunk partial). Replaces the JS per-chunk
/// periph_read loop (one WASM call instead of size/4).
#[wasm_bindgen]
pub fn dma_periph_read(addr: u32, size: u32, pinc: bool, psize: u32) -> Vec<u8> {
    // Tell streaming peripherals these reads are the DMA engine, not a CPU
    // polling loop — DCMI serves them from the sensor rather than the 4-deep
    // FIFO, which is what stops a DMA capture from overrunning.
    system::set_dma_read_active(true);
    let out = dma_periph_read_inner(addr, size, pinc, psize);
    system::set_dma_read_active(false);
    out
}

fn dma_periph_read_inner(addr: u32, size: u32, pinc: bool, psize: u32) -> Vec<u8> {
    let psize = psize.max(1);
    let mut out = Vec::with_capacity(size as usize);
    let mut off = 0u32;
    while off < size {
        let chunk = std::cmp::min(psize, size - off);
        // PINC (peripheral increment) walks addresses; otherwise the same
        // register (e.g. an I2S/SAI data register FIFO) is read repeatedly.
        // Chunks follow PSIZE so 16-bit data registers yield contiguous
        // sample streams instead of zero-padded 4-byte groups.
        let ra = if pinc { addr + off } else { addr };
        let val = sys().p.read(&*sys(), ra, chunk as u8);
        for k in 0..chunk {
            out.push((val >> (k * 8)) as u8);
        }
        off += chunk;
    }
    out
}

/// DMA peripheral-side chunked write: write `bytes` to peripheral `addr` in
/// 4-byte chunks (tail chunk partial). Replaces the JS per-chunk periph_write
/// loop (one WASM call instead of size/4).
#[wasm_bindgen]
pub fn dma_periph_write(addr: u32, bytes: Vec<u8>) {
    let mut j = 0usize;
    while j < bytes.len() {
        let chunk = std::cmp::min(4, bytes.len() - j);
        let mut val = 0u32;
        for k in 0..chunk {
            val |= (bytes[j + k] as u32) << (k * 8);
        }
        sys().p.write(&*sys(), addr + j as u32, chunk as u8, val);
        j += chunk;
    }
}

/// Clear all process-lifetime globals so a NEW emulator instance starts
/// clean.  Must be called before registering that instance's devices.
/// Without it, `ExtDevices` accumulates and a second instance silently binds
/// to the FIRST instance's devices (see system::reset_globals).
#[wasm_bindgen]
pub fn reset_state() {
    system::reset_globals();
}

#[wasm_bindgen]
pub fn gpio_read_output(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow().read_output_pin(port as u8, pin as u8)
}

#[wasm_bindgen]
pub fn gpio_set_input(port: u32, pin: u32, value: bool) {
    sys().p.gpio.borrow_mut().set_input_pin(port as u8, pin as u8, value);
}

#[wasm_bindgen]
pub fn gpio_read_input(port: u32, pin: u32) -> bool {
    sys().p.gpio.borrow().read_input_pin(port as u8, pin as u8)
}

/// Force an ADC channel's next conversion(s) to return `value` (clamped to
/// 12-bit) instead of the synthetic temp/vref/vbat/random default. Unlike
/// spi_tap/i2c_register_slave this can be called any time, including after
/// init() — it's a global override table, not per-instance device wiring
/// (see docs/components.md).
#[wasm_bindgen]
pub fn adc_set_channel_value(peripheral: &str, channel: u32, value: u32) {
    system::adc_set_override(peripheral, channel, value);
}

/// Remove a channel override, reverting it to the synthetic default.
#[wasm_bindgen]
pub fn adc_clear_channel_value(peripheral: &str, channel: u32) {
    system::adc_clear_override(peripheral, channel);
}

#[wasm_bindgen]
pub fn is_watchdog_reset_requested() -> bool {
    system::is_watchdog_reset_requested()
}

#[wasm_bindgen]
pub fn iwdg_reset_flag() -> bool {
    system::iwdg_reset_flag()
}

#[wasm_bindgen]
pub fn wwdg_reset_flag() -> bool {
    system::wwdg_reset_flag()
}

#[wasm_bindgen]
pub fn clear_watchdog_reset_flags() {
    system::clear_watchdog_reset_flags()
}

/// Inject a received byte into the UART at the given peripheral base address.
/// Returns true if a peripheral was found at that address.
#[wasm_bindgen]
pub fn uart_rx_byte(addr: u32, byte: u8) -> bool {
    sys().p.rx_byte(&*sys(), addr, byte)
}

/// Load a WAV file (PCM 16-bit) as the I2S/SAI sample source. DR reads then
/// consume samples from it. Returns an error string on malformed input.
#[wasm_bindgen]
pub fn audio_load_wav(bytes: Vec<u8>) -> Result<(), String> {
    system::audio_load_wav(&bytes)
}

/// Drain the I2S/SAI TX capture FIFO (all DR writes since the last call).
#[wasm_bindgen]
pub fn audio_take_capture() -> Vec<u16> {
    system::audio_take_capture()
}

/// Remaining source samples (0 when no WAV is loaded or it is exhausted).
#[wasm_bindgen]
pub fn audio_source_remaining() -> u32 {
    system::audio_source_remaining()
}

/// Reset the audio source and capture FIFO.
#[wasm_bindgen]
pub fn audio_clear() {
    system::audio_clear();
}

/// Current LTDC scanline (0xFFFF when the controller is disabled).
#[wasm_bindgen]
pub fn ltdc_get_scanline() -> u32 {
    let p = sys().p.clone();
    for slot in &p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(l) = b.as_any_mut().downcast_mut::<peripherals::ltdc::Ltdc>() {
            return l.scanline();
        }
    }
    0xFFFF
}

/// Frames completed by the LTDC scanout since enable.
#[wasm_bindgen]
pub fn ltdc_get_frame_count() -> u32 {
    let p = sys().p.clone();
    for slot in &p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(l) = b.as_any_mut().downcast_mut::<peripherals::ltdc::Ltdc>() {
            return l.frame_count();
        }
    }
    0
}

/// Check if an Ethernet TX poll is pending (firmware wants to send a packet).
#[wasm_bindgen]
pub fn eth_is_tx_poll() -> bool { system::eth_is_tx_poll() }

/// Get the TX descriptor list address for the current poll.
#[wasm_bindgen]
pub fn eth_get_tx_desc_addr() -> u32 { system::eth_get_tx_desc_addr() }

/// Clear the TX poll flag (call after processing descriptors).
#[wasm_bindgen]
pub fn eth_clear_tx_poll() { system::eth_clear_tx_poll(); }

/// Check if an Ethernet RX poll is pending (firmware wants to receive a packet).
#[wasm_bindgen]
pub fn eth_is_rx_poll() -> bool { system::eth_is_rx_poll() }

/// Get the RX descriptor list address for the current poll.
#[wasm_bindgen]
pub fn eth_get_rx_desc_addr() -> u32 { system::eth_get_rx_desc_addr() }

/// Clear the RX poll flag (call after processing descriptors).
#[wasm_bindgen]
pub fn eth_clear_rx_poll() { system::eth_clear_rx_poll(); }

/// Signal to the peripheral that TX descriptor processing is complete.
/// Call this after walking TX descriptors and sending the packet.
#[wasm_bindgen]
pub fn eth_tx_done() { system::eth_set_done(1); }

/// Signal to the peripheral that RX descriptor processing is complete.
/// Call this after writing received data into RX buffers.
#[wasm_bindgen]
pub fn eth_rx_done() { system::eth_set_done(2); }

/// Re-arm the RX poll flag from JS (used when more packets are pending in gwRxQueue).
#[wasm_bindgen]
pub fn eth_signal_rx_poll(desc_addr: u32) { system::eth_signal_rx_poll(desc_addr); }

/// Re-arm the TX poll flag from JS (used when more TX descriptors are pending).
#[wasm_bindgen]
pub fn eth_signal_tx_poll(desc_addr: u32) { system::eth_signal_tx_poll(desc_addr); }

/// True when the FLASH peripheral is unlocked with PG set and !BSY — the
/// JS driver applies program writes to guest memory when this is true.
#[wasm_bindgen]
pub fn flash_is_programming() -> bool { system::flash_is_programming() }

/// Consume a completed erase request (start, len) the JS driver must apply
/// to guest memory (all bytes 0xFF). Empty vec = nothing pending.
#[wasm_bindgen]
pub fn flash_take_erase() -> Vec<u32> {
    match system::take_flash_erase() {
        Some((start, len)) => vec![start, len],
        None => Vec::new(),
    }
}

/// Called by the JS driver after it applied the queued erase to guest memory;
/// clears BSY/EOP so the firmware's busy-wait can proceed.
#[wasm_bindgen]
pub fn flash_erase_applied() {
    sys().flash_erase_applied();
}

/// Collect UART output since last call.
#[wasm_bindgen]
pub fn get_uart_output() -> String {
    use std::mem::take;
    take(&mut *system::get_uart_output().lock().unwrap())
}

/// Add an SPI flash device. Must be called before init().
#[wasm_bindgen]
pub fn add_spi_flash(peripheral: &str, jedec_id: u32, data: &[u8], cs: Option<String>) {
    use crate::ext_devices::spi_flash::{SpiFlash, SpiFlashConfig};
    let config = SpiFlashConfig {
        peripheral: peripheral.to_string(),
        jedec_id,
        content: data.to_vec(),
        size: data.len(),
        cs,
    };
    let flash = SpiFlash::new(config);
    system::get_ext_devices().lock().unwrap().spi_flashes
        .push(std::rc::Rc::new(std::cell::RefCell::new(flash)));
}

/// Debug: flash state summary [wel, status1, cs_state, dummy_pending, pending_program_len]
#[wasm_bindgen]
pub fn spi_flash_debug(peripheral: &str) -> Vec<u32> {
    use crate::ext_devices::SpiFlash;
    let mut out = vec![];
    for f in system::get_ext_devices().lock().unwrap().spi_flashes.iter() {
        let f = f.borrow();
        if f.config.peripheral == peripheral {
            out.push(if f.wel { 1 } else { 0 });
            out.push(f.status1 as u32);
            out.push(if f.cs_state { 1 } else { 0 });
            out.push(f.dummy_pending as u32);
            out.push(f.pending_program.as_ref().map(|p| p.data.len() as u32).unwrap_or(0));
            out.push(f.reply.as_ref().map(|_| 1u32).unwrap_or(0));
        }
    }
    out
}
#[wasm_bindgen]
pub fn add_i2c_eeprom(peripheral: &str, address: u8, data: &[u8]) {
    use crate::ext_devices::i2c_eeprom::{I2cEeprom, I2cEepromConfig};
    let config = I2cEepromConfig {
        peripheral: peripheral.to_string(),
        address,
        content: data.to_vec(),
        size: data.len(),
    };
    let eeprom = I2cEeprom::new(config);
    system::get_ext_devices().lock().unwrap().i2c_eeproms
        .push(std::rc::Rc::new(std::cell::RefCell::new(eeprom)));
}

/// Register a software SPI device. Must be called before init().
#[wasm_bindgen]
pub fn add_software_spi(name: &str, cs: Option<String>, clk: &str, miso: &str, mosi: &str) {
    system::get_software_spi_configs().lock().unwrap()
        .push((name.to_string(), cs, clk.to_string(), miso.to_string(), mosi.to_string()));
}

// ── SPI bus taps (JS hardware layer) ───────────────────────────────────────
// The wasm models the chip only: the SPI controller shifts bytes on the
// bus, and a tap makes every byte/CS event visible to the JS hardware
// layer, which implements the actual device protocol (TFT, sensor, ...).
// Byte/CS event word: bit31 = CS edge, bit30 = asserted, bits7..0 = byte.

/// Register a protocol-agnostic tap on an SPI peripheral. Must be called
/// before init(). `cs` optionally names the GPIO pin used as chip select
/// ("PA4"); when given, CS edges are reported in the event stream. `dc`
/// optionally names a data/command pin; its level is reported in bit 29 of
/// each byte event (1 = data) so the JS device can parse TFT-style traffic.
#[wasm_bindgen]
pub fn spi_tap(peripheral: &str, cs: Option<String>, dc: Option<String>) {
    use crate::ext_devices::spi_tap::{SpiTap, SpiTapConfig};
    let config = SpiTapConfig {
        peripheral: peripheral.to_string(),
        cs,
        dc,
    };
    system::get_ext_devices().lock().unwrap().spi_taps
        .push(std::rc::Rc::new(std::cell::RefCell::new(SpiTap::new(config))));
}

/// Drain all SPI tap events for a peripheral since the last call.
#[wasm_bindgen]
pub fn spi_take_events(peripheral: &str) -> Vec<u32> {
    system::spi_tap_take_events(peripheral)
}

/// Push bytes the JS device answers on the MISO line (read transactions).
#[wasm_bindgen]
pub fn spi_push_miso(peripheral: &str, bytes: &[u8]) {
    system::spi_tap_miso_push(peripheral, bytes);
}

// ── I2C bus taps (JS hardware layer) ───────────────────────────────────────

/// Register a protocol-agnostic I2C slave on a peripheral. Must be called
/// before init(). The address is ACKed like any other registered slave;
/// master writes queue for JS (`i2c_take_tx`) and JS-pushed bytes are
/// returned on master reads (`i2c_push_rx`).
#[wasm_bindgen]
pub fn i2c_register_slave(peripheral: &str, address: u8) {
    use crate::ext_devices::i2c_tap::{I2cTap, I2cTapConfig};
    let config = I2cTapConfig {
        peripheral: peripheral.to_string(),
        address,
    };
    system::get_ext_devices().lock().unwrap().i2c_taps
        .push(std::rc::Rc::new(std::cell::RefCell::new(I2cTap::new(config))));
}

/// Drain all events for a tapped I2C slave since the last call. Each entry
/// is a u32: bit31 = START/STOP boundary event (bit30 = 1 START / 0 STOP),
/// otherwise the low byte is one byte the master wrote to the slave.
#[wasm_bindgen]
pub fn i2c_take_events(peripheral: &str) -> Vec<u32> {
    system::i2c_tap_take_tx(peripheral)
}

/// Queue bytes the tapped I2C slave answers on master reads.
#[wasm_bindgen]
pub fn i2c_push_rx(peripheral: &str, bytes: &[u8]) {
    system::i2c_tap_rx_push(peripheral, bytes);
}

// ── I2C register-file devices (DS3231 RTC) ─────────────────────────────────

/// Register a pointer-addressed register file (DS3231 RTC style) on an I2C
/// peripheral. Must be called before init(). The first write byte of each
/// transaction is the register pointer, subsequent bytes land at `ptr++`;
/// reads return `regs[ptr++]` (pointer persists across address matches).
#[wasm_bindgen]
pub fn i2c_register_regfile(peripheral: &str, address: u8, size: usize, init: &[u8]) {
    use crate::ext_devices::i2c_regfile::{I2cRegFile, I2cRegFileConfig};
    let config = I2cRegFileConfig {
        peripheral: peripheral.to_string(),
        address,
        size,
        init: init.to_vec(),
    };
    system::get_ext_devices().lock().unwrap().i2c_regfiles
        .push(std::rc::Rc::new(std::cell::RefCell::new(I2cRegFile::new(config))));
}

/// Read one register of the first matching regfile on a peripheral.
#[wasm_bindgen]
pub fn i2c_regfile_get(peripheral: &str, offset: usize) -> u8 {
    system::get_ext_devices().lock().unwrap().i2c_regfiles.iter()
        .find(|d| d.borrow().config.peripheral == peripheral)
        .map(|d| d.borrow().get(offset))
        .unwrap_or(0)
}

/// Write one register of the first matching regfile on a peripheral
/// (JS-side poke, e.g. temperature coming from outside the guest).
#[wasm_bindgen]
pub fn i2c_regfile_set(peripheral: &str, offset: usize, value: u8) {
    if let Some(d) = system::get_ext_devices().lock().unwrap().i2c_regfiles.iter()
        .find(|d| d.borrow().config.peripheral == peripheral)
    {
        d.borrow_mut().set(offset, value);
    }
}

// ── QSPI external flash (JS-provided image) ───────────────────────────────

/// Register an external QSPI flash image for the named QUADSPI peripheral.
/// Must be called before init(): the model binds its flash backend once at
/// construction and never rescans. `data` is the raw flash contents (e.g. a
/// W25Q-style image); indirect read/write transfers are serviced from it.
#[wasm_bindgen]
pub fn qspi_register_flash(name: &str, data: &[u8]) {
    crate::peripherals::qspi::qspi_register_flash(name, data);
}

// ── FSMC bank taps (JS memory-mapped device) ───────────────────────────────

/// Register a protocol-agnostic tap on an FSMC memory bank (0 = BANK1, the
/// 0x6000_0000 window). Must be called before init(): the Fsmc peripheral
/// binds its banks' devices once at construction and never rescans.
///
/// Every data-space access to the bank is queued for JS as TWO words —
/// header then value — where the header is `1<<31 | offset` for a write and
/// `offset` for a read. The offset matters: memory-mapped displays in
/// 8080 mode decode an address line as RS/DC, so the address is the only
/// thing separating a command write from a pixel write.
#[wasm_bindgen]
pub fn fsmc_tap(bank: u32) {
    use crate::ext_devices::fsmc_tap::{FsmcTap, FsmcTapConfig};
    let config = FsmcTapConfig { bank: bank as usize };
    system::get_ext_devices().lock().unwrap().fsmc_taps
        .push(std::rc::Rc::new(std::cell::RefCell::new(FsmcTap::new(config))));
}

/// Drain all FSMC tap events for a bank since the last call (2 words per
/// access, see `fsmc_tap`).
#[wasm_bindgen]
pub fn fsmc_take_events(bank: u32) -> Vec<u32> {
    system::fsmc_tap_take_events(bank as usize)
}

/// Queue values the JS device answers on subsequent bank reads, oldest
/// first. An exhausted queue reads back 0.
#[wasm_bindgen]
pub fn fsmc_push_data(bank: u32, values: &[u32]) {
    system::fsmc_tap_data_push(bank as usize, values);
}

// ── DCMI frame source (JS camera sensor) ───────────────────────────────────
// The camera sensor is external hardware (JS). This feeds one captured
// frame (8-bit pixels, row-major) into the on-chip DCMI controller, which
// consumes it with real LINE/FRAME/OVR semantics.

/// Provide the next camera frame to the DCMI controller (8-bit pixels,
/// row-major, width x height). The next CAPTURE start consumes it.
#[wasm_bindgen]
pub fn dcmi_feed_frame(w: u32, h: u32, pixels: &[u8]) {
    system::dcmi_feed_frame(w, h, pixels);
}

/// Forget any fed frame (stop the camera).
#[wasm_bindgen]
pub fn dcmi_clear() {
    system::dcmi_clear();
}

use cpu::{Cpu, mem::{FlatMemory, Memory}};
#[wasm_bindgen]
pub struct WasmCpu { cpu: Cpu, mem: FlatMemory }
#[wasm_bindgen]
impl WasmCpu {
    #[wasm_bindgen(constructor)]
    pub fn new(sp: u32, pc: u32, flash_size: u32, ram_size: u32) -> Self { Self { cpu: Cpu::new(sp, pc), mem: FlatMemory::new(flash_size as usize, ram_size as usize) } }
    /// Load firmware bytes (writes through flash protection).
    pub fn load_firmware(&mut self, data: &[u8], base: u32) { self.mem.load(data, base); }
    pub fn read8(&self, addr: u32) -> u8 { self.mem.read8(addr) }
    pub fn write8(&mut self, addr: u32, v: u8) { self.mem.write8(addr, v) }
    pub fn read32(&self, addr: u32) -> u32 { self.mem.read32(addr) }
    pub fn write32(&mut self, addr: u32, v: u32) { self.mem.write32(addr, v) }
    pub fn mem_read(&self, addr: u32, len: u32) -> Vec<u8> {
        (0..len).map(|i| self.mem.read8(addr.wrapping_add(i))).collect()
    }
    pub fn mem_write(&mut self, addr: u32, data: &[u8]) {
        for (i, &b) in data.iter().enumerate() { self.mem.write8(addr.wrapping_add(i as u32), b); }
    }
    pub fn reset_cpu(&mut self, sp: u32, pc: u32) { self.cpu.reset(sp, pc); }
    /// Enable/disable inline guest exception delivery (NVIC SysTick, ETH,
    /// USART RX, SVC, PendSV...). Off by default (polling-only, matches the
    /// Unicorn path where pending model IRQs never stop execution).
    pub fn set_deliver_irqs(&mut self, v: bool) { self.cpu.deliver_irqs = v; }
    /// True while halted in WFI/WFE (low-power). The driver advances virtual
    /// time and calls `wake()` once an interrupt is pending.
    pub fn sleeping(&self) -> bool { self.cpu.sleeping }
    pub fn wake(&mut self) { self.cpu.sleeping = false; }
    pub fn get_ipsr(&self) -> u32 { self.cpu.ipsr }
    pub fn get_pc(&self) -> u32 { self.cpu.regs.r[15] }
    pub fn get_sp(&self) -> u32 { self.cpu.regs.r[13] }
    pub fn get_regs(&self) -> Vec<u32> { self.cpu.regs.r.to_vec() }
    pub fn get_xpsr(&self) -> u32 { self.cpu.regs.xpsr }
    pub fn get_primask(&self) -> u32 { self.cpu.regs.primask }
    /// Fault program counter, or 0xFFFF_FFFF when running clean.
    pub fn fault_pc(&self) -> u32 { self.cpu.fault.map(|f| f.pc).unwrap_or(0xFFFF_FFFF) }
    /// Packed fault detail: op1 | op2<<16 | len<<... (see fault_op2/fault_len).
    pub fn fault_op1(&self) -> u32 { self.cpu.fault.map(|f| f.op1 as u32).unwrap_or(0) }
    pub fn fault_op2(&self) -> u32 { self.cpu.fault.map(|f| f.op2 as u32).unwrap_or(0) }
    pub fn fault_len(&self) -> u32 { self.cpu.fault.map(|f| f.len as u32).unwrap_or(0) }
    /// Last unmapped-memory access address, or 0xFFFF_FFFF when none.
    pub fn mem_fault(&self) -> u32 { self.mem.bad.get().unwrap_or(0xFFFF_FFFF) }
    pub fn step(&mut self, budget: u32) -> u32 { self.cpu.run(sys(), &mut self.mem, budget) }
    /// PC-trace control for differential debugging (see cpu::trace_*).
    pub fn trace_start(&mut self) { cpu::trace_start(); }
    pub fn trace_stop(&mut self) { cpu::trace_stop(); }
    pub fn take_trace(&mut self) -> Vec<u32> { cpu::take_trace() }
}
