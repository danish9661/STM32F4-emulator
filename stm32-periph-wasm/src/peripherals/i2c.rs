use crate::{system::System, ext_devices::{ExtDevices, I2cDeviceEntry}};
use super::Peripheral;

#[derive(Clone, PartialEq)]
enum I2cState { Idle, StartSent, AddrSent { is_read: bool }, Active { is_read: bool } }

impl Default for I2cState { fn default() -> Self { I2cState::Idle } }

fn i2c_irqs(name: &str) -> Option<(i32, i32)> {
    match name {
        "I2C1" => Some((31, 32)),
        "I2C2" => Some((33, 34)),
        "I2C3" => Some((72, 73)),
        _ => None,
    }
}

#[derive(Clone)]
pub struct I2c {
    name: String,
    devices: Vec<I2cDeviceEntry>,
    active_device: Option<usize>,
    cr1: u32,
    cr2: u32,
    sr1: u32,
    sr2: u32,
    dr: u32,
    oar1: u32,
    oar2: u32,
    ccr: u32,
    trise: u32,
    state: I2cState,
    sr1_read_with_addr: bool,
    irq_ev: i32,
    irq_er: i32,
    /// SMBus/PEC engine: running CRC-8 (poly x^8+x^2+x+1, init 0) over
    /// address + data bytes of the current transaction (reset at START,
    /// like silicon's PEC counter). Checked/transmitted only when ENPEC
    /// (CR1 bit 5) is set; PECERR (SR1 bit 12) latches on mismatch.
    pec_acc: u8,
    /// Multi-master arbitration: when `arb_lose_next` is armed (harness =
    /// the other master), the next address phase loses arbitration — ARLO
    /// (SR1 bit 9) latches, the transaction aborts to Idle, no device is
    /// selected (silicon releases the bus; the winner's bytes never arrive).
    arb_lose_next: bool,
    /// SMBus timeout counter: consecutive Active-state ticks with SMBUS
    /// mode on (reset on state change / STOP). TIMEOUT latches at 32.
    smbus_ticks: u32,
    /// General-call transaction in progress (address 0x00 matched with
    /// ENGC): data bytes are broadcast (no device), GENCALL stays flagged
    /// in SR2 until STOP.
    smbus_gencall: bool,
    /// Harness-armed SMBus alert source address (host-notify): returned in
    /// DR on an Alert-Response-Address read. 0x00 = none pending.
    smbus_alert_addr: u8,
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            name: String::new(), devices: Vec::new(), active_device: None,
            cr1: 0, cr2: 0, sr1: 0, sr2: 0, dr: 0,
            oar1: 0, oar2: 0, ccr: 0, trise: 2,
            state: I2cState::Idle, sr1_read_with_addr: false,
            irq_ev: 0, irq_er: 0,
            pec_acc: 0,
            arb_lose_next: false,
            smbus_ticks: 0,
            smbus_gencall: false,
            smbus_alert_addr: 0,
        }
    }
}

impl I2c {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if !name.starts_with("I2C") { return None; }
        let (irq_ev, irq_er) = i2c_irqs(name)?;
        let devices = ext_devices.find_i2c_devices(name);
        Some(Box::new(Self { name: name.to_string(), devices, irq_ev, irq_er, ..Default::default() }))
    }

    fn reset(&mut self) {
        self.sr1 = 0; self.sr2 = 0; self.dr = 0;
        self.active_device = None; self.state = I2cState::Idle;
        self.sr1_read_with_addr = false;
        self.smbus_gencall = false;
    }

    /// CRC-8/SMBus step (poly 0x07, init 0 — the SMBus PEC polynomial):
    /// folds one byte into the running PEC accumulator.
    fn pec_feed(&mut self, b: u8) {
        let mut crc = self.pec_acc ^ b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
        self.pec_acc = crc;
    }

    /// Harness = the other master: arm arbitration loss on the next
    /// address phase (one-shot). Models a contending master winning the
    /// bus — the loser sees ARLO and releases SDA/SCL.
    pub fn arm_arb_loss(&mut self) {
        self.arb_lose_next = true;
    }

    /// Harness = the SMBus alerting device (host-notify source): arm the
    /// address returned in DR on the next Alert-Response-Address read.
    /// 0x00 clears (none pending).
    pub fn arm_smbus_alert(&mut self, addr: u8) {
        self.smbus_alert_addr = addr & 0x7F;
    }

    /// Current PEC accumulator (harness scope probe: firmware reads the
    /// transmitted PEC back via the DR path; the harness reads it here).
    pub fn pec(&self) -> u8 {
        self.pec_acc
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let itevten = (self.cr2 >> 10) & 1;
        let iterren = (self.cr2 >> 9) & 1;
        let itbufen = (self.cr2 >> 8) & 1;

        let ev_flags = self.sr1 & 0x17;
        let buf_flags = self.sr1 & 0x60;
        let err_flags = self.sr1 & 0x0E00;

        if ev_flags != 0 && itevten != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_ev);
        }
        if buf_flags != 0 && itbufen != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_ev);
        }
        if err_flags != 0 && iterren != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_er);
        }
    }

    /// SMBus special-address match (GCALL 0x00 / ALERT 0x0C / ARP 0x61):
    /// these addresses are answered at the MODEL level (no registered
    /// device needed — silicon answers them in hardware when the matching
    /// enable is set):
    /// - GCALL (general call, 0x00): answered iff ENGC (CR1 bit 6). Sets
    ///   GENCALL (SR2 bit 4) so firmware can tell it from a normal match.
    /// - ALERT response (0x0C): answered iff ALERT (CR1 bit 13, SMBus
    ///   alert). Sets SMBALERT (SR1 bit 15); firmware reads the alerting
    ///   device's address from DR (the model returns the harness-armed
    ///   alert address, default 0x00 = none pending).
    /// - ARP (0x61, bit-shifted 0xC2/0xC3 on the wire): answered iff
    ///   ENARP (CR1 bit 4). Returns HSTS-ready semantics: the transaction
    ///   proceeds and firmware runs the ARP command bytes itself.
    /// Returns true when the address was claimed by special handling.
    fn smbus_special_match(&mut self, sys: &System, addr: u8, is_read: bool) -> bool {
        if addr == 0x00 && self.cr1 & (1 << 6) != 0 {
            // General call: accept, flag GENCALL.
            self.active_device = None;
            self.sr1 = 1 << 1; // ADDR
            self.sr2 = (1 << 0) | (1 << 1) | (1 << 4); // MSL+BUSY+GENCALL
            self.smbus_gencall = true;
            self.state = I2cState::AddrSent { is_read };
            self.fire_interrupts(sys);
            return true;
        }
        if addr == 0x0C && self.cr1 & (1 << 13) != 0 {
            // Alert Response Address: accept, flag SMBALERT, DR carries
            // the harness-armed alerting address (host-notify source).
            self.active_device = None;
            self.sr1 = (1 << 1) | (1 << 15); // ADDR + SMBALERT
            self.sr2 = (1 << 0) | (1 << 1);
            if is_read {
                self.dr = self.smbus_alert_addr as u32;
            }
            self.state = I2cState::AddrSent { is_read };
            self.fire_interrupts(sys);
            return true;
        }
        if addr == 0x61 && self.cr1 & (1 << 4) != 0 {
            // ARP address: accept; firmware owns the command protocol.
            self.active_device = None;
            self.sr1 = 1 << 1; // ADDR
            self.sr2 = (1 << 0) | (1 << 1);
            self.state = I2cState::AddrSent { is_read };
            self.fire_interrupts(sys);
            return true;
        }
        false
    }
}

impl Peripheral for I2c {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x10 => {
                let v = self.dr;
                self.sr1 &= !(1 << 5);
                if let Some(idx) = self.active_device {
                    if matches!(self.state, I2cState::Active { is_read: true }) {
                        // Drop the device borrow before feeding PEC (both
                        // touch &mut self — the borrow checker sees the RefMut
                        // destructor as a use of the immutable borrow).
                        let nb = {
                            let mut d = self.devices[idx].device.borrow_mut();
                            d.read(sys, ()) as u8
                        };
                        // PEC receive path: when ENPEC + PEC positioned
                        // (CR1 PEC bit 12 set = "next byte is the PEC"),
                        // compare the wire byte against the accumulator and
                        // latch PECERR on mismatch (silicon checks, NACKs).
                        if self.cr1 & (1 << 5) != 0 && self.cr1 & (1 << 12) != 0 {
                            self.cr1 &= !(1 << 12);
                            if nb != self.pec_acc {
                                self.sr1 |= 1 << 12; // PECERR
                            }
                        } else {
                            if self.cr1 & (1 << 5) != 0 {
                                self.pec_feed(nb);
                            }
                            self.dr = nb as u32;
                        }
                        self.sr1 |= 1 << 5;
                    }
                }
                self.fire_interrupts(sys);
                v
            }
            0x14 => {
                self.sr1_read_with_addr = (self.sr1 & (1 << 1)) != 0;
                self.sr1
            }
            0x18 => {
                if self.sr1_read_with_addr {
                    self.sr1 &= !(1 << 1);
                    self.sr1_read_with_addr = false;
                    let is_read = match std::mem::replace(&mut self.state, I2cState::Idle) {
                        I2cState::AddrSent { is_read } => {
                            self.state = I2cState::Active { is_read };
                            is_read
                        }
                        s => { self.state = s; false }
                    };
                    if is_read {
                        self.sr1 |= 1 << 5;
                    } else {
                        self.sr1 |= 1 << 6;
                    }
                }
                self.fire_interrupts(sys);
                self.sr2
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let prev_start = self.cr1 & (1 << 8);
                let prev_pe = self.cr1 & 1;
                // START/STOP are action bits: a write carrying START or STOP
                // ORs the action into the stored CR1 so config bits (ENPEC,
                // SMBUS, ARP, ENGC, ALERT, ...) survive like silicon (a
                // START write of 0x101 must not clear ENPEC). A plain
                // config write (no START/STOP) replaces CR1 verbatim so
                // firmware CAN clear config bits (write 1 without them).
                // PE-clear and SWRST still take the full-write path below.
                if value & ((1 << 8) | (1 << 9)) != 0 {
                    let keep = self.cr1 & ((1 << 1) | (1 << 4) | (1 << 5) | (1 << 6) | (1 << 13));
                    self.cr1 = value | keep;
                } else {
                    self.cr1 = value;
                }

                if value & (1 << 15) != 0 {
                    self.reset();
                    self.cr1 = value & 1;
                    return;
                }
                if prev_pe != 0 && value & 1 == 0 {
                    self.reset();
                    return;
                }
                // DBGMCU freeze: while halted with this bus's SMBUS-timeout
                // freeze bit set, the bus clock is stopped — START/STOP edge
                // sequencing holds (writes latch CR1 but move no state, raise
                // no flags, push no tap events). Clears on resume with no
                // catch-up (same rule as TIM/WWDG/IWDG).
                if crate::peripherals::dbgmcu::dbgmcu_frozen(sys, &self.name) {
                    self.cr1 &= !((1 << 8) | (1 << 9));
                    return;
                }

                let start = value & (1 << 8);
                let stop = value & (1 << 9);

                if start != 0 && prev_start == 0 {
                    self.state = I2cState::StartSent;
                    self.sr1 = 1;
                    self.sr2 = (1 << 0) | (1 << 1);
                    self.active_device = None;
                    // PEC counter resets at START (silicon behavior); a
                    // repeated START mid-transaction restarts it too.
                    self.pec_acc = 0;
                    self.cr1 &= !(1 << 8);
                    crate::system::i2c_tap_push_event(&self.name, (1 << 31) | (1 << 30));
                    self.fire_interrupts(sys);
                }

                if stop != 0 {
                    if matches!(self.state, I2cState::Active { .. } | I2cState::AddrSent { .. }) {
                        self.reset();
                    }
                    self.cr1 &= !(1 << 9);
                    crate::system::i2c_tap_push_event(&self.name, 1 << 31);
                }
            }
            0x04 => {
                self.cr2 = value & 0x07FF;
            }
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x10 => {
                match self.state {
                    I2cState::StartSent => {
                        let addr = ((value >> 1) & 0x7F) as u8;
                        let is_read = (value & 1) != 0;

                        // Multi-master arbitration: an armed loss aborts
                        // here — ARLO latches, no device selected, bus
                        // released (state Idle). One-shot: disarm.
                        if self.arb_lose_next {
                            self.arb_lose_next = false;
                            self.sr1 = 1 << 9; // ARLO
                            self.sr2 = (1 << 0) | (1 << 1);
                            self.state = I2cState::Idle;
                            self.active_device = None;
                            self.fire_interrupts(sys);
                            return;
                        }

                        let found = self.devices.iter().position(|d| d.address == addr);

                        // SMBus special addresses first (GCALL/ALERT/ARP):
                        // answered in hardware when enabled, no device needed.
                        if found.is_none() && self.smbus_special_match(sys, addr, is_read) {
                            return;
                        }

                        if let Some(idx) = found {
                            self.active_device = Some(idx);
                            self.devices[idx].device.borrow_mut().reset();
                            // PEC covers the address byte too (silicon
                            // includes ADDR+R/W in the CRC when ENPEC set).
                            if self.cr1 & (1 << 5) != 0 {
                                self.pec_feed(value as u8);
                            }
                            self.sr1 = 1 << 1;
                            self.sr2 = (1 << 0) | (1 << 1);
                            if is_read {
                                let mut d = self.devices[idx].device.borrow_mut();
                                self.dr = d.read(sys, ()) as u32;
                            }
                            self.state = I2cState::AddrSent { is_read };
                        } else {
                            // No ACK: AF (acknowledge failure, SR1 bit 10)
                            // latches — ARLO (bit 9) is arbitration loss
                            // only (see the armed-loss path above).
                            self.sr1 = 1 << 10; // AF
                            self.sr2 = (1 << 0) | (1 << 1);
                            self.state = I2cState::Idle;
                        }
                        self.fire_interrupts(sys);
                    }
                    I2cState::Active { is_read: false } => {
                        // General-call broadcast: no device, bytes sink
                        // (firmware observes GENCALL in SR2, not data).
                        if self.smbus_gencall {
                            if self.cr1 & (1 << 5) != 0 {
                                self.pec_feed(value as u8);
                            }
                            self.sr1 |= 1 << 6;
                            self.fire_interrupts(sys);
                            return;
                        }
                        if let Some(idx) = self.active_device {
                            let mut d = self.devices[idx].device.borrow_mut();
                            d.write(sys, (), value as u8);
                        }
                        // PEC transmit path: ENPEC feeds every data byte;
                        // firmware sends the PEC itself via CR1 PEC bit 12
                        // (PEC transfer request) — the byte written IS the
                        // accumulated CRC, so feed-then-compare trivially
                        // holds; the observable is pec_acc + no PECERR.
                        if self.cr1 & (1 << 5) != 0 {
                            self.pec_feed(value as u8);
                        }
                        self.sr1 |= 1 << 6;
                        self.fire_interrupts(sys);
                    }
                    _ => {}
                }
            }
            0x1C => self.ccr = value & 0xFFF,
            0x20 => self.trise = value & 0x3F,
            _ => {}
        }
    }

    fn tick(&mut self, sys: &System) {
        // SMBus timeout: while SMBUS mode is on (CR1 bit 1), a transaction
        // held in Active with SCL effectively stuck latches TIMEOUT (SR1
        // bit 14). The model has no SCL line to watch, so the harness arms
        // the timeout by holding Active across N ticks with SMBUS set and
        // the freeze-free clock running — after 32 consecutive Active
        // ticks the TIMEOUT bit latches (one-shot per transaction; STOP
        // clears it via reset). Deterministic stand-in for the 25 ms
        // silicon rule; polled firmware observes the same flag.
        if self.cr1 & (1 << 1) != 0
            && matches!(self.state, I2cState::Active { .. })
            && !crate::peripherals::dbgmcu::dbgmcu_frozen(sys, &self.name)
        {
            self.smbus_ticks = self.smbus_ticks.wrapping_add(1);
            if self.smbus_ticks == 32 {
                self.sr1 |= 1 << 14; // TIMEOUT
                self.fire_interrupts(sys);
            }
        } else {
            self.smbus_ticks = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext_devices::i2c_regfile::{I2cRegFile, I2cRegFileConfig};

    const I2C1: u32 = 0x4000_5400;

    fn system_with_regfile() -> std::rc::Rc<crate::system::System> {
        let mut ext = crate::ext_devices::ExtDevices::default();
        ext.i2c_regfiles.push(std::rc::Rc::new(std::cell::RefCell::new(
            I2cRegFile::new(I2cRegFileConfig {
                peripheral: "I2C1".into(),
                address: 0x50,
                size: 16,
                init: vec![0; 16],
            }),
        )));
        crate::system::test_system_with(&ext)
    }

    fn w(sys: &std::rc::Rc<crate::system::System>, addr: u32, v: u32) {
        sys.p.write(sys, addr, 4, v);
    }
    fn r(sys: &std::rc::Rc<crate::system::System>, addr: u32) -> u32 {
        sys.p.read(sys, addr, 4)
    }

    /// START -> ADDR(write) -> SR1/SR2 latch -> one data byte, then read
    /// back the running PEC (ENPEC on). Proves the address byte feeds CRC.
    fn write_one(sys: &std::rc::Rc<crate::system::System>, addr_byte: u32, data: u8) {
        w(sys, I2C1, 1 | (1 << 8)); // PE + START
        w(sys, I2C1 + 0x10, addr_byte);
        assert_ne!(r(sys, I2C1 + 0x14) & (1 << 1), 0, "ADDR set");
        let _ = r(sys, I2C1 + 0x14);
        let _ = r(sys, I2C1 + 0x18); // SR2 latch -> Active
        w(sys, I2C1 + 0x10, data as u32);
    }

    #[test]
    fn pec_tx_accumulates_addr_plus_data() {
        let sys = system_with_regfile();
        // ENPEC (bit 5) + SMBUS (bit 1, so timeout logic stays out).
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5));
        write_one(&sys, 0xA0, 0x5A); // addr 0x50 write + data
        // CRC-8/SMBus over [0xA0, 0x5A], init 0.
        fn crc8(bytes: &[u8]) -> u8 {
            let mut crc = 0u8;
            for &b in bytes {
                crc ^= b;
                for _ in 0..8 {
                    crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
                }
            }
            crc
        }
        assert_eq!(sys.p.i2c_pec(I2C1), crc8(&[0xA0, 0x5A]), "PEC(addr,data)");
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 9)); // STOP (clean)
    }

    #[test]
    fn pec_rx_mismatch_latches_pecerr() {
        let sys = system_with_regfile();
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5)); // PE + SMBUS + ENPEC
        // Write path first (feeds addr into a *write* txn), then a read
        // txn whose PEC byte we deliberately corrupt via the position bit.
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 8)); // START
        w(&sys, I2C1 + 0x10, 0xA1); // addr 0x50 read
        assert_ne!(r(&sys, I2C1 + 0x14) & (1 << 1), 0, "ADDR set (read)");
        let _ = r(&sys, I2C1 + 0x14);
        let _ = r(&sys, I2C1 + 0x18);
        // Position the PEC check, then read one byte: regfile byte 0 is 0,
        // but pec_acc covers [0xA1] only — mismatch is near-certain unless
        // the CRC of [0xA1] happens to equal the data byte (it is 0x7B).
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 12)); // PEC position
        let _ = r(&sys, I2C1 + 0x10);
        assert_ne!(r(&sys, I2C1 + 0x14) & (1 << 12), 0, "PECERR on wrong PEC");
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 9)); // STOP (clean)
    }

    #[test]
    fn arb_loss_aborts_with_arlo_and_no_device() {
        let sys = system_with_regfile();
        w(&sys, I2C1, 1); // PE
        sys.p.i2c_arm_arb_loss(I2C1);
        w(&sys, I2C1, 1 | (1 << 8)); // START
        w(&sys, I2C1 + 0x10, 0xA0); // address phase loses
        let sr1 = r(&sys, I2C1 + 0x14);
        assert_ne!(sr1 & (1 << 9), 0, "ARLO latched");
        assert_eq!(sr1 & (1 << 1), 0, "no ADDR (no device selected)");
        // One-shot: next transaction proceeds normally.
        w(&sys, I2C1, 1 | (1 << 8)); // START
        w(&sys, I2C1 + 0x10, 0xA0);
        assert_ne!(r(&sys, I2C1 + 0x14) & (1 << 1), 0, "ADDR set after disarm");
        w(&sys, I2C1, 1 | (1 << 9)); // STOP (clean)
    }

    #[test]
    fn smbus_timeout_latches_after_32_active_ticks() {
        let sys = system_with_regfile();
        // SMBUS on (bit 1), no ENPEC needed.
        w(&sys, I2C1, 1 | (1 << 1));
        write_one(&sys, 0xA0, 0x00); // enter Active(write)
        assert_eq!(r(&sys, I2C1 + 0x14) & (1 << 14), 0, "no TIMEOUT yet");
        for _ in 0..31 {
            sys.p.peripherals.iter().for_each(|s| {
                if s.start == I2C1 {
                    s.peripheral.borrow_mut().tick(&sys);
                }
            });
        }
        assert_eq!(r(&sys, I2C1 + 0x14) & (1 << 14), 0, "still clean at 31");
        sys.p.peripherals.iter().for_each(|s| {
            if s.start == I2C1 {
                s.peripheral.borrow_mut().tick(&sys);
            }
        });
        assert_ne!(r(&sys, I2C1 + 0x14) & (1 << 14), 0, "TIMEOUT at 32");
        w(&sys, I2C1, 1 | (1 << 1) | (1 << 9)); // STOP clears via reset
        assert_eq!(r(&sys, I2C1 + 0x14) & (1 << 14), 0, "STOP clears TIMEOUT");
    }
}
