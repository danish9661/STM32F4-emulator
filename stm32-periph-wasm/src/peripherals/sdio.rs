use crate::system::System;
use super::Peripheral;

const SDIO_IRQ: i32 = 49;

#[derive(Clone, Copy, PartialEq)]
enum SdState { Idle, Ready, Ident, Stby, Tran }

pub struct Sdio {
    power: u32, clkcr: u32, arg: u32, cmd: u32, respcmd: u32,
    resp: [u32; 4], dtimer: u32, dlen: u32, dctrl: u32, dcount: u32,
    sta: u32, icr: u32, mask: u32, fifocnt: u32, fifo: u32,
    sd_state: SdState, rca: u16, data_xfer_active: bool,
    /// Card image for block read/write (harness = the flash array):
    /// 512-byte blocks, indexed by block number (ARG for CMD17/18/24).
    /// Seeded by `sdio_bind_card` before init (mirrors the QSPI/FSMC
    /// image pattern). Reads return the stored block (erased = 0xFF);
    /// CMD24 writes land here, so a write-then-read round-trips.
    card: Option<Vec<u8>>,
    /// TX staging for CMD24 (host-to-card): words the guest pushed to
    /// the FIFO port while the transfer is armed. Drained into `card`
    /// at completion (silicon streams them onto DAT; the model commits
    /// them to the image — the observable contract is the round-trip).
    tx_stage: Vec<u32>,
    /// APP_CMD latch (CMD55 completed, next command is ACMD<n>).
    app_pending: bool,
    /// Wide-bus select (CLKCR WIDBUS bits 12:11, latched from the last
    /// ACMD6): 0 = 1-bit, 1 = 4-bit, 2 = 8-bit. Stored + readable via a
    /// scope probe; the data path is width-agnostic (same bytes), so this
    /// is the observable contract, not a timing change.
    wide_bus: u8,
    /// SDIO interrupt latch (STA SDIOIT bit 22): set by the harness via
    /// `sdio_card_irq` (card asserts the DAT1 interrupt outside a transfer),
    /// cleared by ICR SDIOITC (bit 22). Gated on MASK SDIOITIE like the
    /// other flags for IRQ49.
    sdio_it: bool,
    /// Data-path timing state (instruction-count clock):
    /// - `data_start`: clock value when the current CMD17/18 transfer
    ///   began (for DTIMER timeout + width-scaled duration).
    /// - `data_done_at`: clock value when the transfer completes
    ///   (DATAEND/DBCKEND latch there, not instantly at CMD time).
    /// - `xfer_len`: latched DLEN snapshot for the in-flight transfer.
    data_start: u64,
    data_done_at: u64,
    xfer_len: u32,
    /// Data CRC fault injection (harness = the bad card): the next
    /// CMD17/18 data completion latches DCRCFAIL instead of DATAEND.
    data_crc_fault: bool,
    /// Direction latch: true when the ARMED transfer is a CMD24
    /// host-to-card write (FIFO words stage for commit). Latched at CMD
    /// time — self.cmd is overwritten by the next command while the
    /// transfer is still in flight, so every arm below must consult the
    /// latch, never cmd.
    xfer_is_write: bool,
    /// Drained-bytes snapshot for the FIFO image path: bytes already
    /// served to the guest from the current transfer. Completion zeroes
    /// DCOUNT (silicon: DCOUNT reads 0 after DATAEND) but the FIFO words
    /// stay readable — firmware drains AFTER polling DATAEND — so the
    /// offset must survive completion (reset only by the next CMD arm).
    fifo_done: u32,
}

impl Sdio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SDIO" {
            Some(Box::new(Sdio { sta: 0x7E48_0000, ..Default::default() }))
        } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.sta & self.mask != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(SDIO_IRQ);
        }
    }
}

impl Default for Sdio {
    fn default() -> Self {
        Self {
            power: 0, clkcr: 0, arg: 0, cmd: 0, respcmd: 0,
            resp: [0; 4], dtimer: 0, dlen: 0, dctrl: 0, dcount: 0,
            sta: 0, icr: 0, mask: 0, fifocnt: 0, fifo: 0,
            sd_state: SdState::Idle, rca: 0, data_xfer_active: false,
            card: None, tx_stage: Vec::new(),
            app_pending: false,
            wide_bus: 0, sdio_it: false,
            data_start: 0, data_done_at: 0, xfer_len: 0,
            data_crc_fault: false, xfer_is_write: false, fifo_done: 0,
        }
    }
}

impl Sdio {
    /// Current bus width select (CLKCR WIDBUS via ACMD6): 0 = 1-bit,
    /// 1 = 4-bit, 2 = 8-bit. Scope probe for the wide-bus contract.
    pub fn bus_width(&self) -> u8 {
        self.wide_bus
    }

    /// Harness = the card's DAT1 interrupt line: latch SDIOIT (STA bit 22).
    /// Fires IRQ49 when MASK SDIOITIE is set, like every other flag.
    /// Cleared by writing ICR SDIOITC (bit 22).
    pub fn card_irq(&mut self, sys: &System, set: bool) {
        self.sdio_it = set;
        if set {
            self.sta |= 1 << 22; // SDIOIT
            self.fire_interrupts(sys);
        } else {
            self.sta &= !(1 << 22);
        }
    }

    /// Harness = the bad card: the next CMD17/18 data completion latches
    /// DCRCFAIL (STA bit 1) instead of DATAEND/DBCKEND. One-shot (consumed
    /// by the completion, like a real single bad block).
    pub fn fault_data_crc(&mut self) {
        self.data_crc_fault = true;
    }

    /// Bind a card image (harness = the flash array): `blocks` 512-byte
    /// blocks, erased 0xFF. Called before init (mirrors the QSPI/FSMC
    /// image pattern). CMD17/18 read from it; CMD24 writes land in it,
    /// so a guest write-then-read round-trips through the image.
    pub fn bind_card(&mut self, blocks: u32) {
        let n = (blocks.max(1) as usize) * 512;
        self.card = Some(vec![0xFF; n]);
    }

    /// Read one 512-byte block from the bound image (erased 0xFF when
    /// unbound). Scope probe for the round-trip contract.
    pub fn read_block(&self, block: u32) -> Vec<u8> {
        match &self.card {
            Some(c) => {
                let o = block as usize * 512;
                if o + 512 <= c.len() { c[o..o + 512].to_vec() } else { vec![0xFF; 512] }
            }
            None => vec![0xFF; 512],
        }
    }

    /// Card block count (0 = unbound).
    pub fn card_blocks(&self) -> u32 {
        match &self.card { Some(c) => (c.len() / 512) as u32, None => 0 }
    }

    /// Data-path timing: virtual-instruction cost of one CMD17/18 transfer
    /// of `len` bytes. Scales with the latched bus width (1/4/8 data lines
    /// move 1/4/8 bits per SDIO clock) and the CLKCR clock divider, on top
    /// of a fixed command overhead. 1 SDIO clock = 1 virtual instruction
    /// at CLKDIV=0 (the model's 1:1 instruction clock); the divider scales
    /// linearly. Documented estimate (not silicon-cycle-exact): the
    /// observable contract is ordering (DBCKEND/DATAEND after the window,
    /// DTIMEOUT when DTIMER is shorter) and width-monotonicity.
    fn data_cost(&self, len: u32) -> u64 {
        let lines = match self.wide_bus {
            1 => 4,
            2 => 8,
            _ => 1,
        } as u64;
        let div = ((self.clkcr & 0xFF) as u64) + 1;
        // Bits on the wire, spread over `lines` data lines, clocked at
        // 1/div virtual instructions per SDIO clock, plus 64 clocks of
        // command/response overhead.
        (len as u64 * 8 / lines) * div + 64 * div
    }
}

impl Sdio {
    /// Application-command prefix state: set by CMD55 (APP_CMD), consumed
    /// by the next command (ACMD<n>). Tracked explicitly: `app_pending`
    /// latches on a completed CMD55 and clears on the following command
    /// (real cards hold APP_CMD only until the next command arrives).
    fn app_cmd_pending(&self) -> bool {
        self.app_pending
    }

    /// Poll the in-flight data transfer toward completion. Called at CMD
    /// time (zero-wait fast path when the window is already elapsed, e.g.
    /// tiny transfers), on every STA read, and from tick(). Latches, in
    /// order: DTIMEOUT (DTIMER shorter than the cost) else DCRCFAIL (fault
    /// armed) else DBCKEND + DATAEND. RXOVERR/TXUNDERR stay on the arm
    /// path (a drained-then-re Fed FIFO is a driver-sequencing artifact,
    /// not silicon behavior — not modeled).
    fn poll_data_done(&mut self, sys: &System) {
        if !self.data_xfer_active {
            return;
        }
        use crate::system::instruction_count;
        let now = instruction_count();
        // DTIMER is in SDIO-clock units: timeout when the elapsed window
        // exceeds DTIMER clocks (scaled like the cost: divider applies).
        let div = ((self.clkcr & 0xFF) as u64) + 1;
        let elapsed_clocks = now.wrapping_sub(self.data_start) / div;
        if elapsed_clocks > self.dtimer as u64 {
            self.sta |= 1 << 3; // DTIMEOUT
            self.data_xfer_active = false;
            self.fifocnt = 0;
            self.fire_interrupts(sys);
            return;
        }
        if now < self.data_done_at {
            return; // window still open
        }
        // Window elapsed: complete (or CRC-fail when armed).
        self.data_xfer_active = false;
        self.dcount = 0;
        self.fifocnt = 0;
        self.sta &= !((1 << 12) | (1 << 13)); // TXACT/RXACT clear
        if self.data_crc_fault {
            self.data_crc_fault = false;
            self.sta |= 1 << 1; // DCRCFAIL
            self.tx_stage.clear(); // bad block: staged TX words dropped
        } else {
            // CMD24 (host-to-card write): commit staged FIFO words into
            // the bound image at completion (silicon streams them onto
            // DAT during the window; the commit lands here so the
            // round-trip is observable right at DATAEND).
            if self.xfer_is_write {
                self.commit_tx_stage();
            }
            self.sta |= (1 << 8) | (1 << 10); // DATAEND + DBCKEND
        }
        self.fire_interrupts(sys);
    }

    /// Commit CMD24 staged TX words into the card image at the latched
    /// block address (ARG at CMD time). Short stages zero-pad (silicon
    /// clocks exactly DLEN bytes; unwritten tail stays erased).
    fn commit_tx_stage(&mut self) {
        // Latch BEFORE the card borrow (both touch &mut self — split the
        // borrow like the I2C PEC path does, or the borrow checker sees
        // the RefMut destructor as a use of the immutable borrow).
        let block = self.arg;
        let staged: Vec<u32> = self.tx_stage.clone();
        if let Some(card) = self.card.as_mut() {
            let o = block as usize * 512;
            if o < card.len() {
                let mut bytes = Vec::with_capacity(512);
                for w in &staged {
                    bytes.extend_from_slice(&w.to_le_bytes());
                }
                bytes.resize(512, 0);
                let n = (o + 512).min(card.len()) - o;
                card[o..o + n].copy_from_slice(&bytes[..n]);
            }
        }
        self.tx_stage.clear();
    }
}

impl Peripheral for Sdio {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.power,
            0x04 => self.clkcr,
            0x08 => self.arg,
            0x0C => self.cmd,
            0x10 => self.respcmd,
            0x14 => self.resp[0],
            0x18 => self.resp[1],
            0x1C => self.resp[2],
            0x20 => self.resp[3],
            0x24 => self.dtimer,
            0x28 => self.dlen,
            0x2C => self.dctrl,
            0x30 => self.dcount,
            0x34 => {
                self.poll_data_done(sys);
                self.sta
            }
            0x38 => self.icr,
            0x3C => self.mask,
            0x48 => self.fifocnt,
            0x80 => {
                // FIFO read port: card-to-host words come from the bound
                // image at latched block + drained offset (CMD17/18 path).
                // Offset = xfer_len - dcount (bytes already drained).
                // (Direction from the CMD-time latch, not self.cmd —
                // see the write arm: cmd is already the NEXT command by
                // the time a completion-adjacent read lands.)
                if self.xfer_is_write {
                    self.fifo // write path: reads return last pushed word
                } else if self.data_xfer_active || self.xfer_len > 0 {
                    // Offset = bytes already served (fifo_done snapshot —
                    // survives completion, when DCOUNT reads 0 but the
                    // words stay in the FIFO for post-DATAEND draining).
                    let done = self.fifo_done as usize;
                    let block = self.arg;

                    let w = match &self.card {
                        Some(c) => {
                            let o = block as usize * 512 + done;
                            let mut b = [0xFFu8; 4];
                            for i in 0..4 {
                                b[i] = *c.get(o + i).unwrap_or(&0xFF);
                            }
                            u32::from_le_bytes(b)
                        }
                        None => 0xFFFF_FFFF,
                    };
                    if self.data_xfer_active && self.dcount > 0 {
                        self.dcount = self.dcount.saturating_sub(4);
                        self.fifocnt = self.dcount.min(512);
                    }
                    // Snapshot AFTER serving (next read serves done+4).
                    // Clamp to the transfer length (over-drain past the
                    // block reads erased — same as the image OOB path).
                    self.fifo_done = (self.fifo_done + 4).min(self.xfer_len);
                    self.fifo = w;
                    w
                } else {
                    self.fifo
                }
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.power = value & 3,
            0x04 => self.clkcr = value & 0x3FFF,
            0x08 => self.arg = value,
            0x0C => {
                self.cmd = value & 0xFFFF;
                if value & 0x40 != 0 {
                    let cmd_index = value as u8 & 0x3F;
                    let wait_type = (value >> 6) & 3;
                    self.respcmd = cmd_index as u32;
                    self.resp = [0; 4];
                    // ACMD prefix: this command arrived under APP_CMD.
                    let acmd = self.app_pending;
                    // The prefix is consumed by whatever comes next
                    // (ACMD or plain command alike — silicon clears it).
                    self.app_pending = false;

                    match (self.sd_state, cmd_index) {
                        (SdState::Idle, 0)  => { self.resp[0] = 0x00FF_FF80; }
                        (SdState::Idle, 2)  => { self.resp[0] = 0x00FF_FF80; self.sd_state = SdState::Ident; }
                        (SdState::Idle, 5)  => { self.resp[0] = 0x20_FF80; }
                        (SdState::Idle, 55) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Idle, _)  if cmd_index >= 41 && cmd_index <= 52 =>
                            { self.resp[0] = 0x50_FF80; self.sd_state = SdState::Ready; }
                        (SdState::Ready, 41) => { self.resp[0] = 0x50_FF80; }
                        (SdState::Ready, 55) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Ready, 5)  => { self.resp[0] = 0x20_FF80; }
                        (SdState::Ready, 2)  => { self.resp[0] = 0x00FF_FF80; self.sd_state = SdState::Ident; }
                        (SdState::Ident, 2)  => { self.resp[0] = 0x00FF_FF80; }
                        (SdState::Ident, 3)  => { self.rca = 0x01D0; self.resp[0] = 0x01D0_0000; self.sd_state = SdState::Stby; }
                        (SdState::Ident, 9)  => { self.resp[0] = 0x10_FFFF; self.resp[1] = 0x7F_FF80_9A; }
                        (SdState::Ident, 10) => { self.resp[0] = 0x10_FFFF; self.resp[1] = 0x7F_FF80_9A; }
                        (SdState::Stby, 7)  => { if (self.arg >> 16) as u16 == self.rca { self.resp[0] = 0x1D0_0000; self.sd_state = SdState::Tran; } }
                        (SdState::Stby, 3)  => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 7)  => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 13) => { self.resp[0] = 0x100; }
                        (SdState::Tran, 16) => { self.resp[0] = 0x200; }
                        (SdState::Tran, 17) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 18) => { self.resp[0] = 0x1D0_0000; }
                        // CMD24 WRITE_BLOCK: single-block host-to-card
                        // write. R1 response like a read; the data path
                        // arms below (shared with 17/18) with DTDIR=write
                        // semantics (DCTRL bit 1 set by firmware selects
                        // card-programming direction; TX words stream via
                        // the FIFO port and commit at completion).
                        (SdState::Tran, 24) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 55) => { self.resp[0] = 0x1D0_0000; }
                        (_, 8) => { self.resp[0] = 0x1AA; }
                        _ => {}
                    }
                    // ACMD handling (application-specific commands, only
                    // valid under the CMD55 prefix):
                    // - ACMD41 (SD_SEND_OP_COND): OCR busy bit follows the
                    //   voltage window — report ready (bit 31) with the
                    //   canned OCR; without the prefix it is an illegal
                    //   command (no response, like silicon).
                    // - ACMD6 (SET_BUS_WIDTH): ARG[1:0] latches WIDBUS
                    //   (1 = 4-bit, 2 = 8-bit); the data path is
                    //   width-agnostic, so the latch IS the contract.
                    // - ACMD13 (SD_STATUS): 512-bit status, canned ready.
                    // - ACMD51 (SEND_SCR): configuration register, canned.
                    if acmd {
                        match cmd_index {
                            41 => { self.resp[0] = 0x80FF_8000; }
                            6 => {
                                self.wide_bus = (self.arg & 3) as u8;
                                self.clkcr = (self.clkcr & !(3 << 11))
                                    | (((self.arg & 3) as u32) << 11);
                                self.resp[0] = 0x100;
                            }
                            13 => { self.resp[0] = 0x100; }
                            51 => { self.resp[0] = 0x100; }
                            _ => {}
                        }
                    } else if cmd_index == 41 || cmd_index == 51 {
                        // Bare ACMD without prefix: illegal (silicon sends
                        // no response). Clear the decode above.
                        if (self.sd_state, cmd_index) == (SdState::Ready, 41)
                            || (self.sd_state, cmd_index) == (SdState::Idle, 41)
                        {
                            self.resp = [0; 4];
                        }
                        if cmd_index == 51 {
                            self.resp = [0; 4];
                        }
                    }
                    // CMD55 latches the prefix for the NEXT command.
                    if cmd_index == 55 {
                        self.app_pending = true;
                    }

                    self.sta |= 1 << 6;
                    if wait_type != 0 { self.sta |= 1 << 10; }

                    if cmd_index == 17 || cmd_index == 18 || cmd_index == 24 {
                        // Latch the direction NOW (see xfer_is_write docs)
                        // and reset the FIFO drain snapshot (new transfer,
                        // FIFO refills from the block start).
                        self.xfer_is_write = cmd_index == 24;
                        self.fifo_done = 0;
                        // Data transfer: arm the timed completion (DATAEND/
                        // DBCKEND latch at data_done_at, not instantly).
                        // DTIMER timeout: if the programmed data timeout is
                        // shorter than the width-scaled cost, the transfer
                        // ends in DTIMEOUT instead (silicon watchdog).
                        use crate::system::instruction_count;
                        let now = instruction_count();
                        let cost = self.data_cost(self.dlen);
                        self.xfer_len = self.dlen;
                        self.data_start = now;
                        self.data_done_at = now.wrapping_add(cost);
                        self.data_xfer_active = true;
                        self.dcount = self.dlen;
                        // RXACT/TXACT + FIFO level reflect an ARMED transfer
                        // at once (firmware polls these before DATAEND);
                        // DBCKEND/DATAEND wait for the window.
                        self.sta |= (1 << 1) | (1 << 3) | (1 << 11);
                        self.sta |= (1 << 12) | (1 << 13); // TXACT+RXACT
                        self.fifocnt = self.dlen.min(512);
                        // Overrun/underrun staging: a zero-length transfer
                        // with the data path enabled is a firmware bug —
                        // silicon flags RXOVERR (read) / TXUNDERR (write)
                        // by direction. Latched now (not timed).
                        if self.dlen == 0 {
                            if self.dctrl & (1 << 1) != 0 {
                                self.sta |= 1 << 4; // TXUNDERR (to card)
                            } else {
                                self.sta |= 1 << 5; // RXOVERR (from card)
                            }
                        }
                        self.poll_data_done(sys);
                    }

                    self.fire_interrupts(sys);
                }
            }
            0x24 => self.dtimer = value,
            0x28 => { self.dlen = value & 0x1FF_FFFF; self.dcount = value & 0x1FF_FFFF; }
            0x2C => {
                self.dctrl = value & 0x1F3F;
                if value & 1 != 0 {
                    self.sta &= !0x3F;
                    self.data_xfer_active = false;
                    self.fifocnt = 0;
                    self.sta |= 1 << 3;
                    self.sta |= 1 << 5;
                    self.fire_interrupts(sys);
                }
            }
            0x38 => {
                self.sta &= !value;
                // SDIOITC (bit 22) clears the card-interrupt latch too.
                if value & (1 << 22) != 0 {
                    self.sdio_it = false;
                }
            }
            0x3C => {
                self.mask = value & 0x7FFF_FFFF;
                self.fire_interrupts(sys);
            }
            0x80 => {
                // FIFO port, direction-aware (latched at CMD time in
                // `xfer_is_write` — self.cmd is overwritten by the NEXT
                // command before completion polls, so re-deriving the
                // direction from cmd here would misroute: after CMD24
                // stages words, a following CMD17 would flip cmd to 17
                // while the CMD24 completion is still pending).
                // - reads (CMD17/18): a CPU-side word drains the
                //   in-flight transfer (DCOUNT/FIFOCNT follow down).
                // - CMD24 (host-to-card write): the word is STAGED (not
                //   drained): DCOUNT/FIFOCNT still follow down (silicon
                //   TXFIFO level), and the staged words commit into the
                //   card image at completion (see poll_data_done).
                if self.data_xfer_active && self.dcount > 0 {
                    self.dcount = self.dcount.saturating_sub(4);
                    self.fifocnt = self.dcount.min(512);
                    if self.xfer_is_write {
                        self.tx_stage.push(value);
                    }
                } else if self.xfer_is_write && self.data_xfer_active {
                    self.tx_stage.push(value);
                }
                self.fifo = value;
            }
            _ => {}
        }
    }
    fn tick(&mut self, sys: &System) {
        self.poll_data_done(sys);
    }
}

#[cfg(test)]
mod gap10_tests {
    use super::*;

    fn sdio() -> (std::rc::Rc<System>, Box<dyn Peripheral>) {
        let sys = crate::system::test_dummy_system();
        let mut s = Sdio::new("SDIO").unwrap();
        {
            let d = s.as_any_mut().downcast_mut::<Sdio>().unwrap();
            d.bind_card(4);
        }
        (sys, s)
    }

    fn cmd(sys: &System, s: &mut Box<dyn Peripheral>, idx: u8, arg: u32) {
        s.write(sys, 0x08, arg);
        s.write(sys, 0x0C, 0x40 | idx as u32);
    }

    // CMD24 single-block write commits staged FIFO words; CMD17 reads
    // them back through the FIFO port (image round-trip).
    #[test]
    fn cmd24_write_round_trips_through_cmd17() {
        use crate::system::INSTRUCTION_COUNT;
        use std::sync::atomic::Ordering;
        let (sys, mut s) = sdio();
        // Tran state: CMD7 select (rca programmed by CMD3 path is 0x1D0
        // only after Ident; force Tran via CMD7 with matching RCA after
        // walking Idle->Ident->Stby quickly).
        cmd(&sys, &mut s, 0, 0);
        cmd(&sys, &mut s, 2, 0);
        cmd(&sys, &mut s, 3, 0);
        s.write(&sys, 0x08, 0x01D0_0000);
        cmd(&sys, &mut s, 7, 0x01D0_0000);
        // CMD24 block 2: stage 4 words via FIFO, generous DTIMER.
        s.write(&sys, 0x24, 0xFFFFF);
        s.write(&sys, 0x28, 512);
        s.write(&sys, 0x2C, 0); // DTDIR=read-path but CMD24 forces TX stage
        cmd(&sys, &mut s, 24, 2);
        for w in [0x11111111u32, 0x22222222, 0x33333333, 0x44444444] {
            s.write(&sys, 0x80, w);
        }
        INSTRUCTION_COUNT.fetch_add(20000, Ordering::Relaxed);
        let sta = s.read(&sys, 0x34);
        assert_ne!(sta & (1 << 8), 0, "DATAEND after CMD24, sta={:#x}", sta);
        // CMD17 the same block back: first FIFO word = staged word 0.
        s.write(&sys, 0x28, 512);
        cmd(&sys, &mut s, 17, 2);
        INSTRUCTION_COUNT.fetch_add(20000, Ordering::Relaxed);
        let _ = s.read(&sys, 0x34);
        assert_eq!(s.read(&sys, 0x80), 0x11111111, "round-trip word 0");
        assert_eq!(s.read(&sys, 0x80), 0x22222222, "round-trip word 1");
    }
}
