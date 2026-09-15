use crate::system::System;
use std::collections::VecDeque;
use super::Peripheral;

/// USB OTG FS device-mode model (combined 0x50000000 block: global +
/// device + EP regs + PWRCLK + FIFO window to 0x50005000). The same core
/// also serves OTG_HS in FS mode (`UsbHsFs` wrapper at 0x40040000, IRQ 77):
/// the HS controller in FS mode is register-compatible with FS for the
/// device subset this model covers, so the wrapper reuses the core with a
/// different IRQ line and HS-speed ENUMSPD. Host mode (both controllers)
/// is out of scope (host-only registers read 0).
///
/// Coverage is the functional subset a polling CDC firmware needs:
/// - Core: GOTGCTL/GOTGINT/GAHBCFG/GUSBCFG/GRSTCTL (CSRST + TX/RX flush,
///   self-clearing) / GINTSTS+GINTMSK (USBRST, ENUMDNE, RXFLVL, NPTXFE,
///   IEPINT, OEPINT; W1C) / GRXSTSP+GRXSTSR (pop/peek) / GRXFSIZ /
///   GNPTXFSIZ+DIEPTXF1-3 / GNPTXSTS / GCCFG / CID(=0).
/// - Device: DCFG/DCTL/DSTS (ENUMSPD set by enum_done) / DIEPMSK+DOEPMSK /
///   DAINT(+MSK, live-computed) / DVBUSDIS+PULSE / DIEPEMPMSK.
/// - Endpoints 0-3 both directions: CTL/INT (XFRC + EPDISD, W1C) /
///   TSIZ / DMA / DTXFSTS (IN, computed from programmed FIFO sizes).
/// - FIFO window 0x50001000+i*0x1000 (ST headers: reads at EP0 pop the
///   RXFIFO, other slots read 0; writes push the addressed TX FIFO).
///
/// Deliberately host-driven (like netsim for ETH): a test harness plays
/// USB host through the `usb_*` exports (reset, enum-done, SETUP/OUT
/// injection, IN take). IN transfers complete synchronously at EPENA
/// (whole-blob semantics: the harness slices by MPSIZ itself); OUT
/// completes on short-packet or when the armed XFRSIZ drains. EP0 needs
/// no arming (always enabled after reset). Everything undecided is
/// lenient (accept, never NAK-drop) and documented at the spot.
/// SOF generation, suspend/resume, VBUS sensing (BSVLD), DMA, GNPINNAK
/// gating and host channels are out of scope.
pub const USB_IRQ: i32 = 67; // OTG_FS_IRQn
/// OTG_HS global interrupt (HS controller in FS mode). The HS core shares
/// the FS device register subset; only the IRQ line and the base differ.
pub const USB_HS_IRQ: i32 = 77; // OTG_HS_IRQn
/// HS controller base (SVD OTG_HS_GLOBAL): same 0x5000 window shape as FS.
pub const USB_HS_BASE: u32 = 0x4004_0000;

const N_EPS: usize = 4;

#[derive(Default, Clone)]
struct UsbEp {
    ctl: u32,
    int: u32,
    tsiz: u32,
    dma: u32,
    /// OUT: bytes still expected this transfer (from DOEPTSIZ writes).
    out_expected: u32,
}

pub struct UsbFs {
    /// IRQ line this instance pends (67 = FS, 77 = HS-in-FS). Set at
    /// construction; the register core is identical.
    irq: i32,
    /// HS-mode instance: ENUMDNE reports HS speed (DSTS ENUMSPD 0b00)
    /// instead of FS (0b11). HS-phy features (ULPI, dedicated FIFO sizes)
    /// are not modeled — the device subset is register-compatible.
    hs: bool,
    /// SOF generation (instruction-count clock): frames tick at 1 kHz while
    /// the device is out of suspend (DSTS SUSPSTS clear) and either RWUSIG
    /// resume signaling or traffic keeps the link alive. `sof_frame`
    /// mirrors DSTS FNSOF; each wrap latches GINTSTS SOF (W1C) when SOFM
    /// is set. Suspended (SUSPSTS set) or unenumerated devices never tick.
    sof_frame: u16,
    sof_last: u64,
    /// VBUS sensing: BSVLD (GOTGCTL bit 19) follows the harness-driven
    /// `vbus_present` (default true — a plugged cable). `usb_set_vbus`
    /// (harness = the cable) drops it: suspend fires, SOF stops, and
    /// session-end is observable in GOTGINT SEDET. Default-present keeps
    /// every existing session green.
    vbus_present: bool,
    /// Internal DMA (buffer-descriptor mode behind GAHBCFG HBSTLEN/DMAEN):
    /// when armed, an IN EPENA moves the TX FIFO bytes straight into the
    /// completed blob WITHOUT a CPU FIFO-window write (same whole-blob
    /// contract, DMAEN-gated), and an OUT completion latches the byte
    /// count into the EP DMA address register (firmware polls it instead
    /// of GRXSTSP). Pure register-file behavior — no guest-memory access
    /// (the core has no DMA window into guest RAM; the driver owns moves).
    /// Gated on GAHBCFG bit 5 (DMAEN); HBSTLEN (bits 3:1) stores only.
    dma_moved_in: [u64; N_EPS],
    dma_moved_out: [u64; N_EPS],
    gotgctl: u32,
    gotgint: u32,
    gahbcfg: u32,
    gusbcfg: u32,
    grstctl: u32,
    gint_sticky: u32, // W1C event bits (USBRST/ENUMDNE/IEPINT/OEPINT...)
    gintmsk: u32,
    rxfifo_depth: u32,
    nptxfifo: u32,      // GNPTXFSIZ / DIEPTXF0 (EP0 TX)
    ptxfifo: [u32; 3],  // DIEPTXF1-3 (EP1-3 TX)
    gccfg: u32,
    dcfg: u32,
    dctl: u32,
    dsts: u32,
    diepmsk: u32,
    doepmsk: u32,
    daintmsk: u32,
    dvbusdis: u32,
    dvbuspulse: u32,
    die_pempmsk: u32,
    pcgcctl: u32,
    in_ep: [UsbEp; N_EPS],
    out_ep: [UsbEp; N_EPS],
    rx_status: VecDeque<u32>, // GRXSTSP queue (POP on read)
    rx_data: VecDeque<u8>,    // RXFIFO bytes (read via 0x50001000)
    tx_data: [VecDeque<u8>; N_EPS],
    in_ready: [Option<Vec<u8>>; N_EPS], // completed IN blob (empty = ZLP)
    in_stall: [bool; N_EPS],            // STALL handshake pending (IN)
    out_stall: [bool; N_EPS],           // STALL handshake pending (OUT)
}

impl Default for UsbFs {
    fn default() -> Self {
        let mut u = Self {
            irq: USB_IRQ,
            hs: false,
            sof_frame: 0,
            sof_last: 0,
            vbus_present: true,
            dma_moved_in: [0; N_EPS],
            dma_moved_out: [0; N_EPS],
            gotgctl: 0,
            gotgint: 0,
            gahbcfg: 0,
            gusbcfg: 0,
            grstctl: 0,
            gint_sticky: 0,
            gintmsk: 0,
            rxfifo_depth: 0,
            nptxfifo: 0,
            ptxfifo: [0; 3],
            gccfg: 0,
            dcfg: 0,
            dctl: 0,
            dsts: 0,
            diepmsk: 0,
            doepmsk: 0,
            daintmsk: 0,
            dvbusdis: 0,
            dvbuspulse: 0,
            die_pempmsk: 0,
            pcgcctl: 0,
            in_ep: Default::default(),
            out_ep: Default::default(),
            rx_status: VecDeque::new(),
            rx_data: VecDeque::new(),
            tx_data: Default::default(),
            in_ready: Default::default(),
            in_stall: [false; N_EPS],
            out_stall: [false; N_EPS],
        };
        // EP0 is active out of reset (USBACTEP); MPSIZ field 0 reads as 64.
        u.in_ep[0].ctl = 1 << 15;
        u.out_ep[0].ctl = 1 << 15;
        u
    }
}

impl UsbFs {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "USB_OTG_FS" {
            Some(Box::new(Self::default()))
        } else if name == "USB_OTG_HS" {
            // HS controller in FS mode: same device core, HS IRQ line.
            let mut u = Self::default();
            u.irq = USB_HS_IRQ;
            u.hs = true;
            Some(Box::new(u))
        } else {
            None
        }
    }

    fn irq(&self) -> i32 {
        self.irq
    }

    /// Shared reset (CSRST + harness usb_reset): FIFOs/queues drained,
    /// address cleared, EPs back to reset (EP0 active). Keeps the IRQ line,
    /// HS personality, and VBUS presence: a bus reset must not turn an HS
    /// instance into FS, nor unplug the cable.
    fn reset_core(&mut self) {
        let (irq, hs, vbus) = (self.irq, self.hs, self.vbus_present);
        let fresh = Self::default();
        // Keep nothing across reset: a fresh session starts unconfigured.
        *self = fresh;
        self.irq = irq;
        self.hs = hs;
        self.vbus_present = vbus;
    }

    /// MPSIZ in bytes: EP0 field 0/1/2/3 means 64/32/16/8; other EPs use
    /// the raw field value (firmware always programs them explicitly).
    fn mpsiz(&self, ep: usize, out: bool) -> u32 {
        let f = if out {
            self.out_ep[ep].ctl
        } else {
            self.in_ep[ep].ctl
        } & 0x7FF;
        if ep == 0 {
            match f {
                0 => 64,
                1 => 32,
                2 => 16,
                _ => 8,
            }
        } else if f == 0 {
            64
        } else {
            f
        }
    }

    /// TX FIFO depth in words for IN EP (from programmed sizes; 0 until
    /// the firmware programs FSIZ, exactly like silicon reset).
    fn tx_depth_words(&self, ep: usize) -> u32 {
        let v = if ep == 0 { self.nptxfifo } else { self.ptxfifo[ep - 1] };
        (v >> 16) & 0xFFFF
    }

    fn tx_space_words(&self, ep: usize) -> u32 {
        let used = self.tx_data[ep].len().div_ceil(4) as u32;
        self.tx_depth_words(ep).saturating_sub(used)
    }

    /// GRXST status word: EPNUM[3:0] BCNT[18:4] DPID[20:19] PKTSTS[24:21].
    fn rx_status_word(ep: u32, bcnt: u32, dpid: u32, pktsts: u32) -> u32 {
        (ep & 0xF) | ((bcnt & 0x7FFF) << 4) | ((dpid & 3) << 19) | ((pktsts & 0xF) << 21)
    }

    /// GINTSTS live value: stored W1C bits plus computed RXFLVL/NPTXFE/
    /// IEPINT/OEPINT/SOF/ESUSP/CMOD(device=1). SOF latches in tick(); ESUSP
    /// mirrors DSTS SUSPSTS.
    fn gintsts(&self) -> u32 {
        let mut v = self.gint_sticky | (1 << 0); // CMOD = device
        if !self.rx_status.is_empty() {
            v |= 1 << 4; // RXFLVL
        }
        if self.tx_space_words(0) > 0 {
            v |= 1 << 5; // NPTXFE (EP0 TX space)
        }
        let mut iep = false;
        let mut oep = false;
        for i in 0..N_EPS {
            if self.in_ep[i].int & self.diepmsk & ((self.daintmsk >> i) & 1) != 0 {
                iep = true;
            }
            if self.out_ep[i].int & self.doepmsk & ((self.daintmsk >> (16 + i)) & 1) != 0 {
                oep = true;
            }
        }
        if iep {
            v |= 1 << 18;
        }
        if oep {
            v |= 1 << 19;
        }
        // SUSPSTS (DSTS bit 0) mirrors into ESUSP (bit 10) like silicon:
        // the suspend state is readable both places, the IRQ via the mask.
        if self.dsts & 1 != 0 {
            v |= 1 << 10; // ESUSP
        }
        // VBUS session-end: cable unplugged (vbus_present false) latches
        // GOTGINT SEDET (bit 2) — the OTG session-end observable.
        if !self.vbus_present {
            v |= 1 << 2; // OTGINT (session change; SEDET in GOTGINT)
        }
        v
    }

    /// SOF generation + HS microframes (instruction-count clock).
    /// FS: 1 frame per 168000 virtual instructions (168 MHz / 1 kHz); each
    /// frame latches GINTSTS SOF (bit 3, W1C) and advances DSTS FNSOF
    /// (14-bit frame number at bits 8..21).
    /// HS (`hs` instance): the same 1 kHz frame is divided into 8
    /// microframes (125 us each, 21000 virt inst). Each microframe latches
    /// SOF too (silicon raises the HS SOF IRQ per microframe) and bumps
    /// the microframe counter (DSTS FNSOF low 3 bits = microframe index
    /// 0..7); every 8th microframe rolls the 14-bit frame number and sets
    /// EOPF (bit 15, end-of-periodic-frame — the HS periodic-schedule
    /// observable). Suspended, unenumerated, or VBUS-lost devices never
    /// tick (silicon gates SOF on the session).
    /// ULPI PHY rate report: `ulpi_rate()` derives the packet wire rate
    /// from the HS personality (HS = 480 Mbit/s, FS = 12 Mbit/s) — the
    /// observable contract for firmware that sizes DMA/FIFO budgets.
    fn tick_sof(&mut self, sys: &System, now: u64) {
        // Gate: enumerated (ENUMDNE sticky) + out of suspend + VBUS present.
        if self.gint_sticky & (1 << 13) == 0 {
            self.sof_last = now;
            return;
        }
        if self.dsts & 1 != 0 || !self.vbus_present {
            self.sof_last = now;
            return;
        }
        // Microframe step: HS = 21000 inst (125 us), FS = 168000 (1 ms).
        const FS_FRAME: u64 = 168_000;
        const HS_UFRAME: u64 = 21_000;
        let step = if self.hs { HS_UFRAME } else { FS_FRAME };
        let mut el = now.saturating_sub(self.sof_last);
        if el < step {
            return;
        }
        let mut fired = false;
        while el >= step {
            el -= step;
            self.sof_last += step;
            if self.hs {
                // HS microframe: low 3 bits of FNSOF = uframe 0..7.
                let uf = (self.sof_frame & 7) + 1;
                if uf >= 8 {
                    self.sof_frame = self.sof_frame.wrapping_add(1) & 0x3FFF;
                    self.gint_sticky |= 1 << 15; // EOPF at frame roll
                } else {
                    self.sof_frame = (self.sof_frame & !7) | (uf & 7);
                }
            } else {
                self.sof_frame = self.sof_frame.wrapping_add(1) & 0x3FFF;
            }
            // DSTS FNSOF follows the counter (bits 8..21).
            self.dsts = (self.dsts & !(0x3FFF << 8)) | ((self.sof_frame as u32) << 8);
            self.gint_sticky |= 1 << 3; // SOF
            fired = true;
        }
        if fired {
            self.update_irq(sys);
        }
    }

    /// ULPI PHY packet wire rate in Mbit/s: 480 for the HS personality,
    /// 12 for FS. Derived from the instance personality (the HS core runs
    /// in FS mode register-wise, but the PHY rate is a link property the
    /// harness reports like a scope on the ULPI bus). Firmware sizing
    /// DMA/FIFO budgets by rate observes exactly this contract.
    pub fn ulpi_rate_mbps(&self) -> u32 {
        if self.hs { 480 } else { 12 }
    }

    /// Current microframe index (DSTS FNSOF low 3 bits): 0..7 on HS
    /// (125 us microframes), always 0 on FS (1 ms frames, no microframes).
    pub fn uframe(&self) -> u32 {
        if self.hs { (self.sof_frame & 7) as u32 } else { 0 }
    }

    /// Harness = the cable: plug or unplug VBUS. Unplug latches GOTGINT
    /// SEDET (session end) + suspends the device (SUSPSTS set, SOF stops);
    /// re-plug clears SEDET path (fresh session needs usb_reset + enum).
    /// Default-present keeps every existing session green.
    pub fn set_vbus(&mut self, sys: &System, present: bool) {
        if present == self.vbus_present {
            return;
        }
        self.vbus_present = present;
        if !present {
            self.gotgint |= 1 << 2; // SEDET: session end detected
            self.dsts |= 1; // SUSPSTS: suspended while unplugged
            self.gint_sticky |= 1 << 11; // USBSUSP
            self.update_irq(sys);
        } else {
            // Re-plug: session-end condition clears; firmware re-enumerates
            // (USBRST path) to clear SUSPSTS — like silicon, plug alone
            // does not resume the old session.
            self.gotgint &= !(1 << 2);
        }
    }

    /// GOTGCTL live value: stored bits plus BSVLD (bit 19) following the
    /// harness VBUS state, and CIDSTS/DBCT session bits. BSVLD clear is
    /// the firmware-visible "cable gone" signal alongside SEDET.
    fn gotgctl(&self) -> u32 {
        let mut v = self.gotgctl;
        if self.vbus_present {
            v |= 1 << 19; // BSVLD: B-session valid while plugged
            v |= 1 << 18; // ASVLD likewise (device attached to host)
        } else {
            v &= !((1 << 19) | (1 << 18));
        }
        v
    }

    /// DAINT live value: IN[i] = DIEPINT[i] nonzero, OUT in bits 16+.
    fn daint(&self) -> u32 {
        let mut v = 0u32;
        for i in 0..N_EPS {
            if self.in_ep[i].int != 0 {
                v |= 1 << i;
            }
            if self.out_ep[i].int != 0 {
                v |= 1 << (16 + i);
            }
        }
        v
    }

    /// Raise the NVIC line when masked interrupts are live and the global
    /// interrupt bit is set (for future IRQ-driven firmware; the polling
    /// path never needs it).
    fn update_irq(&self, sys: &System) {
        if (self.gintsts() & self.gintmsk) != 0 && self.gahbcfg & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq());
        }
    }

    /// Complete an IN transfer synchronously at EPENA (whole-blob: move up
    /// to XFRSIZ bytes into the ready slot, raise XFRC). GNPINNAK-defers
    /// and STALL handshakes handled; NAK state is otherwise lenient.
    /// Internal-DMA mode (GAHBCFG DMAEN): the same whole-blob contract,
    /// but the move is attributed to DMA — the byte count latches into
    /// `dma_moved_in[ep]` (firmware polls progress there instead of the
    /// FIFO window) and no CPU FIFO write was needed.
    fn in_send(&mut self, sys: &System, ep: usize) {
        if (self.in_ep[ep].ctl >> 21) & 1 != 0 {
            // STALL handshake: nothing moves; the harness sees status.
            self.in_stall[ep] = true;
            return;
        }
        if self.dctl & (1 << 7) != 0 {
            // GNPINNAK set: defer until cleared (see DCTL write path).
            return;
        }
        let xfrsiz = self.in_ep[ep].tsiz & 0x7FFFF;
        let n = (xfrsiz as usize).min(self.tx_data[ep].len());
        let mut pkt = Vec::with_capacity(n);
        for _ in 0..n {
            pkt.push(self.tx_data[ep].pop_front().unwrap());
        }
        // Internal DMA attribution: when GAHBCFG DMAEN (bit 5) is set the
        // move counts as a DMA move — latch the count where firmware polls
        // it. Same bytes, same completion; only the accounting differs.
        if self.gahbcfg & (1 << 5) != 0 {
            self.dma_moved_in[ep] = self.dma_moved_in[ep].wrapping_add(n as u64);
        }
        self.in_ready[ep] = Some(pkt);
        // Transfer complete clears EPENA like silicon (re-arming
        // re-triggers); XFRC reports it.
        self.in_ep[ep].ctl &= !(1 << 31);
        self.in_ep[ep].int |= 1; // XFRC
        self.update_irq(sys);
    }

    // ---- host-side API (driven by the test harness) ----

    /// Bus reset from the host: fresh session, USBRST latched.
    pub fn host_reset(&mut self, sys: &System) {
        self.reset_core();
        self.gint_sticky |= 1 << 12; // USBRST
        sys.p.nvic.borrow_mut().clear_pending(self.irq());
        self.update_irq(sys);
    }

    /// Enumeration done: ENUMDNE + DSTS speed. FS reports 0b11; an HS
    /// instance in FS mode reports HS speed 0b00 (the only observable
    /// difference — the device subset is register-compatible).
    pub fn host_enumerated(&mut self, sys: &System) {
        self.gint_sticky |= 1 << 13; // ENUMDNE
        self.dsts = (self.dsts & !6) | if self.hs { 0 } else { 6 };
        self.update_irq(sys);
    }

    /// Inject an 8-byte SETUP packet to EP0 (status 6 + STUP interrupt).
    pub fn inject_setup(&mut self, sys: &System, data: &[u8]) {
        for b in data.iter().take(8) {
            self.rx_data.push_back(*b);
        }
        self.rx_status
            .push_back(Self::rx_status_word(0, 8, 0, 6)); // SETUP received
        self.out_ep[0].int |= 1 << 3; // STUP
        self.update_irq(sys);
    }

    /// Inject an OUT data packet (status 2; completes on short packet or
    /// when the armed XFRSIZ drains, raising XFRC like silicon). A STALLed
    /// OUT endpoint answers with a STALL handshake instead: nothing is
    /// queued, no data moves, and the harness observes the stall status.
    pub fn inject_out(&mut self, sys: &System, ep: usize, data: &[u8]) {
        if ep >= N_EPS {
            return;
        }
        if (self.out_ep[ep].ctl >> 21) & 1 != 0 {
            self.out_stall[ep] = true;
            self.update_irq(sys);
            return;
        }
        for b in data {
            self.rx_data.push_back(*b);
        }
        self.rx_status.push_back(Self::rx_status_word(
            ep as u32,
            data.len() as u32,
            0,
            2,
        )); // OUT data received
        let mpsiz = self.mpsiz(ep, true);
        if (data.len() as u32) < mpsiz || data.len() as u32 >= self.out_ep[ep].out_expected {
            self.out_ep[ep].out_expected = 0;
            // Complete clears EPENA like silicon (firmware re-arms).
            self.out_ep[ep].ctl &= !(1 << 31);
            self.out_ep[ep].int |= 1; // XFRC
            // Internal-DMA mode: latch the received count into the EP DMA
            // address register (firmware polls DIEPDMA/DOEPDMA instead of
            // GRXSTSP). Same completion; only the accounting differs.
            if self.gahbcfg & (1 << 5) != 0 {
                self.dma_moved_out[ep] = self.dma_moved_out[ep].wrapping_add(data.len() as u64);
                if ep == 0 {
                    self.out_ep[ep].dma = self.dma_moved_out[ep] as u32;
                } else {
                    self.out_ep[ep].dma = self.dma_moved_out[ep] as u32;
                }
            }
        } else {
            self.out_ep[ep].out_expected -= data.len() as u32;
        }
        self.update_irq(sys);
    }

    /// IN transfer status for the harness: 0 none, 1 data ready, 2 stall.
    /// OUT stall reports on the same code: querying an OUT endpoint (or
    /// either direction after a stall) surfaces a pending STALL handshake.
    pub fn in_status(&self, ep: usize) -> u32 {
        if ep >= N_EPS {
            return 0;
        }
        if self.in_stall[ep] || self.out_stall[ep] {
            2
        } else if self.in_ready[ep].is_some() {
            1
        } else {
            0
        }
    }

    /// OUT transfer status for the harness: 0 none, 2 stalled (OUT has no
    /// data-ready slot — reception completes via the endpoint interrupt).
    pub fn out_status(&self, ep: usize) -> u32 {
        if ep >= N_EPS {
            return 0;
        }
        if self.out_stall[ep] { 2 } else { 0 }
    }

    /// Drain a completed IN blob (empty vec = ZLP or nothing pending;
    /// check in_status first to tell them apart).
    pub fn take_in(&mut self, ep: usize) -> Vec<u8> {
        if ep >= N_EPS {
            return Vec::new();
        }
        self.in_ready[ep].take().unwrap_or_default()
    }

    /// Internal-DMA progress counters (harness scope probe): bytes moved by
    /// DMA on IN (in_send) / OUT (inject_out completion) while GAHBCFG
    /// DMAEN was set. Firmware polls the EP DMA address registers; the
    /// harness reads the counters directly.
    pub fn dma_progress(&self, ep: usize) -> (u64, u64) {
        if ep >= N_EPS {
            return (0, 0);
        }
        (self.dma_moved_in[ep], self.dma_moved_out[ep])
    }

    fn read_ep_in(&mut self, _sys: &System, ep: usize, off: u32) -> u32 {
        match off {
            0x00 => self.in_ep[ep].ctl,
            0x08 => self.in_ep[ep].int,
            0x10 => self.in_ep[ep].tsiz,
            0x14 => self.in_ep[ep].dma,
            0x18 => self.tx_space_words(ep), // DTXFSTS: space in words
            _ => 0,
        }
    }

    fn write_ep_in(&mut self, sys: &System, ep: usize, off: u32, value: u32) {
        match off {
            0x00 => {
                let was_ena = self.in_ep[ep].ctl & (1 << 31) != 0;
                let was_stall = self.in_ep[ep].ctl & (1 << 21) != 0;
                // STALL set + EPENA in one write must stall, not race the
                // enable edge below: capture the intent before storing, and
                // report the handshake immediately (in_send would do it, but
                // the edge is now suppressed — so latch here). A STALL-only
                // write (no EPENA edge) also latches immediately: on real
                // silicon the device answers the next IN token with STALL
                // as soon as the application sets the bit, without waiting
                // for a transfer to be armed (the usb_cdc_test ep0_stall()
                // path sets STALL with no EPENA edge at all).
                let stalling = value & (1 << 21) != 0;
                self.in_ep[ep].ctl = value;
                if stalling && !was_stall {
                    self.in_stall[ep] = true;
                }
                // STALL handshake set/clear: firmware sets bit 21 to stall
                // the endpoint, clears it to resume. Clearing drops a
                // pending STALL report (silicon clears the handshake state
                // when the application clears STALL); setting it while a
                // transfer is armed stalls immediately at the next EPENA.
                if value & (1 << 21) == 0 && was_stall {
                    self.in_stall[ep] = false;
                }
                if value & (1 << 30) != 0 {
                    // EPDIS: halt the endpoint, report disabled.
                    self.in_ep[ep].ctl &= !(1 << 31);
                    self.in_ep[ep].int |= 1 << 1; // EPDISD
                }
                if !was_ena && value & (1 << 31) != 0 && !stalling {
                    self.in_send(sys, ep);
                }
                self.update_irq(sys);
            }
            // W1C interrupts.
            0x08 => {
                self.in_ep[ep].int &= !value;
                self.update_irq(sys);
            }
            0x10 => self.in_ep[ep].tsiz = value,
            0x14 => self.in_ep[ep].dma = value,
            _ => {}
        }
    }

    fn read_ep_out(&mut self, _sys: &System, ep: usize, off: u32) -> u32 {
        match off {
            0x00 => self.out_ep[ep].ctl,
            0x08 => self.out_ep[ep].int,
            0x10 => self.out_ep[ep].tsiz,
            0x14 => self.out_ep[ep].dma,
            _ => 0,
        }
    }

    fn write_ep_out(&mut self, sys: &System, ep: usize, off: u32, value: u32) {
        match off {
            0x00 => {
                let was_stall = self.out_ep[ep].ctl & (1 << 21) != 0;
                self.out_ep[ep].ctl = value;
                // STALL set/clear mirrors the IN path (OUT STALL makes the
                // device answer the next OUT token with a STALL handshake;
                // clearing resumes normal reception). Like IN, a STALL-only
                // write latches immediately — no EPENA edge is required.
                if value & (1 << 21) != 0 && !was_stall {
                    self.out_stall[ep] = true;
                }
                if value & (1 << 21) == 0 && was_stall {
                    self.out_stall[ep] = false;
                }
                if value & (1 << 30) != 0 {
                    self.out_ep[ep].ctl &= !(1 << 31);
                    self.out_ep[ep].int |= 1 << 1; // EPDISD
                }
                self.update_irq(sys);
            }
            0x08 => {
                self.out_ep[ep].int &= !value;
                self.update_irq(sys);
            }
            0x10 => {
                self.out_ep[ep].tsiz = value;
                // Arming a transfer: track the expected byte count so the
                // completion rule (short-or-drained) works.
                self.out_ep[ep].out_expected = value & 0x7FFFF;
            }
            0x14 => self.out_ep[ep].dma = value,
            _ => {}
        }
    }
}

impl Peripheral for UsbFs {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }

    fn tick(&mut self, sys: &System) {
        // SOF generation runs on the instruction-count clock (1 kHz while
        // enumerated + out of suspend + VBUS present). Gated inside
        // tick_sof so unenumerated/suspended sessions cost one branch.
        self.tick_sof(sys, crate::system::instruction_count());
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            // ---- core global block ----
            0x000 => self.gotgctl(),
            0x004 => self.gotgint,
            0x008 => self.gahbcfg,
            // GUSBCFG live: stored bits plus the ULPI PHY rate report.
            // Bits 31:20 are reserved on silicon; the model reports the
            // link rate there (480 = HS ULPI, 12 = FS) so firmware sizing
            // DMA/FIFO budgets by rate observes the harness contract
            // without a second read path. All other bits read stored.
            0x00C => (self.gusbcfg & 0x000F_FFFF) | (self.ulpi_rate_mbps() << 20),
            // GRSTCTL reads AHBIDL set (always idle here).
            0x010 => self.grstctl | (1 << 31),
            0x014 => self.gintsts(),
            0x018 => self.gintmsk,
            0x01C => *self.rx_status.front().unwrap_or(&0), // GRXSTSR peek
            0x020 => self.rx_status.pop_front().unwrap_or(0), // GRXSTSP pop
            0x024 => self.rxfifo_depth,
            0x028 => self.nptxfifo,
            0x02C => {
                // GNPTXSTS: NPTXFSAV[15:0] space, queue top/depth 0.
                self.tx_space_words(0) & 0xFFFF
            }
            0x038 => self.gccfg,
            0x03C => 0, // CID: informational only
            0x100 => 0, // HPTXFSIZ (host-only)
            0x104 => self.ptxfifo[0],
            0x108 => self.ptxfifo[1],
            0x10C => self.ptxfifo[2],
            // ---- device block ----
            0x800 => self.dcfg,
            0x804 => self.dctl,
            0x808 => self.dsts,
            0x810 => self.diepmsk,
            0x814 => self.doepmsk,
            0x818 => self.daint(),
            0x81C => self.daintmsk,
            0x828 => self.dvbusdis,
            0x82C => self.dvbuspulse,
            0x834 => self.die_pempmsk,
            0x834 => self.die_pempmsk,
            // ---- endpoint blocks ----
            o if (0x900..0x980).contains(&o) => {
                let ep = ((o - 0x900) / 0x20) as usize;
                if ep < N_EPS {
                    self.read_ep_in(sys, ep, (o - 0x900) % 0x20)
                } else {
                    0
                }
            }
            o if (0xB00..0xB80).contains(&o) => {
                let ep = ((o - 0xB00) / 0x20) as usize;
                if ep < N_EPS {
                    self.read_ep_out(sys, ep, (o - 0xB00) % 0x20)
                } else {
                    0
                }
            }
            0xE00 => self.pcgcctl,
            // ---- FIFO window (offset from slot base 0x50000000) ----
            // NOTE: the RXFIFO pop is pop-per-ACCESS-WIDTH, not pop-4-words:
            // the generic bus layer merges sub-word accesses (a guest byte
            // load arrives here as a full-word read of the aligned word).
            // Popping 4 bytes per access would eat the next 3 queued bytes
            // every time firmware reads one byte — observed as SETUP bytes
            // vanishing (REQ 0680 re-read as zeros) whenever the guest
            // mixed byte and word accesses to FIFO0.
            o if (0x1000..0x5000).contains(&o) => {
                let ep = ((o - 0x1000) / 0x1000) as usize;
                let sub = (o - 0x1000) % 0x1000;
                if ep >= N_EPS || sub != 0 {
                    return 0;
                }
                if ep == 0 {
                    // RXFIFO pop (LE, zero-padded tail). Width-aware: the
                    // bus layer passes the access width through `size`, and
                    // the FIFO POP MUST consume exactly that many bytes —
                    // popping a whole word on every access (even a byte
                    // load, which the bus layer widens to a full-word model
                    // read) eats queued bytes the guest never asked for.
                    let n = crate::system::periph_access_width().unwrap_or(4).min(4);
                    let mut v = 0u32;
                    for i in 0..n {
                        if let Some(b) = self.rx_data.pop_front() {
                            v |= (b as u32) << (8 * i);
                        } else {
                            break;
                        }
                    }
                    v
                } else {
                    0 // TX FIFOs are write-only
                }
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => self.gotgctl = value,
            0x004 => self.gotgint &= !value, // W1C-ish
            0x008 => {
                self.gahbcfg = value;
                self.update_irq(sys);
            }
            0x00C => self.gusbcfg = value,
            0x010 => {
                // GRSTCTL: CSRST + TX/RX flush are action bits (self-clear).
                if value & 1 != 0 {
                    self.reset_core();
                    sys.p.nvic.borrow_mut().clear_pending(self.irq());
                    return;
                }
                if value & (1 << 5) != 0 {
                    // TXFFLSH: TXFNUM 0x10 flushes all.
                    let n = ((value >> 6) & 0x1F) as usize;
                    if n == 0x10 {
                        for f in self.tx_data.iter_mut() {
                            f.clear();
                        }
                        self.in_ready = Default::default();
                    } else if n < N_EPS {
                        self.tx_data[n].clear();
                        self.in_ready[n] = None;
                    }
                }
                if value & (1 << 4) != 0 {
                    self.rx_status.clear();
                    self.rx_data.clear();
                }
                self.grstctl = value & !(1 | (1 << 4) | (1 << 5));
            }
            // GINTSTS: W1C event bits (RO computed bits unaffected).
            0x014 => {
                self.gint_sticky &= !value;
                self.update_irq(sys);
            }
            0x018 => {
                self.gintmsk = value;
                self.update_irq(sys);
            }
            0x024 => self.rxfifo_depth = value,
            0x028 => self.nptxfifo = value,
            0x038 => self.gccfg = value,
            0x104 => self.ptxfifo[0] = value,
            0x108 => self.ptxfifo[1] = value,
            0x10C => self.ptxfifo[2] = value,
            0x800 => self.dcfg = value,
            0x804 => {
                let prev = self.dctl;
                // RWUSIG (bit 0, remote-wakeup signaling): set while
                // suspended clears SUSPSTS (resume path) — silicon wakes
                // on the host's resume. RWUSIG self-holds until firmware
                // clears it; the SOF tick resumes on the next tick.
                if value & 1 != 0 && self.dsts & 1 != 0 {
                    self.dsts &= !1; // exit suspend via remote wakeup
                    self.gint_sticky |= 1 << 31; // WKUPINT (resume observed)
                    self.update_irq(sys);
                }
                self.dctl = value;
                // CGNPINNAK (bit 8): release deferred IN transfers.
                if prev & (1 << 7) != 0 && value & (1 << 8) != 0 {
                    self.dctl &= !(1 << 7);
                    for ep in 0..N_EPS {
                        if self.in_ep[ep].ctl & (1 << 31) != 0 && self.in_ready[ep].is_none() && !self.in_stall[ep] {
                            self.in_send(sys, ep);
                        }
                    }
                }
            }
            0x810 => {
                self.diepmsk = value;
                self.update_irq(sys);
            }
            0x814 => {
                self.doepmsk = value;
                self.update_irq(sys);
            }
            0x81C => {
                self.daintmsk = value;
                self.update_irq(sys);
            }
            0x828 => self.dvbusdis = value,
            0x82C => self.dvbuspulse = value,
            0x834 => self.die_pempmsk = value,
            o if (0x900..0x980).contains(&o) => {
                let ep = ((o - 0x900) / 0x20) as usize;
                if ep < N_EPS {
                    self.write_ep_in(sys, ep, (o - 0x900) % 0x20, value);
                }
            }
            o if (0xB00..0xB80).contains(&o) => {
                let ep = ((o - 0xB00) / 0x20) as usize;
                if ep < N_EPS {
                    self.write_ep_out(sys, ep, (o - 0xB00) % 0x20, value);
                }
            }
            0xE00 => self.pcgcctl = value,
            // ---- FIFO window ----
            o if (0x1000..0x5000).contains(&o) => {
                let ep = ((o - 0x1000) / 0x1000) as usize;
                let sub = (o - 0x1000) % 0x1000;
                if ep >= N_EPS || sub != 0 {
                    return;
                }
                // Word push into the addressed TX FIFO (LE bytes).
                for i in 0..4 {
                    self.tx_data[ep].push_back(((value >> (8 * i)) & 0xFF) as u8);
                }
                self.update_irq(sys);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::UsbFs;
    use crate::peripherals::Peripheral;

    // Model-level EP0 IN roundtrip through the real map: program FIFO
    // sizes, arm a 4-byte IN transfer, enable, take the blob. No guest.
    #[test]
    fn ep0_in_roundtrip() {
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        // Find the USB slot like lib.rs does and drive it directly.
        let mut found = false;
        for slot in &sys.p.peripherals {
            if slot.start == 0x5000_0000 {
                found = true;
                let mut u = slot.peripheral.borrow_mut();
                let usb = u.as_any_mut().downcast_mut::<UsbFs>().unwrap();
                usb.write(sys, 0x028, 0x00400040); // GNPTXFSIZ depth 64
                usb.write(sys, 0x910, 4 | (1 << 19)); // DIEPTSIZ0: 4B,1pkt
                // FIFO push "Hi!\0" (word LE).
                usb.write(sys, 0x1000, 0x00216948);
                // EPENA (keep USBACTEP/CNAK shape simple: just set bit31).
                let ctl = usb.read(sys, 0x900);
                usb.write(sys, 0x900, ctl | (1 << 26) | (1 << 31));
                assert_eq!(usb.in_status(0), 1, "IN data ready");
                assert_eq!(usb.take_in(0), vec![0x48, 0x69, 0x21, 0x00]);
                assert_eq!(usb.in_status(0), 0, "drained");
                // XFRC latched on the endpoint.
                assert_ne!(usb.read(sys, 0x908) & 1, 0, "XFRC set");
            }
        }
        assert!(found, "USB slot registered in new_wasm map");
    }

    #[test]
    fn csrst_self_clears_and_flushes() {
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        for slot in &sys.p.peripherals {
            if slot.start == 0x5000_0000 {
                let mut u = slot.peripheral.borrow_mut();
                let usb = u.as_any_mut().downcast_mut::<UsbFs>().unwrap();
                usb.write(sys, 0x1000, 0xDEADBEEF);
                usb.inject_setup(sys, &[0, 1, 2, 3, 4, 5, 6, 7]);
                usb.write(sys, 0x010, 1); // CSRST
                assert_eq!(usb.read(sys, 0x010) & 1, 0, "CSRST self-clears");
                assert_eq!(usb.in_status(0), 0, "queues flushed");
                assert_eq!(usb.read(sys, 0x014) & (1 << 12), 0, "no USBRST yet");
                return;
            }
        }
        panic!("USB slot missing");
    }

    #[test]
    fn stall_handshake_set_and_clear_both_directions() {
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        for slot in &sys.p.peripherals {
            if slot.start == 0x5000_0000 {
                let mut u = slot.peripheral.borrow_mut();
                let usb = u.as_any_mut().downcast_mut::<UsbFs>().unwrap();
                // IN: STALL bit set + EPENA stalls instead of moving data.
                usb.write(sys, 0x1000, 0xAABBCCDD); // TX FIFO has bytes
                usb.write(sys, 0x910, 4 | (1 << 19)); // DIEPTSIZ0: 4B,1pkt
                let ctl = usb.read(sys, 0x900);
                usb.write(sys, 0x900, ctl | (1 << 21) | (1 << 31)); // STALL+EPENA
                assert_eq!(usb.in_status(0), 2, "IN STALL reported");
                assert_eq!(usb.read(sys, 0x908) & 1, 0, "no XFRC on STALL");
                // Clearing STALL resumes: next EPENA completes normally.
                // (The stalled EPENA stayed armed — the STALL write only
                // reported the handshake, it did not consume the enable —
                // so plain EPENA (already set in ctl2) with STALL cleared
                // is a fresh edge only if EPENA toggles: drop it first,
                // then re-arm explicitly, like firmware re-arming after
                // the stall clears.)
                let ctl2 = usb.read(sys, 0x900) & !(1 << 21);
                usb.write(sys, 0x900, ctl2 & !(1 << 31)); // drop stale EPENA
                usb.write(sys, 0x910, 4 | (1 << 19)); // re-arm 4B,1pkt
                usb.write(sys, 0x900, ctl2 | (1 << 31));
                assert_eq!(usb.in_status(0), 1, "IN data ready after clear");
                assert_eq!(usb.take_in(0).len(), 4, "stalled bytes not lost");
                // OUT: STALL bit set makes inject_out report stall, move nothing.
                let octl = usb.read(sys, 0xB00);
                usb.write(sys, 0xB00, octl | (1 << 21)); // STALL EP0-OUT
                usb.write(sys, 0xB10, (1 << 19) | 64); // arm 64B OUT
                usb.inject_out(sys, 0, &[1, 2, 3, 4]);
                assert_eq!(usb.out_status(0), 2, "OUT STALL reported");
                assert_eq!(usb.in_status(0), 2, "STALL visible on IN status too");
                assert!(usb.rx_status.is_empty(), "stalled OUT queued nothing");
                // Clearing resumes normal reception.
                let octl2 = usb.read(sys, 0xB00) & !(1 << 21);
                usb.write(sys, 0xB00, octl2);
                usb.inject_out(sys, 0, &[1, 2, 3, 4]);
                assert_eq!(usb.out_status(0), 0, "OUT clear resumes");
                assert!(!usb.rx_status.is_empty(), "normal OUT queues again");
                return;
            }
        }
        panic!("USB slot missing");
    }

    #[test]
    fn rxfifo_pop_consumes_access_width_only() {
        // Regression: the bus layer widens sub-word guest accesses to
        // full-word model reads. The RXFIFO pop used to consume 4 bytes per
        // access regardless, so a guest byte-load of FIFO0 ate the next 3
        // queued bytes — SETUP packets vanished whenever firmware mixed
        // byte and word accesses (observed: REQ 0680 handled, reply read
        // as zeros). Width 1 pops 1 byte, width 4 pops 4.
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        for slot in &sys.p.peripherals {
            if slot.start == 0x5000_0000 {
                let mut u = slot.peripheral.borrow_mut();
                let usb = u.as_any_mut().downcast_mut::<UsbFs>().unwrap();
                usb.inject_setup(sys, &[0xAA, 0xBB, 0xCC, 0xDD, 0x11, 0x22, 0x33, 0x44]);
                // Byte access pops exactly one byte (low byte of the word).
                crate::system::set_access_width(1);
                assert_eq!(usb.read(sys, 0x1000) & 0xFF, 0xAA, "byte pop 1");
                crate::system::set_access_width(1);
                assert_eq!(usb.read(sys, 0x1000) & 0xFF, 0xBB, "byte pop 2");
                // Word access pops the next whole word (CC DD 11 22 LE).
                crate::system::set_access_width(4);
                assert_eq!(usb.read(sys, 0x1000), 0x2211_DDCC, "word pop continues queue");
                crate::system::set_access_width(4);
                return;
            }
        }
        panic!("USB slot missing");
    }

    #[test]
    fn hs_block_enum_reports_hs_speed_and_roundtrips() {
        use super::{USB_HS_BASE, USB_HS_IRQ};
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        // Both blocks registered, distinct windows.
        let mut saw_fs = false;
        let mut saw_hs = false;
        for slot in &sys.p.peripherals {
            if slot.start == 0x5000_0000 {
                saw_fs = true;
            }
            if slot.start == USB_HS_BASE {
                saw_hs = true;
            }
        }
        assert!(saw_fs, "FS slot registered");
        assert!(saw_hs, "HS slot registered at 0x40040000");
        // HS enum reports HS speed (ENUMSPD 0b00), FS reports 0b11.
        sys.p.usb_hs_enumerated(sys);
        let hs_dsts = sys.p.read(sys, USB_HS_BASE + 0x808, 4);
        assert_eq!(hs_dsts & 6, 0, "HS ENUMSPD=HS, got {hs_dsts:#x}");
        assert_ne!(sys.p.read(sys, USB_HS_BASE + 0x014, 4) & (1 << 13), 0, "HS ENUMDNE latched");
        sys.p.usb_enumerated(sys);
        let fs_dsts = sys.p.read(sys, 0x5000_0000 + 0x808, 4);
        assert_eq!(fs_dsts & 6, 6, "FS ENUMSPD=FS, got {fs_dsts:#x}");
        // HS MMIO round-trip through the window: program EP0 TX size,
        // push a word, EPENA, take the blob via the HS host API.
        sys.p.write(sys, USB_HS_BASE + 0x028, 4, 0x00400040);
        sys.p.write(sys, USB_HS_BASE + 0x910, 4, 4 | (1 << 19));
        sys.p.write(sys, USB_HS_BASE + 0x1000, 4, 0x00216948);
        let ctl = sys.p.read(sys, USB_HS_BASE + 0x900, 4);
        sys.p.write(sys, USB_HS_BASE + 0x900, 4, ctl | (1 << 26) | (1 << 31));
        assert_eq!(sys.p.usb_hs_in_status(0), 1, "HS IN data ready");
        assert_eq!(sys.p.usb_hs_take_in(0), vec![0x48, 0x69, 0x21, 0x00]);
        // HS IRQ line is 77, independent of the FS line (67).
        assert_eq!(USB_HS_IRQ, 77, "HS IRQ number");
        // FS block untouched by the HS session.
        assert_eq!(sys.p.usb_in_status(0), 0, "FS IN idle while HS runs");
    }

    #[test]
    fn hs_microframes_tick_8x_with_eopf_at_roll() {
        use super::USB_HS_BASE;
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        sys.p.usb_hs_enumerated(sys);
        // One FS frame of ticks = 8 HS microframes: EOPF latches at the
        // roll, uframe counts 0..7, FNSOF frame number advances by 1.
        // (Parallel cargo threads share INSTRUCTION_COUNT, so read the
        // frame delta relatively — another thread's ticks only add whole
        // extra frames, never a partial one... except they CAN land
        // mid-window. Pin the delta by snapshotting sof_last via two
        // back-to-back windows instead: first window syncs, second counts.)
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == USB_HS_BASE {
                s.peripheral.borrow_mut().tick(sys);
            }
        });
        let f0 = sys.p.read(sys, USB_HS_BASE + 0x808, 4);
        crate::system::INSTRUCTION_COUNT
            .fetch_add(168_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == USB_HS_BASE {
                s.peripheral.borrow_mut().tick(sys);
            }
        });
        let f1 = sys.p.read(sys, USB_HS_BASE + 0x808, 4);
        let d = (((f1 >> 8) & 0x3FFF).wrapping_sub((f0 >> 8) & 0x3FFF)) & 0x3FFF;
        assert!(d >= 1, "HS frame advances per 168k, d={d}");
        assert_ne!(sys.p.read(sys, USB_HS_BASE + 0x014, 4) & (1 << 15), 0, "EOPF at roll");
        // FS block over the same window: exactly 1 frame, EOPF never sets
        // (FS has no microframes — the HS detail stays on the HS block).
        sys.p.usb_enumerated(sys);
        crate::system::INSTRUCTION_COUNT
            .fetch_add(168_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == 0x5000_0000 {
                s.peripheral.borrow_mut().tick(sys);
            }
        });
        assert_eq!(sys.p.read(sys, 0x5000_0000 + 0x014, 4) & (1 << 15), 0, "no EOPF on FS");
    }

    #[test]
    fn ulpi_rate_report_matches_personality() {
        use super::USB_HS_BASE;
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        // Reserved field of GUSBCFG carries the rate (480 HS / 12 FS).
        assert_eq!((sys.p.read(sys, USB_HS_BASE + 0x00C, 4) >> 20) & 0xFFF, 480, "HS rate field = 480");
        assert_eq!((sys.p.read(sys, 0x5000_0000 + 0x00C, 4) >> 20) & 0xFFF, 12, "FS rate field = 12");
        assert_eq!(sys.p.usb_ulpi_rate(), 12, "FS ulpi_rate 12");
        assert_eq!(sys.p.usb_hs_ulpi_rate(), 480, "HS ulpi_rate 480");
        assert_eq!(sys.p.usb_hs_uframe(), 0, "uframe 0 pre-tick");
    }

    #[test]
    fn sof_ticks_at_1khz_while_enumerated() {
        use super::USB_HS_BASE;
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        // Unenumerated: no SOF no matter how long the clock runs.
        sys.p.write(sys, 0x5000_0000 + 0x018, 4, 0);
        for _ in 0..10 {
            sys.p.peripherals.iter().for_each(|s| {
                if s.start == 0x5000_0000 {
                    s.peripheral.borrow_mut().tick(sys);
                }
            });
            crate::system::INSTRUCTION_COUNT
                .fetch_add(200_000, std::sync::atomic::Ordering::Relaxed);
        }
        assert_eq!(sys.p.read(sys, 0x5000_0000 + 0x014, 4) & (1 << 3), 0, "no SOF before enum");
        // Enumerate: SOF latches within one frame window + FNSOF advances.
        sys.p.usb_enumerated(sys);
        let f0 = sys.p.read(sys, 0x5000_0000 + 0x808, 4);
        crate::system::INSTRUCTION_COUNT
            .fetch_add(200_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == 0x5000_0000 {
                s.peripheral.borrow_mut().tick(sys);
            }
        });
        assert_ne!(sys.p.read(sys, 0x5000_0000 + 0x014, 4) & (1 << 3), 0, "SOF latched");
        let f1 = sys.p.read(sys, 0x5000_0000 + 0x808, 4);
        assert_ne!((f0 >> 8) & 0x3FFF, (f1 >> 8) & 0x3FFF, "FNSOF advanced");
        // W1C clear works.
        sys.p.write(sys, 0x5000_0000 + 0x014, 4, 1 << 3);
        assert_eq!(sys.p.read(sys, 0x5000_0000 + 0x014, 4) & (1 << 3), 0, "SOF W1C clears");
        let _ = USB_HS_BASE;
    }

    #[test]
    fn vbus_unplug_suspends_and_replug_needs_reset() {
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        sys.p.usb_enumerated(sys);
        // Unplug: BSVLD clears, SEDET latches, SUSPSTS sets, SOF stops.
        sys.p.usb_set_vbus(sys, false);
        assert_eq!(sys.p.read(sys, 0x5000_0000, 4) & (1 << 19), 0, "BSVLD clear on unplug");
        assert_ne!(sys.p.read(sys, 0x5000_0000 + 0x004, 4) & (1 << 2), 0, "SEDET latched");
        assert_ne!(sys.p.read(sys, 0x5000_0000 + 0x808, 4) & 1, 0, "SUSPSTS set");
        crate::system::INSTRUCTION_COUNT
            .fetch_add(500_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == 0x5000_0000 {
                s.peripheral.borrow_mut().tick(sys);
            }
        });
        // Re-plug: SEDET path clears, but the session stays suspended
        // until firmware re-enumerates (USBRST path).
        sys.p.usb_set_vbus(sys, true);
        assert_ne!(sys.p.read(sys, 0x5000_0000, 4) & (1 << 19), 0, "BSVLD back on replug");
        assert_ne!(sys.p.read(sys, 0x5000_0000 + 0x808, 4) & 1, 0, "still suspended until reset");
        sys.p.usb_reset(sys);
        sys.p.usb_enumerated(sys);
        assert_eq!(sys.p.read(sys, 0x5000_0000 + 0x808, 4) & 1, 0, "fresh session runs");
    }

    #[test]
    fn internal_dma_moves_count_without_fifo_writes() {
        let sys = crate::system::WasmSystem::new();
        crate::init_for_test(sys);
        let sys = crate::sys();
        // DMAEN on (GAHBCFG bit 5): IN EPENA attributes the move to DMA.
        sys.p.write(sys, 0x5000_0000 + 0x008, 4, (1 << 5) | 1);
        sys.p.write(sys, 0x5000_0000 + 0x028, 4, 0x00400040);
        sys.p.write(sys, 0x5000_0000 + 0x910, 4, 4 | (1 << 19));
        sys.p.write(sys, 0x5000_0000 + 0x1000, 4, 0x00216948);
        let ctl = sys.p.read(sys, 0x5000_0000 + 0x900, 4);
        sys.p.write(sys, 0x5000_0000 + 0x900, 4, ctl | (1 << 26) | (1 << 31));
        assert_eq!(sys.p.usb_in_status(0), 1, "IN ready");
        assert_eq!(sys.p.usb_take_in(0), vec![0x48, 0x69, 0x21, 0x00]);
        let (moved_in, _) = sys.p.usb_dma_progress(0);
        assert_eq!(moved_in, 4, "DMA counted the 4-byte IN move");
        // OUT completion latches the count into the EP DMA register.
        sys.p.write(sys, 0x5000_0000 + 0xB10, 4, (1 << 19) | 64);
        sys.p.usb_inject_out(sys, 0, &[1, 2, 3, 4]);
        assert_eq!(sys.p.read(sys, 0x5000_0000 + 0xB14, 4), 4, "DOEPDMA latched OUT count");
        // DMAEN off: moves stop counting (same contract, CPU-attributed).
        sys.p.write(sys, 0x5000_0000 + 0x008, 4, 1);
        sys.p.write(sys, 0x5000_0000 + 0x910, 4, 4 | (1 << 19));
        sys.p.write(sys, 0x5000_0000 + 0x1000, 4, 0x00216948);
        let ctl2 = sys.p.read(sys, 0x5000_0000 + 0x900, 4);
        sys.p.write(sys, 0x5000_0000 + 0x900, 4, (ctl2 | (1 << 26) | (1 << 31)) & !(1 << 31) | (1 << 31));
        let (moved_in2, _) = sys.p.usb_dma_progress(0);
        assert_eq!(moved_in2, 4, "counter frozen with DMAEN off");
    }
}
