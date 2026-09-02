use std::sync::atomic::{AtomicU64, AtomicBool, AtomicI32, AtomicU8, AtomicU32, Ordering};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Mutex;
use crate::peripherals::{Peripherals, gpio::GpioPorts};
use crate::ext_devices::ExtDevices;

// UART output buffer: USART write_dr pushes chars here, JS reads via get_uart_output()
use std::sync::OnceLock;
static UART_OUTPUT: OnceLock<Mutex<String>> = OnceLock::new();
pub fn get_uart_output() -> &'static Mutex<String> {
    UART_OUTPUT.get_or_init(|| Mutex::new(String::new()))
}

// Global ExtDevices: populated by JS add_* calls before init
static EXT_DEVICES: OnceLock<Mutex<ExtDevices>> = OnceLock::new();
pub fn get_ext_devices() -> &'static Mutex<ExtDevices> {
    EXT_DEVICES.get_or_init(|| Mutex::new(ExtDevices::default()))
}

pub static INSTRUCTION_COUNT: AtomicU64 = AtomicU64::new(0);
pub fn instruction_count() -> u64 { INSTRUCTION_COUNT.load(Ordering::Relaxed) }

static WATCHDOG_RESET_EVENT: AtomicBool = AtomicBool::new(false);
// Persistent reset-cause bits, latched on watchdog expiry until the firmware
// clears them via RCC->CSR RMVF. Bit 29 (IWDGRSTF) / bit 30 (WWDGRSTF).
static IWDG_RESET_FLAG: AtomicBool = AtomicBool::new(false);
static WWDG_RESET_FLAG: AtomicBool = AtomicBool::new(false);

// Software SPI configs queued before init, registered after GPIO exists
static SOFTWARE_SPI_CONFIGS: OnceLock<Mutex<Vec<(String, Option<String>, String, String, String)>>> = OnceLock::new();
pub fn get_software_spi_configs() -> &'static Mutex<Vec<(String, Option<String>, String, String, String)>> {
    SOFTWARE_SPI_CONFIGS.get_or_init(|| Mutex::new(Vec::new()))
}
pub fn is_watchdog_reset_requested() -> bool { WATCHDOG_RESET_EVENT.swap(false, Ordering::Acquire) }
/// Latch a watchdog reset event. cause: bit0 = IWDG, bit1 = WWDG (so a single
/// call can set both if needed). The event flag is consumed by the JS driver
/// (is_watchdog_reset_requested); the per-source flag persists for RCC->CSR.
pub fn request_watchdog_reset(cause: u8) {
    WATCHDOG_RESET_EVENT.store(true, Ordering::Release);
    if cause & 1 != 0 { IWDG_RESET_FLAG.store(true, Ordering::Release); }
    if cause & 2 != 0 { WWDG_RESET_FLAG.store(true, Ordering::Release); }
}
pub fn iwdg_reset_flag() -> bool { IWDG_RESET_FLAG.load(Ordering::Acquire) }
pub fn wwdg_reset_flag() -> bool { WWDG_RESET_FLAG.load(Ordering::Acquire) }
/// Clear the latched watchdog reset-cause bits (RCC->CSR RMVF write).
pub fn clear_watchdog_reset_flags() {
    IWDG_RESET_FLAG.store(false, Ordering::Release);
    WWDG_RESET_FLAG.store(false, Ordering::Release);
}

// Ethernet MAC event flags
static ETH_TX_POLL: AtomicBool = AtomicBool::new(false);
static ETH_RX_POLL: AtomicBool = AtomicBool::new(false);
// 0=none, 1=TX done, 2=RX done, 3=both. Set by JS after descriptor processing.
static ETH_DONE: AtomicU8 = AtomicU8::new(0);
// TX/RX descriptor addresses captured when poll demand is written
static ETH_TX_DESC_ADDR: AtomicU32 = AtomicU32::new(0);
static ETH_RX_DESC_ADDR: AtomicU32 = AtomicU32::new(0);

pub fn eth_signal_tx_poll(desc_addr: u32) { ETH_TX_POLL.store(true, Ordering::Release); ETH_TX_DESC_ADDR.store(desc_addr, Ordering::Release); }
pub fn eth_signal_rx_poll(desc_addr: u32) { ETH_RX_POLL.store(true, Ordering::Release); ETH_RX_DESC_ADDR.store(desc_addr, Ordering::Release); }
pub fn eth_is_tx_poll() -> bool { ETH_TX_POLL.load(Ordering::Acquire) }
pub fn eth_clear_tx_poll() { ETH_TX_POLL.store(false, Ordering::Release); }
pub fn eth_is_rx_poll() -> bool { ETH_RX_POLL.load(Ordering::Acquire) }
pub fn eth_clear_rx_poll() { ETH_RX_POLL.store(false, Ordering::Release); }
pub fn eth_get_tx_desc_addr() -> u32 { ETH_TX_DESC_ADDR.load(Ordering::Acquire) }
pub fn eth_get_rx_desc_addr() -> u32 { ETH_RX_DESC_ADDR.load(Ordering::Acquire) }
pub fn eth_set_done(flags: u8) { ETH_DONE.fetch_or(flags, Ordering::Release); }
pub fn eth_take_done() -> u8 { ETH_DONE.swap(0, Ordering::Acquire) }

// FLASH programming/erase state shared with the JS driver (which applies the
// actual memory mutations to guest memory).
static FLASH_PROGRAMMING: AtomicBool = AtomicBool::new(false);
static FLASH_ERASE: Mutex<Option<(u32, u32)>> = Mutex::new(None);

pub fn set_flash_programming(v: bool) { FLASH_PROGRAMMING.store(v, Ordering::Release); }
pub fn flash_is_programming() -> bool { FLASH_PROGRAMMING.load(Ordering::Acquire) }
pub fn queue_flash_erase(start: u32, len: u32) {
    *FLASH_ERASE.lock().unwrap() = Some((start, len));
}
pub fn take_flash_erase() -> Option<(u32, u32)> {
    FLASH_ERASE.lock().unwrap().take()
}

impl WasmSystem {
    pub fn flash_erase_applied(&self) {
        self.p.flash_erase_applied();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DmaDir { Read, Write, MemCopy }

#[derive(Debug, Clone)]
pub struct DmaTransfer {
    pub direction: DmaDir,
    pub stream_idx: usize,
    pub dma_name: String,
    pub src: u32,
    pub dst: u32,
    pub size: usize,
    pub peri_addr: u32,
    pub peripheral: bool,
    pub pinc: bool, // PINC: increment the peripheral address per transfer
    pub p_size: usize, // peripheral data width in bytes (PSIZE)
}

impl DmaTransfer {
    pub fn to_u32_vec(&self) -> Vec<u32> {
        vec![
            self.direction as u32,
            self.stream_idx as u32,
            self.src,
            self.dst,
            self.size as u32,
            self.peri_addr,
            self.peripheral as u32,
            self.pinc as u32,
            self.p_size as u32,
        ]
    }
}

static DMA_COMPLETED: [AtomicBool; 8] = [
    AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false),
    AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false), AtomicBool::new(false),
];

// Per-stream DMA interrupt info: IRQ number (-1 = none) and flags (bit 0=TCIE, 1=HTIE, 2=TEIE)
static DMA_STREAM_IRQ: [AtomicI32; 8] = [
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
    AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1), AtomicI32::new(-1),
];
static DMA_STREAM_FLAGS: [AtomicU8; 8] = [
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
    AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0), AtomicU8::new(0),
];

pub fn set_dma_intr_info(stream_idx: usize, irq: i32, flags: u8) {
    if stream_idx < 8 {
        DMA_STREAM_IRQ[stream_idx].store(irq, Ordering::Release);
        DMA_STREAM_FLAGS[stream_idx].store(flags, Ordering::Release);
    }
}

// --- CAN bus: staged transmit requests arbitrate globally across CAN1/CAN2.
// A TXRQ mailbox write stages a frame; the next system tick runs arbitration
// (lowest arbitration ID wins; ties broken by node, then mailbox index). The
// winner's mailbox completes (TSR TXOK|TME|RQCP) and the frame is delivered
// to every node's RX FIFO that passes its filter banks (the transmitter also
// receives its own frame, matching real CAN self-ACK traffic). Losers stay
// staged and complete on the next free round.
#[derive(Debug, Clone, Copy)]
pub struct CanFrame {
    pub node: u8,        // 1 = CAN1, 2 = CAN2
    pub mailbox: usize,  // 0..=2
    pub id: u32,         // 11-bit STID, or 29-bit value for extended frames
    pub ext: bool,
    pub rtr: bool,
    pub dlc: u8,
    pub data: [u8; 8],
    pub loopback: bool,  // BTR LBKM: deliver only to the transmitting node
}

static CAN_STAGED: OnceLock<Mutex<Vec<CanFrame>>> = OnceLock::new();
fn can_staged() -> &'static Mutex<Vec<CanFrame>> {
    CAN_STAGED.get_or_init(|| Mutex::new(Vec::new()))
}

pub fn can_stage_tx(f: CanFrame) {
    can_staged().lock().unwrap().push(f);
}

pub(crate) fn can_take_staged() -> Vec<CanFrame> {
    std::mem::take(&mut *can_staged().lock().unwrap())
}

pub(crate) fn can_restage(frames: Vec<CanFrame>) {
    can_staged().lock().unwrap().extend(frames);
}

// --- Audio: WAV-backed sample source + TX capture FIFO --------------------
// JS loads a real WAV file (audio_load_wav) into the PCM source. I2S/SAI DR
// reads (RX/DMA PERIPH->MEM) consume the next source sample; DR writes (TX/
// DMA MEM->PERIPH) append to the capture FIFO, which JS drains with
// audio_take_capture (playback in the browser via WebAudio, or comparison
// against the firmware's intended stream in tests).
pub struct PcmSource {
    pub data: Vec<i16>,
    pub cursor: usize,
}

pub fn audio_clear() {
    if let Some(m) = AUDIO_SOURCE.get() {
        *m.lock().unwrap() = None;
    }
    if let Some(m) = AUDIO_CAPTURE.get() {
        m.lock().unwrap().clear();
    }
}

/// Parse a standard RIFF WAV (PCM 16-bit, mono or stereo — stereo sources
/// are downmixed by taking the left channel), returning an error string on
/// malformed input or unsupported formats.
pub fn audio_load_wav(bytes: &[u8]) -> Result<(), String> {
    if bytes.len() < 44 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a RIFF/WAVE file".to_string());
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u16)> = None; // (format, channels, bits)
    let mut data: Option<(usize, usize)> = None; // (offset, len)
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes([bytes[pos + 4], bytes[pos + 5], bytes[pos + 6], bytes[pos + 7]]) as usize;
        let body = pos + 8;
        if &id[..] == b"fmt " && body + 16 <= bytes.len() {
            fmt = Some((
                u16::from_le_bytes([bytes[body], bytes[body + 1]]),
                u16::from_le_bytes([bytes[body + 2], bytes[body + 3]]),
                u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]),
            ));
        } else if &id[..] == b"data" {
            data = Some((body, size.min(bytes.len() - body)));
            break;
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }
    let (format, channels, bits) = fmt.ok_or("missing fmt chunk")?;
    let (off, len) = data.ok_or("missing data chunk")?;
    if format != 1 {
        return Err(format!("unsupported audio format {format} (only PCM)"));
    }
    if channels == 0 || channels > 2 {
        return Err("channels must be 1 or 2".to_string());
    }
    if bits != 16 {
        return Err(format!("unsupported bit depth {bits} (only 16-bit)"));
    }
    let mut samples = Vec::with_capacity(len / 2);
    let mut i = off;
    // stereo: take the left channel; mono: every sample
    let stride = if channels == 2 { 4 } else { 2 };
    while i + 1 < off + len {
        let s = i16::from_le_bytes([bytes[i], bytes[i + 1]]);
        samples.push(s);
        i += stride;
    }
    if samples.is_empty() {
        return Err("empty data chunk".to_string());
    }
    *AUDIO_SOURCE.get_or_init(|| Mutex::new(None)).lock().unwrap() =
        Some(PcmSource { data: samples, cursor: 0 });
    Ok(())
}

pub fn audio_source_remaining() -> u32 {
    let Some(m) = AUDIO_SOURCE.get() else { return 0 };
    let g = m.lock().unwrap();
    g.as_ref().map_or(0, |s| (s.data.len() - s.cursor) as u32)
}

/// Consume the next source sample (None when no WAV is loaded or it is
/// exhausted — callers fall back to their synthetic generator).
pub fn audio_source_next() -> Option<i16> {
    let mut g = AUDIO_SOURCE.get()?.lock().unwrap();
    let src = g.as_mut()?;
    if src.cursor >= src.data.len() { return None; }
    let s = src.data[src.cursor];
    src.cursor += 1;
    Some(s)
}

pub fn audio_capture_push(v: u16) {
    AUDIO_CAPTURE.get_or_init(|| Mutex::new(Vec::new())).lock().unwrap().push(v);
}

pub fn audio_take_capture() -> Vec<u16> {
    AUDIO_CAPTURE.get().map_or(Vec::new(), |m| std::mem::take(&mut *m.lock().unwrap()))
}

static AUDIO_SOURCE: OnceLock<Mutex<Option<PcmSource>>> = OnceLock::new();
static AUDIO_CAPTURE: OnceLock<Mutex<Vec<u16>>> = OnceLock::new();
pub(crate) fn audio_buses_ready() -> bool {
    AUDIO_SOURCE.get().is_some() && AUDIO_CAPTURE.get().is_some()
}

// ── ADC channel-value injection (JS hardware layer plumbing) ───────────────
// A global override table, not per-Adc-instance state: JS can set/clear a
// channel value at any time (unlike spi_tap/i2c_register_slave, which must
// run before init() because Spi/I2c snapshot their device list once at
// construction — see docs/components.md). Adc::start_conversion checks this
// before falling back to its synthetic temp/vref/vbat/random logic.
static ADC_OVERRIDES: OnceLock<Mutex<std::collections::HashMap<(String, u32), u32>>> = OnceLock::new();

fn adc_overrides() -> &'static Mutex<std::collections::HashMap<(String, u32), u32>> {
    ADC_OVERRIDES.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
pub fn adc_set_override(peripheral: &str, channel: u32, value: u32) {
    adc_overrides().lock().unwrap().insert((peripheral.to_string(), channel), value & 0xFFF);
}
pub fn adc_clear_override(peripheral: &str, channel: u32) {
    adc_overrides().lock().unwrap().remove(&(peripheral.to_string(), channel));
}
pub fn adc_get_override(peripheral: &str, channel: u32) -> Option<u32> {
    adc_overrides().lock().unwrap().get(&(peripheral.to_string(), channel)).copied()
}

// ── SPI bus taps (JS hardware layer plumbing) ──────────────────────────────
// Event word layout: bit 31 = CS edge event, bit 30 = asserted (1) when CS
// is a CS event, bit 29 = DC level (1 = data) when the tap has a DC pin,
// bits 7..0 = the shifted byte. Byte and CS events interleave in the order
// the controller produced them.
static SPI_TAP_EVENTS: OnceLock<Mutex<std::collections::HashMap<String, Vec<u32>>>> = OnceLock::new();
static SPI_TAP_MISO: OnceLock<Mutex<std::collections::HashMap<String, Vec<u8>>>> = OnceLock::new();

fn spi_tap_events() -> &'static Mutex<std::collections::HashMap<String, Vec<u32>>> {
    SPI_TAP_EVENTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
fn spi_tap_miso() -> &'static Mutex<std::collections::HashMap<String, Vec<u8>>> {
    SPI_TAP_MISO.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn spi_tap_push_byte(peri: &str, v: u32) {
    spi_tap_events().lock().unwrap().entry(peri.to_string()).or_default().push(v & 0x2FF);
}
pub fn spi_tap_push_cs(peri: &str, asserted: bool) {
    let e = 0x8000_0000u32 | (if asserted { 1 << 30 } else { 0 });
    spi_tap_events().lock().unwrap().entry(peri.to_string()).or_default().push(e);
}
pub fn spi_tap_take_events(peri: &str) -> Vec<u32> {
    spi_tap_events().lock().unwrap().get_mut(peri).map(std::mem::take).unwrap_or_default()
}
pub fn spi_tap_miso_push(peri: &str, bytes: &[u8]) {
    spi_tap_miso().lock().unwrap().entry(peri.to_string()).or_default().extend_from_slice(bytes);
}
pub(crate) fn spi_tap_miso_pop(peri: &str) -> u8 {
    spi_tap_miso().lock().unwrap().get_mut(peri).and_then(|q| q.first().copied().map(|b| { q.remove(0); b })).unwrap_or(0xFF)
}

// ── I2C bus taps (JS hardware layer plumbing) ─────────────────────────────
// The TX queue carries u32 events: bit31 = boundary event (bit30 = 1 START /
// 0 STOP), otherwise the low byte is one master-write byte. START/STOP let
// the JS device parser find transaction group boundaries (SSD1306 needs
// them: a data group's length is only terminated by STOP).
static I2C_TAP_TX: OnceLock<Mutex<std::collections::HashMap<String, Vec<u32>>>> = OnceLock::new();
static I2C_TAP_RX: OnceLock<Mutex<std::collections::HashMap<String, Vec<u8>>>> = OnceLock::new();

fn i2c_tap_tx() -> &'static Mutex<std::collections::HashMap<String, Vec<u32>>> {
    I2C_TAP_TX.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
fn i2c_tap_rx() -> &'static Mutex<std::collections::HashMap<String, Vec<u8>>> {
    I2C_TAP_RX.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub fn i2c_tap_push_tx(peri: &str, v: u8) {
    i2c_tap_tx().lock().unwrap().entry(peri.to_string()).or_default().push(v as u32);
}
pub fn i2c_tap_push_event(peri: &str, ev: u32) {
    i2c_tap_tx().lock().unwrap().entry(peri.to_string()).or_default().push(ev);
}
pub fn i2c_tap_take_tx(peri: &str) -> Vec<u32> {
    i2c_tap_tx().lock().unwrap().get_mut(peri).map(std::mem::take).unwrap_or_default()
}
pub fn i2c_tap_rx_push(peri: &str, bytes: &[u8]) {
    i2c_tap_rx().lock().unwrap().entry(peri.to_string()).or_default().extend_from_slice(bytes);
}
pub(crate) fn i2c_tap_rx_pop(peri: &str) -> u8 {
    i2c_tap_rx().lock().unwrap().get_mut(peri).and_then(|q| q.first().copied().map(|b| { q.remove(0); b })).unwrap_or(0xFF)
}

// ── FSMC bank taps (JS memory-mapped device plumbing) ─────────────────────
// Each access is TWO event words, so the JS device sees the address as well
// as the value (an 8080-mode display decodes one address line as RS/DC —
// command vs pixel data — so the offset is what distinguishes them):
//   word0: bit31 = 1 write / 0 read, bits 30..0 = byte offset in the bank
//   word1: value written, or value returned on a read
// Reads are answered from FSMC_TAP_DATA, a JS-pushed queue (`fsmc_push_data`)
// analogous to the SPI tap's MISO queue; an empty queue reads back 0.
static FSMC_TAP_EVENTS: OnceLock<Mutex<std::collections::HashMap<usize, Vec<u32>>>> = OnceLock::new();
static FSMC_TAP_DATA: OnceLock<Mutex<std::collections::HashMap<usize, Vec<u32>>>> = OnceLock::new();

fn fsmc_tap_events() -> &'static Mutex<std::collections::HashMap<usize, Vec<u32>>> {
    FSMC_TAP_EVENTS.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}
fn fsmc_tap_data() -> &'static Mutex<std::collections::HashMap<usize, Vec<u32>>> {
    FSMC_TAP_DATA.get_or_init(|| Mutex::new(std::collections::HashMap::new()))
}

pub(crate) fn fsmc_tap_push(bank: usize, write: bool, offset: u32, value: u32) {
    let hdr = (offset & 0x7FFF_FFFF) | if write { 1 << 31 } else { 0 };
    let mut m = fsmc_tap_events().lock().unwrap();
    let q = m.entry(bank).or_default();
    q.push(hdr);
    q.push(value);
}
pub fn fsmc_tap_take_events(bank: usize) -> Vec<u32> {
    fsmc_tap_events().lock().unwrap().get_mut(&bank).map(std::mem::take).unwrap_or_default()
}
pub fn fsmc_tap_data_push(bank: usize, values: &[u32]) {
    fsmc_tap_data().lock().unwrap().entry(bank).or_default().extend_from_slice(values);
}
pub(crate) fn fsmc_tap_data_pop(bank: usize) -> u32 {
    fsmc_tap_data().lock().unwrap().get_mut(&bank)
        .and_then(|q| if q.is_empty() { None } else { Some(q.remove(0)) })
        .unwrap_or(0)
}

// ── "this register read is coming from the DMA engine" ────────────────────
// Set around the peripheral-side reads the DMA driver issues. A peripheral
// register read is otherwise indistinguishable from a CPU load, but for a
// STREAMING peripheral the difference is the whole point: the DMA drains at
// bus rate and cannot overrun, while a CPU polling loop is far too slow and
// does. DCMI reads it in its DR handler.
static DMA_READ_ACTIVE: AtomicBool = AtomicBool::new(false);
pub fn set_dma_read_active(v: bool) { DMA_READ_ACTIVE.store(v, Ordering::Relaxed); }
pub(crate) fn dma_read_active() -> bool { DMA_READ_ACTIVE.load(Ordering::Relaxed) }

// ── DCMI frame source (JS camera sensor plumbing) ─────────────────────────
static DCMI_FRAME: OnceLock<Mutex<Option<(u32, u32, Vec<u8>)>>> = OnceLock::new();
pub fn dcmi_feed_frame(w: u32, h: u32, pixels: &[u8]) {
    *DCMI_FRAME.get_or_init(|| Mutex::new(None)).lock().unwrap() = Some((w, h, pixels.to_vec()));
}
pub(crate) fn dcmi_frame() -> Option<(u32, u32, Vec<u8>)> {
    DCMI_FRAME.get()?.lock().unwrap().clone()
}
pub fn dcmi_clear() {
    if let Some(m) = DCMI_FRAME.get() {
        *m.lock().unwrap() = None;
    }
}

pub struct WasmSystem {
    pub p: Rc<Peripherals>,
    pending_dma: RefCell<Vec<DmaTransfer>>,
}

#[cfg(test)]
pub fn test_dummy_system() -> ::std::rc::Rc<crate::system::System> {
    use crate::ext_devices::ExtDevices;
    use crate::peripherals::Peripherals;
    let gpio = GpioPorts::default();
    // Empty ext devices: keeps tests independent of the global (shared,
    // Rc<RefCell>-based) device list, whose cross-thread borrows race when
    // tests run in parallel (see bug fix 2026-08-10).
    let empty = ExtDevices::default();
    let p = Rc::new(Peripherals::new_wasm(gpio, &empty));
    ::std::rc::Rc::new(WasmSystem { p, pending_dma: RefCell::new(Vec::new()) })
}

/// Like `test_dummy_system` but with a caller-supplied device list, for
/// tests that need a peripheral actually bound to an ext device (the
/// binding happens once, at construction).
#[cfg(test)]
pub fn test_system_with(ext: &crate::ext_devices::ExtDevices) -> ::std::rc::Rc<crate::system::System> {
    use crate::peripherals::Peripherals;
    let p = Rc::new(Peripherals::new_wasm(GpioPorts::default(), ext));
    ::std::rc::Rc::new(WasmSystem { p, pending_dma: RefCell::new(Vec::new()) })
}

#[cfg(test)]
pub fn dummy_gpio() -> crate::peripherals::gpio::GpioPorts {
    crate::peripherals::gpio::GpioPorts::default()
}

impl WasmSystem {
    pub fn new() -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::new_wasm(gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()) }
    }

    pub fn new_svd(svd_xml: &str) -> Self {
        let gpio = GpioPorts::default();
        let ext = get_ext_devices().lock().unwrap();
        let p = Rc::new(Peripherals::from_svd(svd_xml, gpio, &*ext));
        drop(ext);
        Self::register_software_spis(&p);
        WasmSystem { p, pending_dma: RefCell::new(Vec::new()) }
    }

    fn register_software_spis(p: &Peripherals) {
        use crate::peripherals::sw_spi::{SoftwareSpi, SoftwareSpiConfig};
        let configs = get_software_spi_configs().lock().unwrap();
        let ext_devices = get_ext_devices().lock().unwrap();
        for (name, cs, clk, miso, mosi) in configs.iter() {
            let config = SoftwareSpiConfig {
                name: name.clone(),
                cs: cs.clone(),
                clk: clk.clone(),
                miso: miso.clone(),
                mosi: mosi.clone(),
            };
            SoftwareSpi::register(config, &mut p.gpio.borrow_mut(), &ext_devices);
        }
    }

    pub fn queue_dma_transfer(&self, t: DmaTransfer) {
        self.pending_dma.borrow_mut().push(t);
    }

    pub fn pending_dma_count(&self) -> usize {
        self.pending_dma.borrow().len()
    }

    /// Is a queued DMA transfer aimed at a peripheral register inside
    /// `[start, end)`? A streaming peripheral uses this to tell "the DMA is
    /// my consumer" from "the CPU is polling me", which are very different
    /// flow-control situations — see the DCMI tick.
    pub fn dma_pending_for_range(&self, start: u32, end: u32) -> bool {
        self.pending_dma.borrow().iter()
            .any(|t| t.peripheral && t.peri_addr >= start && t.peri_addr < end)
    }

    pub fn take_pending_dma_transfer(&self, index: usize) -> Option<DmaTransfer> {
        let mut pending = self.pending_dma.borrow_mut();
        if index < pending.len() {
            Some(pending.remove(index))
        } else {
            None
        }
    }

    pub fn mark_dma_completed(&self, stream_idx: usize, _success: bool) {
        DMA_COMPLETED[stream_idx].store(true, Ordering::Release);
        // Fire NVIC interrupt after transfer completes
        if stream_idx < 8 {
            let irq = DMA_STREAM_IRQ[stream_idx].swap(-1, Ordering::Acquire);
            if irq >= 0 {
                let flags = DMA_STREAM_FLAGS[stream_idx].swap(0, Ordering::Acquire);
                if flags & 0x7 != 0 {
                    self.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }

    pub fn dma_check_completion(&self, stream_idx: usize) -> bool {
        DMA_COMPLETED[stream_idx].swap(false, Ordering::Acquire)
    }

    pub fn tick(&self) {
        let p = self.p.clone();
        for slot in &p.peripherals {
            slot.peripheral.borrow_mut().tick(self);
        }
        crate::peripherals::can::arbitrate_bus(self);
        p.nvic.borrow_mut().maybe_set_systick_intr_pending();
    }

    pub fn addr_desc(&self, addr: u32) -> String {
        self.p.addr_desc(addr)
    }
}

pub type System = WasmSystem;

// SAFETY: WasmSystem contains Rc<RefCell> peripherals and is single-system
// (SYS AtomicPtr). On wasm32-unknown-unknown the module is single-threaded
// — Send/Sync are never exercised. wasm-bindgen requires them for exported
// types, so we assert unsafely. Native `cargo test` (multi-threaded) is
// guarded by per-suite Mutex locks (CAN_TEST_LOCK, AUDIO_TEST_LOCK, etc.)
// to avoid `already borrowed` panics. Do not share WasmSystem across OS
// threads in a native build; use the WASM artifact for multi-instance.
#[cfg(target_arch = "wasm32")]
unsafe impl Sync for WasmSystem {}
#[cfg(target_arch = "wasm32")]
unsafe impl Send for WasmSystem {}
#[cfg(not(target_arch = "wasm32"))]
unsafe impl Sync for WasmSystem {}
#[cfg(not(target_arch = "wasm32"))]
unsafe impl Send for WasmSystem {}

#[cfg(test)]
mod audio_tests {
    use super::*;

    // AUDIO_SOURCE / AUDIO_CAPTURE are process-global (the model is
    // single-system), so audio tests must run serially.
    static AUDIO_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    pub fn make_pcm16_wav(samples: &[i16], channels: u16) -> Vec<u8> {
        let data_len = samples.len() * 2 * channels as usize;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len as u32).to_le_bytes());
        w.extend_from_slice(b"WAVE");
        w.extend_from_slice(b"fmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes()); // PCM
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&44100u32.to_le_bytes());
        w.extend_from_slice(&(44100 * 2 * channels as u32).to_le_bytes());
        w.extend_from_slice(&(2 * channels as u16).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&(data_len as u32).to_le_bytes());
        for s in samples {
            for _ in 0..channels {
                w.extend_from_slice(&s.to_le_bytes());
            }
        }
        w
    }

    #[test]
    fn wav_parse_mono_pcm16() {
        let _g = AUDIO_TEST_LOCK.lock().unwrap();
        audio_clear();
        let src: Vec<i16> = (0..8).map(|i| i * 100 + 1).collect();
        let wav = make_pcm16_wav(&src, 1);
        assert!(audio_load_wav(&wav).is_ok());
        assert_eq!(audio_source_remaining(), 8);
        for (i, s) in src.iter().enumerate() {
            assert_eq!(audio_source_next(), Some(*s), "sample {i}");
        }
        assert_eq!(audio_source_next(), None);
        assert_eq!(audio_source_remaining(), 0);
    }

    #[test]
    fn wav_stereo_downmix_takes_left() {
        let _g = AUDIO_TEST_LOCK.lock().unwrap();
        audio_clear();
        let wav = make_pcm16_wav(&[7, 99], 2);
        assert!(audio_load_wav(&wav).is_ok());
        assert_eq!(audio_source_next(), Some(7));
        assert_eq!(audio_source_next(), Some(99));
    }

    #[test]
    fn wav_rejects_garbage_and_bad_format() {
        let _g = AUDIO_TEST_LOCK.lock().unwrap();
        audio_clear();
        assert!(audio_load_wav(b"nope").is_err());
        let mut wav = make_pcm16_wav(&[1, 2, 3], 1);
        wav[20] = 3; // corrupt audio format -> not PCM
        assert!(audio_load_wav(&wav).is_err());
    }

    #[test]
    fn capture_fifo_roundtrip() {
        let _g = AUDIO_TEST_LOCK.lock().unwrap();
        audio_clear();
        assert_eq!(audio_take_capture(), Vec::<u16>::new());
        audio_capture_push(0x1234);
        audio_capture_push(0x5678);
        assert_eq!(audio_take_capture(), vec![0x1234, 0x5678]);
        assert_eq!(audio_take_capture(), Vec::<u16>::new());
    }
}

// ── process-wide state reset ────────────────────────────────────────────────
/// Clear every process-lifetime global so a fresh emulator instance starts
/// clean.  Without this, creating a second instance in the same process is
/// broken in a subtle way: `ExtDevices` ACCUMULATES, and the peripheral
/// constructors use `find_*_device(name)`, which returns the FIRST match —
/// so instance 2 silently binds to instance 1's devices (measured: a regfile
/// seeded 0x22 read back 0x11 from the previous instance, and rtc_test hung
/// right after its first UART line when run after another firmware).
///
/// Call this BEFORE registering devices for a new instance (emulator.js does
/// it immediately after the wasm module is ready).  It is safe to call when
/// no instance exists — every table is lazily created.
pub fn reset_globals() {
    use std::sync::atomic::Ordering::Relaxed;
    if let Some(m) = EXT_DEVICES.get() { *m.lock().unwrap() = ExtDevices::default(); }
    if let Some(m) = SOFTWARE_SPI_CONFIGS.get() { m.lock().unwrap().clear(); }
    if let Some(m) = UART_OUTPUT.get() { m.lock().unwrap().clear(); }
    if let Some(m) = SPI_TAP_EVENTS.get() { m.lock().unwrap().clear(); }
    if let Some(m) = SPI_TAP_MISO.get() { m.lock().unwrap().clear(); }
    if let Some(m) = FSMC_TAP_EVENTS.get() { m.lock().unwrap().clear(); }
    if let Some(m) = FSMC_TAP_DATA.get() { m.lock().unwrap().clear(); }
    if let Some(m) = I2C_TAP_TX.get() { m.lock().unwrap().clear(); }
    if let Some(m) = I2C_TAP_RX.get() { m.lock().unwrap().clear(); }
    if let Some(m) = ADC_OVERRIDES.get() { m.lock().unwrap().clear(); }
    if let Some(m) = CAN_STAGED.get() { m.lock().unwrap().clear(); }
    if let Some(m) = AUDIO_SOURCE.get() { *m.lock().unwrap() = None; }
    if let Some(m) = AUDIO_CAPTURE.get() { m.lock().unwrap().clear(); }
    if let Some(m) = DCMI_FRAME.get() { *m.lock().unwrap() = None; }
    *FLASH_ERASE.lock().unwrap() = None;
    DMA_READ_ACTIVE.store(false, Relaxed);
    FLASH_PROGRAMMING.store(false, Relaxed);
    WATCHDOG_RESET_EVENT.store(false, Relaxed);
    ETH_TX_POLL.store(false, Relaxed);
    ETH_RX_POLL.store(false, Relaxed);
    ETH_DONE.store(0, Relaxed);
    ETH_TX_DESC_ADDR.store(0, Relaxed);
    ETH_RX_DESC_ADDR.store(0, Relaxed);
    for i in 0..8 {
        DMA_COMPLETED[i].store(false, Relaxed);
        DMA_STREAM_IRQ[i].store(0, Relaxed);
        DMA_STREAM_FLAGS[i].store(0, Relaxed);
    }
    // NOTE: deliberately NOT resetting INSTRUCTION_COUNT here — peripherals
    // capture last_tick at construction; zeroing the global afterwards makes
    // elapsed = now.wrapping_sub(last_tick) enormous and breaks tick logic.
    // INSTRUCTION_COUNT.store(0, Relaxed);
}
