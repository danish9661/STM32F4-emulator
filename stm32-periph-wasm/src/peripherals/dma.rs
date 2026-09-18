use crate::system::{System, DmaTransfer, DmaDir, set_dma_intr_info};
use super::Peripheral;

#[derive(Default)]
pub struct Dma {
    name: String,
    streams: [Stream; 8],
}

impl Dma {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        // NOTE: prefix match would also swallow "DMA2D" (the Chrom-ART
        // engine, a separate model) and panic below in stream_irq — match
        // the two real controllers exactly.
        if name == "DMA1" || name == "DMA2" {
            Some(Box::new(Self { name: name.to_string(), ..Self::default() }))
        } else {
            None
        }
    }

    fn stream_irq(&self, stream: usize) -> i32 {
        match self.name.as_str() {
            "DMA1" => 11 + stream as i32,
            "DMA2" => 56 + stream as i32,
            _ => panic!("Unknown DMA controller: {}", self.name),
        }
    }

    /// Controller name ("DMA1"/"DMA2", for the with_dma driver lookup).
    pub fn controller_name(&self) -> &str {
        &self.name
    }

    /// Flip one stream's double-buffer target at its TC moment (called
    /// from mark_dma_completed for both controllers; the index space is
    /// per-controller 0..8 so both are fanned out — only the stream that
    /// actually runs DBM flips).
    pub fn flip_stream_target(&mut self, stream_idx: usize) {
        if stream_idx < 8 {
            self.streams[stream_idx].flip_dbm_target();
        }
    }

    /// FEIF (FIFO error, status bit 0) scope probe for one stream.
    pub fn stream_feif(&self, stream_idx: usize) -> bool {
        stream_idx < 8 && self.streams[stream_idx].status & 1 != 0
    }

    /// CT (current target, CR bit 19 view) scope probe for one stream.
    pub fn stream_ct(&self, stream_idx: usize) -> bool {
        stream_idx < 8 && self.streams[stream_idx].ct()
    }

    /// FCR FIFO threshold in words (scope probe for the FTH contract).
    pub fn stream_fifo_threshold(&self, stream_idx: usize) -> u32 {
        if stream_idx < 8 { self.streams[stream_idx].fifo_threshold_words() } else { 0 }
    }
}

impl Peripheral for Dma {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let mut v = 0u32;
                for i in 0..=3 {
                    // Completion latches TCIF/HTIF into the stream status
                    // (real IFCR w1c semantics) so repeated reads keep
                    // seeing it until the guest clears it.
                    if _sys.dma_check_completion(i) {
                        self.streams[i].status |= (1 << 4) | (1 << 3);
                    }
                    v |= (self.streams[i].status as u32) << (i * 6);
                }
                v
            }
            0x04 => {
                let mut v = 0u32;
                for i in 0..=3 {
                    if _sys.dma_check_completion(i + 4) {
                        self.streams[i + 4].status |= (1 << 4) | (1 << 3);
                    }
                    v |= (self.streams[i + 4].status as u32) << (i * 6);
                }
                v
            }
            _ => {
                match Access::from_offset(offset) {
                    Access::StreamReg(i, o) => self.streams[i].read(&self.name, _sys, o),
                    _ => 0,
                }
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x08 => {
                for i in 0..=3 {
                    let mask = (value >> (i * 6)) & 0x1F;
                    if mask != 0 { self.streams[i].status &= !(mask as u8); }
                }
            }
            0x0C => {
                for i in 0..=3 {
                    let mask = (value >> (i * 6)) & 0x1F;
                    if mask != 0 { self.streams[i + 4].status &= !(mask as u8); }
                }
            }
            _ => {
                match Access::from_offset(offset) {
                    Access::StreamReg(i, o) => {
                        self.streams[i].write(&self.name, sys, i, o, value, self.stream_irq(i));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Default)]
struct Stream {
    cr: u32,
    next_cr: Option<u32>,
    ndtr: u32,
    par: u32,
    m0ar: u32,
    m1ar: u32,
    fcr: u32,
    status: u8,
    /// Double-buffer target latch (DBM bit 18): false = M0AR is the
    /// current target, true = M1AR. Flips at each TC (silicon swaps the
    /// memory target every full transfer); CT (CR bit 19) reads it back.
    /// Re-armed to M0AR whenever the stream is (re-)enabled.
    dbm_target_m1: bool,
}

impl Stream {
    fn channel(&self) -> u8 { ((self.cr >> 25) & 0b111) as u8 }
    fn dir(&self) -> Dir {
        match (self.cr >> 6) & 0b11 {
            0b00 => Dir::Read,
            0b01 => Dir::Write,
            0b10 => Dir::MemCopy,
            _ => Dir::Invalid,
        }
    }
    fn dma_size(bits: u32) -> usize {
        match bits { 0b00 => 1, 0b01 => 2, _ => 4 }
    }
    fn data_size(&self) -> usize {
        let msize = Self::dma_size((self.cr >> 13) & 0b11);
        let psize = Self::dma_size((self.cr >> 11) & 0b11);
        std::cmp::max(msize, psize) * self.ndtr as usize
    }
    fn data_addr(&self) -> u32 {
        if self.dbm_target_m1 { self.m1ar } else { self.m0ar }
    }

    /// FIFO threshold level in 32-bit words: FCR FTH[1:0] = 1/4-full,
    /// 1/2-full, 3/4-full, full of the 4-word FIFO → 1/2/3/4 words.
    fn fifo_threshold_words(&self) -> u32 {
        match (self.fcr >> 1) & 3 {
            0 => 1,
            1 => 2,
            2 => 3,
            _ => 4,
        }
    }

    /// Whether this stream runs the FIFO (DMDIS bit 2 set) vs direct
    /// mode (reset = direct, threshold ignored).
    fn fifo_mode(&self) -> bool {
        self.fcr & (1 << 2) != 0
    }

    /// FEIF (FIFO error) probe: in FIFO mode a burst wider than the
    /// threshold with an undersized NDTR cannot stage — silicon flags
    /// FEIF (status bit 0) and the stream stalls. Direct mode never
    /// flags (no FIFO to overrun).
    fn fifo_error(&self) -> bool {
        if !self.fifo_mode() {
            return false;
        }
        // Burst widths: MBURST (CR bits 24:23) / PBURST (bits 22:21):
        // 0 = single, 1 = INCR4, 2 = INCR8, 3 = INCR16 (beats per burst).
        let burst = |sel: u32| match sel { 1 => 4, 2 => 8, 3 => 16, _ => 1 };
        let beats = burst((self.cr >> 23) & 3).max(burst((self.cr >> 21) & 3));
        // A burst must fit the FIFO threshold AND the remaining NDTR:
        // fewer items left than one burst = FIFO underrun at commit.
        beats > self.fifo_threshold_words() || (self.ndtr as u32) < beats
    }

    /// Flip the double-buffer target (DBM): called when the JS/driver
    /// side completes a DBM transfer (the TC moment — silicon swaps M0/M1
    /// at terminal count). Outside DBM it is a no-op (CT reads M0).
    /// Returns the NEW target address (the buffer the NEXT transfer will
    /// fill — what firmware reads after seeing CT flip).
    pub fn flip_dbm_target(&mut self) -> u32 {
        if self.cr & (1 << 18) != 0 {
            self.dbm_target_m1 = !self.dbm_target_m1;
        }
        self.data_addr()
    }

    /// Whether this stream runs double-buffer mode (scope probe).
    pub fn dbm(&self) -> bool {
        self.cr & (1 << 18) != 0
    }

    fn do_xfer(&self, name: &str, sys: &System, stream_idx: usize) {
        let dir = self.dir();
        let data_addr = self.data_addr();
        let size = self.data_size();
        let peri_addr = self.par;

        let dma_dir = match dir {
            Dir::Read => DmaDir::Read,
            Dir::Write => DmaDir::Write,
            Dir::MemCopy => DmaDir::MemCopy,
            Dir::Invalid => return,
        };

        let (src, dst) = match dir {
            Dir::Read => (peri_addr, self.m0ar),
            Dir::Write => (data_addr, peri_addr),
            Dir::MemCopy => (peri_addr, data_addr),
            Dir::Invalid => (0, 0),
        };

        let peripheral = dir != Dir::MemCopy;
        let pinc = (self.cr >> 9) & 1 != 0; // PINC (bit 9): increment PA per transfer; bit 10 is MINC
        let p_size = Self::dma_size((self.cr >> 11) & 0b11);
        sys.queue_dma_transfer(DmaTransfer {
            direction: dma_dir,
            stream_idx,
            dma_name: name.to_string(),
            src, dst,
            size,
            peri_addr,
            peripheral,
            pinc,
            p_size,
        });

        log::debug!("{} queued DMA xfer stream={} dir={:?} src=0x{:08x} dst=0x{:08x} size={}",
            name, stream_idx, dir, src, dst, size);
    }

    /// Current-target probe for double-buffer mode (CT, CR bit 19):
    /// true = M1AR is the live target. Outside DBM it always reads M0.
    fn ct(&self) -> bool {
        (self.cr & (1 << 18) != 0) && self.dbm_target_m1
    }

    fn read(&mut self, _name: &str, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => {
                // CT (bit 19) is status, not storage: recompose it from
                // the double-buffer latch on every read (silicon flips it
                // at TC; a stored bit would go stale after completion).
                let mut v = self.cr & !(1 << 19);
                if self.ct() {
                    v |= 1 << 19;
                }
                if let Some(next_cr) = self.next_cr.take() {
                    self.cr = next_cr;
                }
                if self.dir() == Dir::Write && self.data_size() == 0 {
                    self.next_cr = Some(self.cr ^ 1);
                }
                v
            }
            0x0004 => self.ndtr,
            0x0008 => self.par,
            0x000c => self.m0ar,
            0x0010 => self.m1ar,
            0x0014 => self.fcr,
            _ => 0,
        }
    }

    /// FIFO-error check at EN time (see `fifo_error`): when the burst
    /// cannot stage, latch FEIF (status bit 0) + pend the error IRQ when
    /// TEIE is set, and do NOT queue the transfer (silicon stalls the
    /// stream before moving a byte). Returns true when stalled.
    fn check_fifo_error(&mut self, sys: &System, irq: i32) -> bool {
        if self.fifo_error() {
            self.status |= 1; // FEIF
            if (self.cr >> 2) & 1 != 0 {
                // TEIE covers transfer errors incl. FIFO error.
                sys.p.nvic.borrow_mut().set_intr_pending(irq);
            }
            return true;
        }
        false
    }

    fn write(&mut self, name: &str, sys: &System, stream_idx: usize, offset: u32, mut value: u32, irq: i32) {
        match offset {
            0x0000 => {
                // CT (bit 19) is read-only status: a guest write cannot
                // set the target (silicon ignores it); mask it out so a
                // read-modify-write of CR never flips the latch.
                value &= !(1 << 19);
                self.cr = value;
                if value & 1 != 0 {
                    // DBM re-arm: enabling always restarts at M0AR.
                    if value & (1 << 18) != 0 {
                        self.dbm_target_m1 = false;
                    }
                    // Direct-mode error: DBM requires the FIFO (DMDIS
                    // set). DBM with direct mode is a configuration
                    // error — silicon flags TEIF + refuses EN; the
                    // model latches TEIF (status bit 2, +IRQ on TEIE)
                    // and drops the enable (CR stays as written but the
                    // transfer never queues — guest observes EN set
                    // with no completion, then TEIF).
                    if value & (1 << 18) != 0 && !self.fifo_mode() {
                        self.status |= 1 << 2; // TEIF
                        if (value >> 2) & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(irq);
                        }
                        return;
                    }
                    if self.check_fifo_error(sys, irq) {
                        return; // stalled: no transfer queued
                    }
                    self.do_xfer(name, sys, stream_idx);
                    // TCIF/HTIF are NOT set here: completion flags appear only
                    // once the JS driver services the queue (dma_set_completed
                    // -> dma_check_completion in the LISR/HISR read path).
                    self.status &= !((1 << 4) | (1 << 3));
                    let tcie = ((value >> 4) & 1) as u8;
                    let htie = ((value >> 3) & 1) as u8;
                    let teie = ((value >> 2) & 1) as u8;
                    let flags = tcie | (htie << 1) | (teie << 2);
                    set_dma_intr_info(stream_idx, irq, flags);
                    if tcie != 0 || htie != 0 || teie != 0 {
                        sys.p.nvic.borrow_mut().set_intr_pending(irq);
                    }
                    value &= !1;
                    self.ndtr = 0;
                    self.next_cr = Some(value);
                }
            }
            0x0004 => { self.ndtr = value & 0xFFFF; }
            0x0008 => { self.par = value; }
            0x000c => { self.m0ar = value; }
            0x0010 => { self.m1ar = value; }
            0x0014 => { self.fcr = value; }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Dir { Read, Write, MemCopy, Invalid }

enum Access { Reg(u32), StreamReg(usize, u32) }

impl Access {
    fn from_offset(offset: u32) -> Self {
        if offset < 0x10 { Access::Reg(offset) }
        else {
            let stride = 0x18;
            let start = 0x10;
            let o = offset - start;
            Access::StreamReg((o / stride) as usize, o % stride)
        }
    }
}

#[cfg(test)]
mod gap10_tests {
    use super::*;
    use crate::system::test_dummy_system;

    fn stream(name: &str) -> (std::rc::Rc<crate::system::System>, Box<dyn Peripheral>) {
        let sys = test_dummy_system();
        let d = Dma::new(name).unwrap();
        (sys, d)
    }

    // FCR threshold contract: FTH field maps 0/1/2/3 → 1/2/3/4 words.
    #[test]
    fn fcr_threshold_maps_fth() {
        let (sys, mut d) = stream("DMA2");
        let b = d.as_any_mut().downcast_mut::<Dma>().unwrap();
        for (fth, words) in [(0u32, 1u32), (1, 2), (2, 3), (3, 4)] {
            b.streams[0].fcr = fth << 1;
            assert_eq!(b.streams[0].fifo_threshold_words(), words, "FTH={fth}");
        }
        let _ = &sys;
    }

    // INCR8 burst against a 1-word threshold stalls with FEIF (FIFO mode);
    // direct mode never flags.
    #[test]
    fn burst_over_threshold_stalls_feif() {
        let (sys, mut d) = stream("DMA2");
        let b = d.as_any_mut().downcast_mut::<Dma>().unwrap();
        // FIFO mode (DMDIS), FTH=00 (1 word), MBURST=INCR8, NDTR=16.
        b.streams[0].cr = (2 << 23) | (1 << 2);
        b.streams[0].fcr = 1 << 2; // DMDIS, FTH=00
        b.streams[0].ndtr = 16;
        b.streams[0].write("DMA2", &sys, 0, 0x0000, b.streams[0].cr | 1, 56 + 0);
        assert_ne!(b.streams[0].status & 1, 0, "FEIF latches on burst>threshold");
        // Direct mode (DMDIS clear): same shape never flags.
        let (sys2, mut d2) = stream("DMA2");
        let b2 = d2.as_any_mut().downcast_mut::<Dma>().unwrap();
        b2.streams[0].cr = 2 << 23;
        b2.streams[0].fcr = 0;
        b2.streams[0].ndtr = 16;
        b2.streams[0].write("DMA2", &sys2, 0, 0x0000, b2.streams[0].cr | 1, 56 + 0);
        assert_eq!(b2.streams[0].status & 1, 0, "direct mode never FEIFs");
    }

    // DBM without FIFO (direct mode) is a config error: TEIF latches,
    // no transfer queued.
    #[test]
    fn dbm_needs_fifo_or_teif() {
        let (sys, mut d) = stream("DMA2");
        let b = d.as_any_mut().downcast_mut::<Dma>().unwrap();
        b.streams[0].cr = 1 << 18; // DBM, direct mode (DMDIS clear)
        b.streams[0].fcr = 0;
        b.streams[0].ndtr = 8;
        let n0 = sys.pending_dma_count();
        b.streams[0].write("DMA2", &sys, 0, 0x0000, b.streams[0].cr | 1, 56 + 0);
        assert_ne!(b.streams[0].status & (1 << 2), 0, "TEIF on DBM+direct");
        assert_eq!(sys.pending_dma_count(), n0, "stalled: nothing queued");
    }

    // DBM+CT: EN arms M0 (CT reads 0); flip at TC points at M1 (CT=1).
    #[test]
    fn dbm_ct_flips_at_tc() {
        let (sys, mut d) = stream("DMA2");
        let b = d.as_any_mut().downcast_mut::<Dma>().unwrap();
        b.streams[0].m0ar = 0x20000000;
        b.streams[0].m1ar = 0x20001000;
        b.streams[0].cr = (1 << 18) | (1 << 2); // DBM + DMDIS
        b.streams[0].fcr = (1 << 2) | (3 << 1); // FIFO, full threshold
        b.streams[0].ndtr = 16;
        b.streams[0].write("DMA2", &sys, 0, 0x0000, b.streams[0].cr | 1, 56 + 0);
        assert!(!b.streams[0].ct(), "CT=M0 right after EN");
        let next = b.streams[0].flip_dbm_target();
        assert_eq!(next, 0x20001000, "TC flips target to M1");
        assert!(b.streams[0].ct(), "CT reads M1 after flip");
    }
}
