use crate::system::{CanFrame, System, can_restage, can_stage_tx, can_take_staged};
use super::Peripheral;

#[derive(Clone, Copy)]
struct Mailbox {
    tir: u32, tdtr: u32, tdlr: u32, tdhr: u32,
}

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    tx: [Mailbox; 3],
    // 3 mailboxes per FIFO: rx[fif*3 + slot] (RIR at 0x1B0/0x1C0/0x1D0 for
    // FIFO0, 0x1E0/0x1F0/0x200 for FIFO1 — the real F407 map).
    rx: [Mailbox; 6],
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
        });
        self.fire_interrupts(sys);
    }

    /// Mark a mailbox transmission complete (won arbitration).
    fn complete_tx(&mut self, sys: &System, i: usize) {
        if i >= 3 { return; }
        self.tsr |= 1 << (8 + i);                  // TXOK
        self.tsr |= 1 << (16 + i);                 // TME (mailbox empty again)
        self.tsr |= 1 << (31 - 4 * i);             // RQCP (bits 31/27/23)
        self.fire_interrupts(sys);
    }

    /// Deliver a won frame into the RX FIFO chosen by the first passing
    /// filter bank. Returns false if no active filter matched.
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
        mx.tdtr = f.dlc as u32;
        mx.tdlr = f.data[0] as u32 | (f.data[1] as u32) << 8
            | (f.data[2] as u32) << 16 | (f.data[3] as u32) << 24;
        mx.tdhr = f.data[4] as u32 | (f.data[5] as u32) << 8
            | (f.data[6] as u32) << 16 | (f.data[7] as u32) << 24;
        *r = (*r & !0x3) | (fmp + 1);              // FMP++
        self.fire_interrupts(sys);
        true
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
    /// entry is released and FULL cleared.
    fn release_fifo(&mut self, fifo: usize) {
        let r = if fifo == 0 { &mut self.rf0r } else { &mut self.rf1r };
        let fmp = *r & 0x3;
        if fmp != 0 {
            *r = (*r & !0x3) | (fmp - 1);
            *r &= !0x4;                            // clear FULL
        }
    }
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0, msr: 0, tsr: 0, rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            tx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 3],
            rx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 6],
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
            0x018 => self.esr,
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
}