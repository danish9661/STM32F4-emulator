use crate::system::{CanFrame, System, can_restage, can_stage_tx, can_take_staged};
use super::Peripheral;

#[derive(Clone, Copy)]
struct Mailbox {
    tir: u32, tdtr: u32, tdlr: u32, tdhr: u32,
}

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    /// FDCAN bit-timing extension (model registers past the bxCAN map, in
    /// the FD window guard band — real FD needs an FDCAN block; this is
    /// the documented emulation surface):
    /// - NBTP (nominal bit timing, FDCAN_NBTP layout): BRP[7:0] + TSEG1 +
    ///   TSEG2 + SJW. Programs the arbitration-phase bit rate; the model
    ///   converts it to a virtual-instruction cost per classic byte and
    ///   per arbitration byte of FD frames (arbitration always runs at
    ///   the nominal rate, like silicon).
    /// - DBTP (data bit timing, FDCAN_DBTP layout): same fields for the
    ///   FD data phase; BRS frames pay the data rate for payload bytes.
    /// - TEST/PSR-style status is folded into ESR (no extra regs).
    /// Reset: nominal 500 kbit/s, data 2 Mbit/s equivalents (see
    /// `fd_timing_cost`); a zero BRP field means "unprogrammed" and keeps
    /// the reset cost (silicon would not transmit; the model stays
    /// lenient and reports the reset cost — documented, not silent).
    nbtp: u32,
    dbtp: u32,
    tx: [Mailbox; 3],
    // 3 mailboxes per FIFO: rx[fif*3 + slot] (RIR at 0x1B0/0x1C0/0x1D0 for
    // FIFO0, 0x1E0/0x1F0/0x200 for FIFO1 — the real F407 map).
    rx: [Mailbox; 6],
    /// CAN FD payload window: 6 slots x 64 bytes + valid lengths. Filled
    /// on FD delivery, cleared on classic delivery, drained by RFOM like
    /// the classic FIFO (release shifts the window with the slots).
    fd_rx: [[u8; 64]; 6],
    fd_len: [u8; 6],
    fmr: u32, fm1r: u32, fs1r: u32, ffa1r: u32, fa1r: u32,
    filter: [u32; 56],
    irq_base: i32,
    node: u8,
    // CAN2 shares the 28 global filter banks via its own register block,
    // accessing banks 14..27 (real F407 behavior).
    filter_off: usize,
}

impl Can {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let (irq_base, node, filter_off) = match name {
            "CAN1" => (19, 1u8, 0usize),
            "CAN2" => (63, 2u8, 14usize),
            _ => return None,
        };
        Some(Box::new(Can {
            mcr: 0x0001_0002, msr: 0x0000_0C02, tsr: 0x1C00_0000,
            irq_base, node, filter_off,
            ..Self::default()
        }))
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let base = self.irq_base;
        // TX (TMEIE bit 0) — fires when a mailbox completes (TME 16..18,
        // historically also CODE bits 24..26 set at request time)
        if self.ier & 0x01 != 0 && self.tsr & 0x0707_0000 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base);
        }
        // RX0 (FMPIE0 bit 1, FFIE0 bit 2, FOVIE0 bit 3)
        if self.ier & 0x0E != 0 && self.rf0r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 1);
        }
        // RX1 (FMPIE1 bit 4, FFIE1 bit 5, FOVIE1 bit 6)
        if self.ier & 0x70 != 0 && self.rf1r & 0x03 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 2);
        }
        // SCE (EWGIE bit 7, EPVIE bit 8, BOFIE bit 9, LECIE bit 10, ERRIE bit 11)
        if self.ier & 0xF80 != 0 && self.esr != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(base + 3);
        }
    }

    /// Stage a TX-requested mailbox onto the shared bus. Completion (TSR
    /// TXOK|TME|RQCP) is deferred to bus arbitration so two nodes can
    /// contend in the same round.
    fn stage_mailbox(&mut self, i: usize, sys: &System) {
        let m = self.tx[i];
        self.tsr |= 1 << i;                        // TXRQ — stays set (historic behavior)
        self.tsr &= !(1 << (8 + i));               // clear stale TXOK
        self.tsr &= !(1 << (16 + i));              // clear TME (mailbox busy)
        let rqcp = (self.tsr >> 26) & 7;
        self.tsr = (self.tsr & !(7 << 26)) | ((rqcp & !(1 << i)) << 26);
        let ext = m.tir & (1 << 2) != 0;
        let id = if ext { (m.tir >> 3) & 0x1FFF_FFFF } else { (m.tir >> 21) & 0x7FF };
        let b = [
            (m.tdlr & 0xFF) as u8, ((m.tdlr >> 8) & 0xFF) as u8,
            ((m.tdlr >> 16) & 0xFF) as u8, ((m.tdlr >> 24) & 0xFF) as u8,
            (m.tdhr & 0xFF) as u8, ((m.tdhr >> 8) & 0xFF) as u8,
            ((m.tdhr >> 16) & 0xFF) as u8, ((m.tdhr >> 24) & 0xFF) as u8,
        ];
        can_stage_tx(CanFrame {
            node: self.node,
            mailbox: i,
            id,
            ext,
            rtr: m.tir & (1 << 1) != 0,
            dlc: (m.tdtr & 0xF) as u8,
            data: b,
            loopback: self.btr & (1 << 30) != 0,
            // Classic bxCAN path: never FD (FD frames enter via
            // can_inject_fd / the FD mailbox window below).
            fd: false,
            fd_len: 0,
            fd_data: [0; 64],
        });
        self.fire_interrupts(sys);
    }

    /// Mark a mailbox transmission complete (won arbitration).
    fn complete_tx(&mut self, sys: &System, i: usize) {
        if i >= 3 { return; }
        self.tsr |= 1 << (8 + i);                  // TXOK
        self.tsr |= 1 << (16 + i);                 // TME (mailbox empty again)
        self.tsr |= 1 << (31 - 4 * i);             // RQCP (bits 31/27/23)
        self.note_success(); // clean TX recovers TEC toward 0, clears LEC
        self.fire_interrupts(sys);
    }

    /// Deliver a won frame into the RX FIFO chosen by the first passing
    /// filter bank. Returns false if no active filter matched.
    /// CAN FD frames (f.fd) share the filter + FIFO path (arbitration on
    /// the ID is identical); the first 8 payload bytes land in the classic
    /// mailbox words (guest reads them without knowing FD), the full
    /// payload in the FD window, and TDTR carries DLC + FDF (bit 16).
    fn receive_frame(&mut self, sys: &System, f: &CanFrame) -> bool {
        let mut fifo = None;
        for bank in 0..14usize {
            if self.filter_pass(f, bank) {
                fifo = Some(if (self.ffa1r >> (bank + self.filter_off)) & 1 != 0 { 1 } else { 0 });
                break;
            }
        }
        let Some(fif) = fifo else { return false };
        let r = if fif == 0 { &mut self.rf0r } else { &mut self.rf1r };
        let fmp = *r & 0x3;
        if fmp >= 3 {
            *r |= 1 << 3;                          // FOVR — FIFO full, drop newest
            return true;
        }
        let slot = fmp as usize;
        let mx = &mut self.rx[fif * 3 + slot];
        mx.tir = if f.ext {
            (f.id << 3) | (1 << 2) | (if f.rtr { 1 << 1 } else { 0 })
        } else {
            (f.id << 21) | (if f.rtr { 1 << 1 } else { 0 })
        };
        mx.tdtr = f.dlc as u32 | (if f.fd { 1 << 16 } else { 0 }); // FDF
        if f.fd {
            // First 8 payload bytes in the classic words (guest-compatible),
            // full 64 in the FD window (slot-indexed, see can_fd_read).
            for i in 0..8 {
                let b = f.fd_data.get(i).copied().unwrap_or(0) as u32;
                if i < 4 {
                    mx.tdlr |= b << (8 * i);
                } else {
                    mx.tdhr |= b << (8 * (i - 4));
                }
            }
            let base = fif * 3 + slot;
            if let Some(dst) = self.fd_rx.get_mut(base) {
                dst[..f.fd_len as usize].copy_from_slice(&f.fd_data[..f.fd_len as usize]);
                self.fd_len[base] = f.fd_len;
            }
        } else {
            mx.tdlr = f.data[0] as u32 | (f.data[1] as u32) << 8
                | (f.data[2] as u32) << 16 | (f.data[3] as u32) << 24;
            mx.tdhr = f.data[4] as u32 | (f.data[5] as u32) << 8
                | (f.data[6] as u32) << 16 | (f.data[7] as u32) << 24;
            let base = fif * 3 + slot;
            if let Some(dst) = self.fd_rx.get_mut(base) {
                dst.fill(0);
                self.fd_len[base] = 0;
            }
        }
        *r = (*r & !0x3) | (fmp + 1);              // FMP++
        self.fire_interrupts(sys);
        true
    }

    /// Read one byte of the FD payload window for (`fifo`, `slot`, `idx`):
    /// 2 FIFOs x 3 slots x 64 bytes. Classic frames read 0 past byte 8
    /// (their window was cleared on delivery).
    pub fn fd_byte(&self, fifo: usize, slot: usize, idx: usize) -> u8 {
        if fifo > 1 || slot > 2 || idx >= 64 {
            return 0;
        }
        self.fd_rx[fifo * 3 + slot][idx]
    }

    /// Valid FD payload length for (`fifo`, `slot`) — 0 for classic frames.
    pub fn fd_payload_len(&self, fifo: usize, slot: usize) -> u8 {
        if fifo > 1 || slot > 2 {
            return 0;
        }
        self.fd_len[fifo * 3 + slot]
    }

    /// FDCAN bit-timing cost model (virtual instructions per byte).
    /// Nominal rate from NBTP (FDCAN layout: BRP[7:0], TSEG1[15:8],
    /// TSEG2[22:16], SJW[26:24]); data rate from DBTP (same layout).
    /// bit_time = (BRP+1) * (1 + TSEG1 + TSEG2) virtual clocks per bit;
    /// cost per byte = 8 * bit_time. A zero BRP (unprogrammed) keeps the
    /// reset cost (nominal 500 kbit/s = 336 inst/bit, data 2 Mbit/s =
    /// 84 inst/bit on the 168 MHz clock). Returns (nominal_per_byte,
    /// data_per_byte).
    pub fn fd_timing_cost(&self) -> (u64, u64) {
        fn per_byte(reg: u32, reset_per_bit: u64) -> u64 {
            let brp = (reg & 0xFF) as u64;
            if brp == 0 && reg & 0xFFFF00 == 0 {
                return reset_per_bit * 8;
            }
            let tseg1 = ((reg >> 8) & 0xFF) as u64;
            let tseg2 = ((reg >> 16) & 0x7F) as u64;
            let tq = 1 + tseg1 + tseg2;
            (brp + 1) * tq.max(1) * 8
        }
        (per_byte(self.nbtp, 336), per_byte(self.dbtp, 84))
    }

    /// Wire-time cost of one frame in virtual instructions: arbitration
    /// bytes (11/29-bit ID + control ≈ 8 bytes equivalent) at the nominal
    /// rate; FD payload bytes at the data rate iff BRS else nominal;
    /// classic payload at nominal. Matches silicon's two-rate split —
    /// a BRS frame with a fast data phase costs strictly less than the
    /// same bytes at nominal (the mock pins the ratio).
    pub fn fd_frame_cost(&self, fd: bool, brs: bool, payload_bytes: usize) -> u64 {
        let (nom, data) = self.fd_timing_cost();
        let arb = 8 * nom;
        if !fd {
            return arb + payload_bytes as u64 * nom;
        }
        let rate = if brs { data } else { nom };
        arb + payload_bytes as u64 * rate
    }

    /// Error counters + bus-off state (classic bxCAN, observable subset):
    /// - TEC/REC live in ESR bits 23:16 / 31:24. The model counts TX
    ///   completions down toward 0 (errors would count up — the harness
    ///   drives errors via `can_note_error`, see below); a quiet bus
    ///   reads 0/0 like silicon after reset.
    /// - BOFF (ESR bit 2): set when TEC exceeds 255 (classic bus-off
    ///   entry); cleared when the harness recovers the bus
    ///   (`can_note_error` with recover=true models 128x11 recessive
    ///   bits). EPVF (bit 1, TEC/REC > 127) and EWGF (bit 0, > 96)
    ///   derive live from the counters like silicon.
    /// - LEC (bits 6:4): last error code, latched by `can_note_error`
    ///   (0 none, 1 stuff, 2 form, 3 ack, 4 bit-recessive, 5 bit-dominant,
    ///   6 CRC, 7 custom). Cleared on a clean completion or by writing
    ///   ESR (silicon: LEC clears on read after a good frame; the write
    ///   path here mirrors the mock's needs — documented, not silent).
    pub fn note_error(&mut self, sys: &System, lec: u8, recover: bool) {
        if recover {
            self.esr &= !((0xFF << 16) | (0xFF << 24) | (1 << 2) | (7 << 4));
            self.msr &= !(1 << 2); // clear ERRI-adjacent latched state view
            self.fire_interrupts(sys);
            return;
        }
        // One error event: TEC +8 (transmit error), LEC latched.
        let tec = ((self.esr >> 16) & 0xFF).saturating_add(8).min(256);
        if tec >= 256 {
            self.esr |= 1 << 2; // BOFF
            self.esr = (self.esr & !(0xFF << 16)) | (0xFF << 16);
        } else {
            self.esr = (self.esr & !(0xFF << 16)) | ((tec & 0xFF) << 16);
        }
        self.esr = (self.esr & !(7 << 4)) | ((lec as u32 & 7) << 4);
        self.fire_interrupts(sys);
    }

    /// Successful TX completion decrements TEC toward 0 (classic
    /// error-passive recovery direction) and clears a latched LEC.
    fn note_success(&mut self) {
        let tec = (self.esr >> 16) & 0xFF;
        if tec > 0 {
            let nt = tec.saturating_sub(1);
            self.esr = (self.esr & !(0xFF << 16)) | (nt << 16);
            if nt <= 127 {
                self.esr &= !(1 << 2); // leave bus-off below threshold
            }
        }
    }

    /// Live ESR view: EPVF/EWGF derived from TEC/REC thresholds (silicon
    /// computes them continuously; the stored ESR holds TEC/REC/LEC/BOFF).
    fn esr_live(&self) -> u32 {
        let mut v = self.esr;
        let tec = (v >> 16) & 0xFF;
        let rec = (v >> 24) & 0xFF;
        if tec > 96 || rec > 96 { v |= 1; } else { v &= !1; } // EWGF
        if tec > 127 || rec > 127 { v |= 1 << 1; } else { v &= !(1 << 1); } // EPVF
        v
    }

    /// Test whether `f` passes any filter bank of this node. Filter layout
    /// follows the real F407: 28 global banks (CAN2 uses 14..27), fa1r
    /// enables, fm1r mask/list, fs1r 32/16-bit; masks live in word 2b+1.
    fn filter_pass(&self, f: &CanFrame, bank_local: usize) -> bool {
        let g = bank_local + self.filter_off;
        if g >= 28 { return false; }
        if (self.fa1r >> g) & 1 == 0 { return false; }
        let w0 = *self.filter.get(2 * g).unwrap_or(&0);
        let w1 = *self.filter.get(2 * g + 1).unwrap_or(&0);
        let fbit = if f.ext {
            (f.id << 3) | (1 << 2)
        } else {
            (f.id << 21)
        } | (if f.rtr { 1 << 1 } else { 0 });
        if (self.fm1r >> g) & 1 == 0 {
            // mask mode
            if (self.fs1r >> g) & 1 != 0 {
                (fbit & w1) == (w0 & w1)
            } else {
                // 16-bit: two standard-frame filters (STID at bits 15:5 / 31:21)
                if f.ext { return false; }
                let id_a = (w0 >> 5) & 0x7FF; let msk_a = (w1 >> 5) & 0x7FF;
                let id_b = (w0 >> 21) & 0x7FF; let msk_b = (w1 >> 21) & 0x7FF;
                (((f.id ^ id_a) & msk_a) == 0) || (((f.id ^ id_b) & msk_b) == 0)
            }
        } else {
            // list mode (exact match)
            if (self.fs1r >> g) & 1 != 0 {
                (fbit & 0x1FFF_FFFE) == (w0 & 0x1FFF_FFFE)
            } else {
                if f.ext { return false; }
                let id_a = (w0 >> 5) & 0x7FF;
                let id_b = (w0 >> 21) & 0x7FF;
                f.id == id_a || f.id == id_b
            }
        }
    }

    /// Free a FIFO entry (RFOM write-1). Mirrors the real F407: the oldest
    /// entry is released and FULL cleared. The FD window shifts with the
    /// classic slots so fd_byte(fifo, 0, i) always reads the oldest frame.
    fn release_fifo(&mut self, fifo: usize) {
        let r = if fifo == 0 { &mut self.rf0r } else { &mut self.rf1r };
        let fmp = *r & 0x3;
        if fmp != 0 {
            *r = (*r & !0x3) | (fmp - 1);
            *r &= !0x4;                            // clear FULL
            let base = fifo * 3;
            for s in 0..2 {
                self.fd_rx[base + s] = self.fd_rx[base + s + 1];
                self.fd_len[base + s] = self.fd_len[base + s + 1];
            }
            self.fd_rx[base + 2] = [0; 64];
            self.fd_len[base + 2] = 0;
            // Classic mailbox words shift too (RX mailboxes are a FIFO).
            for s in 0..2 {
                self.rx[base + s] = self.rx[base + s + 1];
            }
            self.rx[base + 2] = Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 };
        }
    }
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0, msr: 0, tsr: 0, rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            // Reset costs: nominal 500 kbit/s, data 2 Mbit/s equivalents
            // on the 168 MHz virtual clock (see fd_timing_cost).
            nbtp: 0x0000_0000,
            dbtp: 0x0000_0000,
            tx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 3],
            rx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 6],
            fd_rx: [[0; 64]; 6],
            fd_len: [0; 6],
            fmr: 0x2A1C_0E01, fm1r: 0, fs1r: 0xFFFF_FFFF, ffa1r: 0, fa1r: 0,
            filter: [0; 56],
            irq_base: 0, node: 0, filter_off: 0,
        }
    }
}

impl Peripheral for Can {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000 => self.mcr,
            0x004 => self.msr,
            0x008 => self.tsr,
            0x00C => self.rf0r,
            0x010 => self.rf1r,
            0x014 => self.ier,
            0x018 => self.esr_live(),
            0x01C => self.btr,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return 0; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir,
                    0x04 => self.tx[i].tdtr,
                    0x08 => self.tx[i].tdlr,
                    0x0C => self.tx[i].tdhr,
                    _ => 0,
                }
            }
            0x1B0..=0x1EC => {
                let off = offset - 0x1B0;
                let m = (off / 0x10) as usize;
                if m >= 6 { return 0; }
                match off % 0x10 {
                    0x00 => self.rx[m].tir,
                    0x04 => self.rx[m].tdtr,
                    0x08 => self.rx[m].tdlr,
                    0x0C => self.rx[m].tdhr,
                    _ => 0,
                }
            }
            0x200 => self.fmr,
            0x204 => self.fm1r >> self.filter_off,
            0x20C => self.fs1r >> self.filter_off,
            0x214 => self.ffa1r >> self.filter_off,
            0x21C => self.fa1r >> self.filter_off,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize;
                self.filter.get(i + 2 * self.filter_off).copied().unwrap_or(0)
            }
            // ---- FDCAN bit-timing registers (model extension in the FD
            // guard band): NBTP @0x3E0, DBTP @0x3E4 (between F0 slots
            // ending at 0x3E0 and the F1 window at 0x3E8). FDCAN layouts,
            // stored verbatim; cost derived by fd_timing_cost().
            0x3E0 => self.nbtp,
            0x3E4 => self.dbtp,
            // ---- CAN FD payload window (model extension, bxCAN has no FD
            // registers — real FD needs an FDCAN block; this window is the
            // documented emulation surface): F0 slots at 0x320+slot*0x40
            // (0x320/0x360/0x3A0), F1 slot 0 at 0x3E8 (past NBTP/DBTP at
            // 0x3E0/0x3E4), F1 slots 1-2 shadow F0 slot 0/1 (documented
            // alias: the window covers 5 of 6 slots; F1 slot 2 is
            // unreachable — firmware uses FIFO0 for FD, like silicon
            // routes FD traffic to the configured FIFO). 16 LE words per
            // slot (word w at +w*4).
            o if (0x320..0x400).contains(&o) => {
                let (fifo, srel) = if o < 0x3E8 {
                    (0, o - 0x320)
                } else {
                    (1, o - 0x3E8)
                };
                let slot = (srel / 0x40) as usize;
                let word = ((srel % 0x40) / 4) as usize;
                if slot > 2 || word >= 16 {
                    return 0;
                }
                // F1 alias rule (see above): F1 slot s shadows F0 slot s
                // for s in 0..=1; F1 slot 2 reads 0.
                let (rfifo, rslot) = if fifo == 1 {
                    if slot == 2 {
                        return 0;
                    }
                    (0, slot)
                } else {
                    (0, slot)
                };
                let base = word * 4;
                (self.fd_byte(rfifo, rslot, base) as u32)
                    | ((self.fd_byte(rfifo, rslot, base + 1) as u32) << 8)
                    | ((self.fd_byte(rfifo, rslot, base + 2) as u32) << 16)
                    | ((self.fd_byte(rfifo, rslot, base + 3) as u32) << 24)
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => {
                let mask = 0x7F3F;
                self.mcr = (self.mcr & !mask) | (value & mask);
                let inrq = value & 1;
                let sleep = (value >> 1) & 1;
                if inrq != 0 {
                    self.msr |= 1; self.msr &= !2;
                } else {
                    self.msr &= !1; self.msr |= 2;
                }
                if sleep != 0 { self.msr |= 2; }
                else if inrq == 0 { self.msr &= !2; }
            }
            0x004 => self.msr = (self.msr & !0x0C0B) | (value & 0x0C0B),
            0x008 => self.tsr &= !(value & 0x0007_0707),
            0x00C => {
                let rtom = value & 0x10;
                if value & 0x20 != 0 { self.release_fifo(0); }  // RFOM w1c
                self.rf0r = (self.rf0r & 0xFFFF_FFCF) | rtom;   // FMP/FULL/FOVR untouched
                self.fire_interrupts(sys);
            }
            0x010 => {
                let rtom = value & 0x10;
                if value & 0x20 != 0 { self.release_fifo(1); }  // RFOM w1c
                self.rf1r = (self.rf1r & 0xFFFF_FFCF) | rtom;
                self.fire_interrupts(sys);
            }
            0x014 => {
                self.ier = value & 0x7FF;
                self.fire_interrupts(sys);
            }
            0x01C => self.btr = value & 0x3FFF_FFFF,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir = value,
                    0x04 => self.tx[i].tdtr = value,
                    0x08 => self.tx[i].tdlr = value,
                    0x0C => self.tx[i].tdhr = value,
                    _ => {}
                }
                if (offset - 0x180) % 0x10 == 0 && value & 1 != 0 {
                    if self.msr & 1 == 0 {
                        self.stage_mailbox(i, sys);
                    }
                }
            }
            0x1B0..=0x1EC => {
                let off = offset - 0x1B0;
                let m = (off / 0x10) as usize;
                if m >= 6 { return; }
                match off % 0x10 {
                    0x00 => self.rx[m].tir = value,
                    0x04 => self.rx[m].tdtr = value,
                    0x08 => self.rx[m].tdlr = value,
                    0x0C => self.rx[m].tdhr = value,
                    _ => {}
                }
            }
            0x200 => {
                if value & 1 != 0 {
                    self.fm1r = 0; self.fs1r = 0xFFFF_FFFF; self.ffa1r = 0; self.fa1r = 0;
                }
                self.fmr = value & 0x3F;
            }
            0x204 => self.fm1r = value << self.filter_off,
            0x20C => self.fs1r = value << self.filter_off,
            0x214 => self.ffa1r = value << self.filter_off,
            0x21C => self.fa1r = value << self.filter_off,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize + 2 * self.filter_off;
                if let Some(f) = self.filter.get_mut(i) { *f = value; }
            }
            0x3E0 => self.nbtp = value,
            0x3E4 => self.dbtp = value,
            _ => {}
        }
    }
}

/// One arbitration round on the shared CAN bus: among all staged frames,
/// the lowest arbitration ID wins (ties: lower node, then mailbox index).
/// The winner's mailbox completes on its node and the frame is delivered to
/// every node's RX that passes its filters (loopback frames only to the
/// transmitting node). Losers are re-staged for the next free round.
pub fn arbitrate_bus(sys: &System) {
    let mut staged = can_take_staged();
    if staged.is_empty() { return; }
    staged.sort_by_key(|f| (f.id, f.node, f.mailbox));
    let winner = staged.remove(0);
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        let Some(can) = b.as_any_mut().downcast_mut::<Can>() else { continue };
        if winner.loopback {
            if can.node == winner.node {
                can.receive_frame(sys, &winner);
            }
        } else {
            can.receive_frame(sys, &winner);
        }
        if can.node == winner.node {
            can.complete_tx(sys, winner.mailbox);
        }
    }
    if !staged.is_empty() {
        can_restage(staged);
    }
}

/// Inject a frame from an external transmitter onto the shared bus. Delivered
/// to every CAN node whose accept filters pass it (real bus broadcast), so the
/// guest's RX FIFO sees it exactly as if another node on the wire sent it.
/// `ext` selects a 29-bit extended ID; `rtr` marks a remote-transmit frame.
pub fn can_inject(sys: &System, id: u32, dlc: u32, data: &[u8], ext: bool, rtr: bool) {
    let mut d = [0u8; 8];
    let n = data.len().min(8);
    d[..n].copy_from_slice(&data[..n]);
    let f = CanFrame {
        node: 0,
        mailbox: 0,
        id,
        ext,
        rtr,
        dlc: dlc.min(8) as u8,
        data: d,
        loopback: false,
        fd: false,
        fd_len: 0,
        fd_data: [0; 64],
    };
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        let Some(can) = b.as_any_mut().downcast_mut::<Can>() else { continue };
        can.receive_frame(sys, &f);
    }
}

/// CAN FD data-length code: classic 0..=8 map 1:1; FD codes 9..=15 map to
/// 12/16/20/24/32/48/64 bytes (ISO 11898-1 Table 8). Values > 15 clamp.
pub fn fd_dlc_to_len(dlc: u8) -> usize {
    match dlc {
        0..=8 => dlc as usize,
        9 => 12, 10 => 16, 11 => 20, 12 => 24, 13 => 32, 14 => 48,
        _ => 64,
    }
}

/// Inject a CAN FD frame from an external transmitter (FDF set, BRS
/// optional). Up to 64 data bytes; `len` over 64 clamps. Delivered through
/// the same filter + FIFO path as classic frames (arbitration on the
/// 11/29-bit ID is identical); the FD payload lands in the per-node FD
/// mailbox window (see `can_fd_read`) and TDTR reports DLC + FDF.
/// `brs` marks bit-rate-switch (data phase at the FD rate — a flag only;
/// the virtual clock has no second bit-time to model).
pub fn can_inject_fd(sys: &System, id: u32, data: &[u8], ext: bool, brs: bool) {
    let mut fd_data = [0u8; 64];
    let n = data.len().min(64);
    fd_data[..n].copy_from_slice(&data[..n]);
    // DLC code for the byte count (smallest code covering n).
    let dlc = if n <= 8 { n as u8 }
    else if n <= 12 { 9 } else if n <= 16 { 10 } else if n <= 20 { 11 }
    else if n <= 24 { 12 } else if n <= 32 { 13 } else if n <= 48 { 14 }
    else { 15 };
    let f = CanFrame {
        node: 0,
        mailbox: 0,
        id,
        ext,
        rtr: false, // FD has no RTR (RRS reserved; RTR frames are classic)
        dlc,
        data: [0; 8],
        loopback: false,
        fd: true,
        fd_len: n as u8,
        fd_data,
    };
    let _ = brs;
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        let Some(can) = b.as_any_mut().downcast_mut::<Can>() else { continue };
        can.receive_frame(sys, &f);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{test_dummy_system, can_stage_tx};
    use std::rc::Rc;

    // The staged TX queue is process-global (shared by the node wrappers),
    // so CAN tests must run serially — otherwise parallel tests steal each
    // other's staged frames mid-arbitration.
    static CAN_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    const CAN1: u32 = 0x4000_6400;
    const CAN2: u32 = 0x4000_6800;

    /// Send a frame from the given node over the real register interface.
    /// Writes TDTR/TDLR/TDHR first, then TIR with TXRQ, returning nothing.
    fn tx_frame(sys: &Rc<System>, base: u32, id: u32, payload: &[u8; 8]) {
        sys.p.write(&sys, base + 0x184, 4, 8);
        let l = payload[0] as u32 | (payload[1] as u32) << 8
            | (payload[2] as u32) << 16 | (payload[3] as u32) << 24;
        let h = payload[4] as u32 | (payload[5] as u32) << 8
            | (payload[6] as u32) << 16 | (payload[7] as u32) << 24;
        sys.p.write(&sys, base + 0x188, 4, l);
        sys.p.write(&sys, base + 0x18C, 4, h);
        sys.p.write(&sys, base + 0x180, 4, (id << 21) | 1); // TIR0 TXRQ
    }

    /// Enable a pass-all filter (bank 0, 32-bit mask mode, FIFO0) + RX irqs.
    fn enable_rx(sys: &Rc<System>, base: u32) {
        sys.p.write(&sys, base + 0x200, 4, 1);       // FMR FINIT
        sys.p.write(&sys, base + 0x204, 4, 0);       // FM1R mask mode
        sys.p.write(&sys, base + 0x20C, 4, 0xFFFF_FFFF); // FS1R all 32-bit
        sys.p.write(&sys, base + 0x214, 4, 0);       // FFA1R FIFO0
        sys.p.write(&sys, base + 0x240, 4, 0);       // filter0 id=0
        sys.p.write(&sys, base + 0x244, 4, 0);       // filter0 mask=0 (pass all)
        sys.p.write(&sys, base + 0x21C, 4, 1);       // FA1R bank 0 active
        sys.p.write(&sys, base + 0x200, 4, 0);       // leave FINIT
        sys.p.write(&sys, base + 0x014, 4, 0x7F);    // IER: all TX/RX interrupts
    }

    #[test]
    fn arbitration_lowest_id_wins_and_broadcasts() {
        let _g = CAN_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        enable_rx(&sys, CAN1);
        enable_rx(&sys, CAN2);
        // Stage node 1 (id 0x300) and node 2 (id 0x200) in the same round.
        tx_frame(&sys, CAN1, 0x300, b"HI-CAN1!");
        tx_frame(&sys, CAN2, 0x200, b"HELLO-2!");
        arbitrate_bus(&sys);
        // 0x200 wins: both nodes have it in RX.
        let r00 = sys.p.read(&sys, CAN1 + 0x00C, 4) & 0x3;
        let r01 = sys.p.read(&sys, CAN2 + 0x00C, 4) & 0x3;
        assert_eq!(r00, 1, "CAN1 FMP0 after win");
        assert_eq!(r01, 1, "CAN2 FMP0 after win");
        let tir1 = sys.p.read(&sys, CAN1 + 0x1B0, 4);
        assert_eq!((tir1 >> 21) & 0x7FF, 0x200, "CAN1 RX id");
        let l = sys.p.read(&sys, CAN1 + 0x1B8, 4);
        assert_eq!(&l.to_le_bytes(), b"HELL", "CAN1 RX data L");
        // Winner (CAN2) completed TX: TXOK0 + TME0 + RQCP0.
        let tsr2 = sys.p.read(&sys, CAN2 + 0x008, 4);
        assert_ne!(tsr2 & (1 << 8), 0, "CAN2 TXOK0");
        assert_ne!(tsr2 & (1 << 16), 0, "CAN2 TME0");
        assert_ne!(tsr2 & (1 << 31), 0, "CAN2 RQCP0");
        // Loser (CAN1) is still pending: no TXOK yet.
        let tsr1 = sys.p.read(&sys, CAN1 + 0x008, 4);
        assert_eq!(tsr1 & (1 << 8), 0, "CAN1 TXOK0 deferred for loser");
        assert_eq!(tsr1 & (1 << 16), 0, "CAN1 TME0 deferred");
        // Next round: loser completes alone and both receive the 0x300 frame.
        arbitrate_bus(&sys);
        let tsr1b = sys.p.read(&sys, CAN1 + 0x008, 4);
        assert_ne!(tsr1b & (1 << 8), 0, "CAN1 TXOK0 after free round");
        assert_ne!(tsr1b & (1 << 16), 0, "CAN1 TME0 after free round");
        let r00b = sys.p.read(&sys, CAN1 + 0x00C, 4) & 0x3;
        assert_eq!(r00b, 2, "CAN1 FMP0 = 2 frames");
    }

    #[test]
    fn loopback_delivers_only_to_sender() {
        let _g = CAN_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        enable_rx(&sys, CAN1);
        // CAN2 has no filter enabled — remains empty.
        sys.p.write(&sys, CAN2 + 0x21C, 4, 0);
        sys.p.write(&sys, CAN1 + 0x01C, 4, 1 << 30); // BTR LBKM
        tx_frame(&sys, CAN1, 0x123, b"CANLOOP!");
        arbitrate_bus(&sys);
        let r0 = sys.p.read(&sys, CAN1 + 0x00C, 4) & 0x3;
        assert_eq!(r0, 1, "loopback self-delivery");
        let tir = sys.p.read(&sys, CAN1 + 0x1B0, 4);
        assert_eq!((tir >> 21) & 0x7FF, 0x123);
        assert_eq!(sys.p.read(&sys, CAN2 + 0x00C, 4) & 0x3, 0, "peer not delivered");
        let tsr = sys.p.read(&sys, CAN1 + 0x008, 4);
        assert_ne!(tsr & (1 << 16), 0, "loopback TX completes");
    }

    #[test]
    fn filter_gated_delivery() {
        let _g = CAN_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        sys.p.write(&sys, CAN1 + 0x200, 4, 1);
        sys.p.write(&sys, CAN1 + 0x21C, 4, 1);       // bank 0 active
        sys.p.write(&sys, CAN1 + 0x240, 4, 0x300 << 21); // only id 0x300 passes
        sys.p.write(&sys, CAN1 + 0x244, 4, 0x7FF << 21); // mask = STID
        sys.p.write(&sys, CAN1 + 0x20C, 4, 0xFFFF_FFFF); // 32-bit scale
        sys.p.write(&sys, CAN1 + 0x200, 4, 0);
        sys.p.write(&sys, CAN1 + 0x014, 4, 0x7F);
        tx_frame(&sys, CAN2, 0x200, b"HELLO-2!");
        arbitrate_bus(&sys);
        assert_eq!(sys.p.read(&sys, CAN1 + 0x00C, 4) & 0x3, 0, "unmatched id dropped");
        tx_frame(&sys, CAN2, 0x300, b"HI-CAN1!");
        arbitrate_bus(&sys);
        assert_eq!(sys.p.read(&sys, CAN1 + 0x00C, 4) & 0x3, 1, "matched id delivered");
    }

    #[test]
    fn fifo_release_and_fill_to_overflow() {
        let _g = CAN_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        enable_rx(&sys, CAN1);
        sys.p.write(&sys, CAN2 + 0x21C, 4, 0);
        for i in 0..3 {
            tx_frame(&sys, CAN2, 0x400 + i, &[i as u8; 8]);
            arbitrate_bus(&sys);
        }
        let fmp = sys.p.read(&sys, CAN1 + 0x00C, 4);
        assert_eq!(fmp & 0x3, 3, "FMP fills to 3");
        assert_eq!(fmp & (1 << 3), 0, "FOVR not yet");
        // 4th frame: FOVR set, FMP stays 3.
        tx_frame(&sys, CAN2, 0x403, &[9; 8]);
        arbitrate_bus(&sys);
        let r = sys.p.read(&sys, CAN1 + 0x00C, 4);
        assert_eq!(r & 0x3, 3);
        assert_ne!(r & (1 << 3), 0, "FOVR0 set on overflow");
        // RFOM release shifts the FIFO.
        sys.p.write(&sys, CAN1 + 0x00C, 4, 0x20);
        let r2 = sys.p.read(&sys, CAN1 + 0x00C, 4);
        assert_eq!(r2 & 0x3, 2, "FMP decremented on RFOM");
        assert_eq!(r2 & 0x4, 0, "FULL0 cleared");
    }

    #[test]
    fn fd_roundtrip_64b_dlc_fdf_window_and_release() {
        use super::{can_inject_fd, fd_dlc_to_len};
        let _g = CAN_TEST_LOCK.lock().unwrap();
        // DLC table pins (ISO 11898-1).
        assert_eq!(fd_dlc_to_len(8), 8);
        assert_eq!(fd_dlc_to_len(9), 12);
        assert_eq!(fd_dlc_to_len(13), 32);
        assert_eq!(fd_dlc_to_len(15), 64);
        let sys = test_dummy_system();
        enable_rx(&sys, CAN1);
        // 64-byte FD frame: first 8 bytes visible in the classic words,
        // full payload in the FD window, TDTR = DLC|FDF.
        let payload: Vec<u8> = (0u8..64).collect();
        can_inject_fd(&sys, 0x123, &payload, false, false);
        let tdtr = sys.p.read(&sys, CAN1 + 0x1B4, 4);
        assert_eq!(tdtr & 0xF, 15, "DLC=15 for 64B, got {tdtr:#x}");
        assert_ne!(tdtr & (1 << 16), 0, "FDF set");
        let tdlr = sys.p.read(&sys, CAN1 + 0x1B8, 4);
        assert_eq!(tdlr, 0x0302_0100, "classic words carry first bytes");
        // FD window via the harness path + via MMIO (F0 base 0x320 /
        // F1 base 0x3A0, slot stride 0x40, 16 LE words per slot).
        assert_eq!(sys.p.can_fd_len(CAN1, 0, 0), 64, "fd_len 64");
        for i in [0usize, 7, 8, 31, 63] {
            assert_eq!(sys.p.can_fd_byte(CAN1, 0, 0, i), i as u8, "fd byte {i}");
            let w = sys.p.read(&sys, CAN1 + 0x320 + (i & !3) as u32, 4);
            assert_eq!(((w >> (8 * (i % 4))) & 0xFF) as u8, i as u8, "mmio fd byte {i}");
        }
        // RFOM release shifts the FD window with the FIFO (oldest gone).
        let p2: Vec<u8> = (100u8..116).collect(); // 16 bytes -> DLC 10
        can_inject_fd(&sys, 0x124, &p2, false, true);
        sys.p.write(&sys, CAN1 + 0x00C, 4, 0x20); // RFOM
        assert_eq!(sys.p.can_fd_len(CAN1, 0, 0), 16, "window shifted to 2nd frame");
        assert_eq!(sys.p.can_fd_byte(CAN1, 0, 0, 0), 100, "2nd frame first byte");
        let tdtr2 = sys.p.read(&sys, CAN1 + 0x1B4, 4);
        assert_eq!(tdtr2 & 0xF, 10, "DLC=10 for 16B");
        // Classic frame after FD: window cleared for that slot.
        tx_frame(&sys, CAN2, 0x200, &[7; 8]);
        arbitrate_bus(&sys);
        sys.p.write(&sys, CAN1 + 0x00C, 4, 0x20); // release the 16B FD frame
        assert_eq!(sys.p.can_fd_len(CAN1, 0, 0), 0, "classic slot has no FD length");
    }
}