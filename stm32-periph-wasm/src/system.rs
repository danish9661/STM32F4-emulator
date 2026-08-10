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

static WATCHDOG_RESET: AtomicBool = AtomicBool::new(false);

// Software SPI configs queued before init, registered after GPIO exists
static SOFTWARE_SPI_CONFIGS: OnceLock<Mutex<Vec<(String, Option<String>, String, String, String)>>> = OnceLock::new();
pub fn get_software_spi_configs() -> &'static Mutex<Vec<(String, Option<String>, String, String, String)>> {
    SOFTWARE_SPI_CONFIGS.get_or_init(|| Mutex::new(Vec::new()))
}
pub fn is_watchdog_reset_requested() -> bool { WATCHDOG_RESET.swap(false, Ordering::Acquire) }
pub fn request_watchdog_reset() { WATCHDOG_RESET.store(true, Ordering::Release); }

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

unsafe impl Sync for WasmSystem {}
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
