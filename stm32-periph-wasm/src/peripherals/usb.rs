use crate::system::System;
use std::collections::VecDeque;
use super::Peripheral;

/// USB OTG FS device-mode model (combined 0x50000000 block: global +
/// device + EP regs + PWRCLK + FIFO window to 0x50005000). Host mode and
/// OTG_HS are out of scope (their SVD entries stay dropped → benign 0).
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
    in_stall: [bool; N_EPS],            // STALL handshake pending
}

impl Default for UsbFs {
    fn default() -> Self {
        let mut u = Self {
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
        } else {
            None
        }
    }

    /// Shared reset (CSRST + harness usb_reset): FIFOs/queues drained,
    /// address cleared, EPs back to reset (EP0 active).
    fn reset_core(&mut self) {
        let fresh = Self::default();
        // Keep nothing across reset: a fresh session starts unconfigured.
        *self = fresh;
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
    /// IEPINT/OEPINT/CMOD(device=1).
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
            sys.p.nvic.borrow_mut().set_intr_pending(USB_IRQ);
        }
    }

    /// Complete an IN transfer synchronously at EPENA (whole-blob: move up
    /// to XFRSIZ bytes into the ready slot, raise XFRC). GNPINNAK-defers
    /// and STALL handshakes handled; NAK state is otherwise lenient.
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
        sys.p.nvic.borrow_mut().clear_pending(USB_IRQ);
        self.update_irq(sys);
    }

    /// Enumeration done at full speed: ENUMDNE + DSTS speed (0b11 = FS).
    pub fn host_enumerated(&mut self, sys: &System) {
        self.gint_sticky |= 1 << 13; // ENUMDNE
        self.dsts = (self.dsts & !6) | 6;
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
    /// when the armed XFRSIZ drains, raising XFRC like silicon).
    pub fn inject_out(&mut self, sys: &System, ep: usize, data: &[u8]) {
        if ep >= N_EPS {
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
        } else {
            self.out_ep[ep].out_expected -= data.len() as u32;
        }
        self.update_irq(sys);
    }

    /// IN transfer status for the harness: 0 none, 1 data ready, 2 stall.
    pub fn in_status(&self, ep: usize) -> u32 {
        if ep >= N_EPS {
            return 0;
        }
        if self.in_stall[ep] {
            2
        } else if self.in_ready[ep].is_some() {
            1
        } else {
            0
        }
    }

    /// Drain a completed IN blob (empty vec = ZLP or nothing pending;
    /// check in_status first to tell them apart).
    pub fn take_in(&mut self, ep: usize) -> Vec<u8> {
        if ep >= N_EPS {
            return Vec::new();
        }
        self.in_ready[ep].take().unwrap_or_default()
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
                self.in_ep[ep].ctl = value;
                if value & (1 << 30) != 0 {
                    // EPDIS: halt the endpoint, report disabled.
                    self.in_ep[ep].ctl &= !(1 << 31);
                    self.in_ep[ep].int |= 1 << 1; // EPDISD
                }
                if !was_ena && value & (1 << 31) != 0 {
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
                self.out_ep[ep].ctl = value;
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

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            // ---- core global block ----
            0x000 => self.gotgctl,
            0x004 => self.gotgint,
            0x008 => self.gahbcfg,
            0x00C => self.gusbcfg,
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
            o if (0x1000..0x5000).contains(&o) => {
                let ep = ((o - 0x1000) / 0x1000) as usize;
                let sub = (o - 0x1000) % 0x1000;
                if ep >= N_EPS || sub != 0 {
                    return 0;
                }
                if ep == 0 {
                    // RXFIFO pop (word-wise, LE, zero-padded tail).
                    let mut v = 0u32;
                    for i in 0..4 {
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
                    sys.p.nvic.borrow_mut().clear_pending(USB_IRQ);
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
}
