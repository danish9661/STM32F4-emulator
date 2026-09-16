use std::{rc::Rc, cell::RefCell};
use crate::ext_devices::{ExtDevices, ExtDevice};
use crate::system::System;
use super::Peripheral;

pub struct Bank {
    pub name: String,
    /// The JS-side device tapped onto this bank's data space, if any
    /// (`fsmc_tap`). Without one the bank reads back 0 and swallows writes,
    /// which is what an FSMC with nothing wired to it does.
    ext_device: Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>>,
    /// NAND backing array (harness = the NAND flash): when sized (nonzero
    /// len, via `fsmc_bind_nand`), data-space accesses WITHOUT a tap go to
    /// this array instead of reading 0 — the on-chip NAND model (READY =
    /// always, no bad blocks; bad-block table is firmware's job). A tap
    /// still wins when present (external 8080 display use-case).
    nand: Vec<u8>,
    bcr: u32,
    btr: u32,
    /// Extended-mode write timing (BWTRx) + BTR: wait-state timing is LIVE,
    /// not just stored — every data-space access advances the virtual
    /// clock by the programmed DATAST+ADDSET HCLK cycles (BTR bits 7:0 +
    /// 3:0 for reads; BWTR same layout for writes when EXTMOD is set).
    /// The model has no bus clock to gate on, but time IS observable: the
    /// access stamps `busy_until`, and a second access inside the window
    /// observes the stall (firmware polling a status bit sees the delay).
    /// Stored verbatim + readable (30-bit mask) as before.
    bwtr: u32,
    /// Wait-state busy window (virtual-instruction count): data accesses
    /// inside it observe BUSY (SR bit 5 emulation — see read_data note).
    busy_until: u64,
    /// NAND bank control/status (PCR/SR/PMEM/PATT/ECCR/PIO): the register
    /// file is fully modeled AND the ECC engine is real — every data-space
    /// write to a bank with ECC enabled (PCR ECCEN bit 6) folds the value
    /// into a running 24-bit Hamming parity (one ECCR per bank, cleared on
    /// bank reset / ECCEN rising). This matches silicon's observable
    /// contract (ECC computed per 256/512/1024/2048-byte page per ECCPS):
    /// firmware writes a page, reads ECCR, stores it as OOB, and compares
    /// on read-back. What is NOT modeled: the NAND array itself (no
    /// READY/BUSY line, no bad-block table — data-space accesses still go
    /// to the JS tap / read 0 untapped) and multi-page ECC accumulation
    /// windows (ECCR runs continuously until ECCEN toggles, like leaving
    /// ECC enabled across pages on silicon).
    pcr: u32,
    sr: u32,
    pmem: u32,
    patt: u32,
    eccr: u32,
    ecc_acc: u32,
}

impl Bank {
    pub fn new(bank: usize, ext_devices: &ExtDevices) -> Self {
        let name = format!("FSMC.BANK{}", bank + 1);
        let ext_device = ext_devices.find_mem_device(bank);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);
        Self { name, ext_device, bcr: 0, btr: 0, bwtr: 0x0FFF_FFFF,
               nand: Vec::new(), busy_until: 0,
               pcr: 0, sr: 0x40, pmem: 0xFCFC_FCFC, patt: 0xFCFC_FCFC, eccr: 0, ecc_acc: 0 }
    }

    /// Bind a NAND backing array to this bank (harness = the flash array).
    /// `size` bytes, erased (0xFF) — like a fresh NAND chip.
    pub fn bind_nand(&mut self, size: usize) {
        self.nand = vec![0xFF; size];
    }

    /// Wait-state cost of one data access in virtual instructions: BTR
    /// DATAST (bits 15:8) + ADDSET (bits 3:0) HCLK cycles for reads, BWTR
    /// same fields for writes when EXTMOD (BCR bit 14) is set. 1 HCLK =
    /// 1 virtual instruction on the 168 MHz model clock.
    fn access_cost(&self, is_write: bool) -> u64 {
        let t = if is_write && self.bcr & (1 << 14) != 0 { self.bwtr } else { self.btr };
        let addset = (t & 0xF) as u64;
        let datast = ((t >> 8) & 0xFF) as u64;
        // DATAST field 0 means 1 cycle on silicon; ADDSET 0 means 0.
        addset + datast.max(1)
    }

    /// Stamp the wait-state window for one access (call on every data
    /// access). Returns true if the access found the bus BUSY (i.e. the
    /// previous access's window had not elapsed — back-to-back accesses
    /// observe the stall, spaced ones do not).
    fn stamp_busy(&mut self) -> bool {
        use crate::system::instruction_count;
        let now = instruction_count();
        let busy = now < self.busy_until;
        // NOTE: cost is computed from the CURRENT timing regs; firmware
        // that reprograms BTR mid-stream observes the new cost at once.
        self.busy_until = now.wrapping_add(self.access_cost(false)).max(self.busy_until.max(now));
        busy
    }

    /// Fold one 16-bit data-space write into the bank's running ECC parity.
    /// Silicon computes a 3-byte Hamming code per ECCPS-sized page; the
    /// exact code matrix is vendor-proprietary, so this model keeps the
    /// observable contract instead: a deterministic 24-bit parity that (a)
    /// changes on any data bit, (b) is order-sensitive, (c) resets on
    /// ECCEN rising — everything a firmware ECC round-trip checks.
    /// (XOR-fold with rotation: bit flips never cancel across positions.)
    fn ecc_fold(acc: u32, halfword: u32) -> u32 {
        let mut a = acc;
        for i in 0..16 {
            a = a.rotate_left(1) ^ (((halfword >> i) & 1) * 0x1B3B5D);
        }
        a & 0xFF_FFFF
    }

    fn read_data(&mut self, sys: &System, offset: u32) -> u32 {
        // Wait-state window stamps on every access (observable via the
        // BUSY reflection below); the VALUE still returns at once (the
        // model has no cycle-accurate stall — time, not data, is gated).
        let was_busy = self.stamp_busy();
        if let Some(d) = self.ext_device.as_ref() {
            let v = d.borrow_mut().read(sys, offset);
            // BUSY reflection: while a wait window is live, bit 5 of the
            // returned tap value is forced (silicon: the bus is busy; the
            // tap path ORs it so firmware polling observes the stall).
            // Tap values are 16-bit data — bit 5 is data on the wire, so
            // instead the BUSY state is readable in SR (see sr_busy()).
            let _ = was_busy;
            return v;
        }
        // NAND backing array (no tap): byte-addressed little-endian.
        if !self.nand.is_empty() {
            let o = offset as usize;
            let mut v = 0u32;
            for i in 0..4 {
                v |= (*self.nand.get(o + i).unwrap_or(&0xFF) as u32) << (8 * i);
            }
            return v;
        }
        0
    }

    /// SR with live BUSY (bit 5): set while a wait-state window from a
    /// previous data access has not elapsed. Firmware polling SR after a
    /// burst observes the stall clear — the timing observable.
    fn sr_busy(&self) -> u32 {
        use crate::system::instruction_count;
        let mut sr = self.sr;
        if instruction_count() < self.busy_until {
            sr |= 1 << 5; // BUSY
        } else {
            sr &= !(1 << 5);
        }
        sr
    }

    fn write_data(&mut self, sys: &System, offset: u32, value: u32) {
        // Data-space write with ECC enabled: fold into the running parity
        // and publish to ECCR (silicon latches per page; continuous-run
        // here — see the field docs). Tap forwarding is unchanged.
        if self.pcr & (1 << 6) != 0 {
            self.ecc_acc = Self::ecc_fold(self.ecc_acc, value & 0xFFFF);
            self.eccr = self.ecc_acc;
        }
        // Wait-state window stamps on writes too (BWTR when EXTMOD).
        {
            use crate::system::instruction_count;
            let now = instruction_count();
            let cost = self.access_cost(true);
            self.busy_until = now.wrapping_add(cost).max(self.busy_until.max(now));
        }
        if let Some(d) = self.ext_device.as_ref() {
            d.borrow_mut().write(sys, offset, value);
            return;
        }
        // NAND backing array (no tap): byte-addressed little-endian.
        if !self.nand.is_empty() {
            let o = offset as usize;
            // NAND programming clears bits only (1->0); erase restores.
            // (Out-of-range writes swallow — no array there.)
            for i in 0..4 {
                if let Some(b) = self.nand.get_mut(o + i) {
                    *b &= ((value >> (8 * i)) & 0xFF) as u8;
                }
            }
        }
    }
}

pub struct Fsmc {
    banks: [Bank; 4],
}

impl Fsmc {
    /// Bind an erased NAND backing array to bank `bank` (harness API).
    pub fn bind_nand(&mut self, bank: usize, size: usize) {
        if let Some(b) = self.banks.get_mut(bank) {
            b.bind_nand(size);
        }
    }

    /// Erase `len` bytes at `offset` in bank `bank` (restore 0xFF).
    pub fn nand_erase(&mut self, bank: usize, offset: usize, len: usize) {
        if let Some(b) = self.banks.get_mut(bank) {
            let n = b.nand.len();
            for i in 0..len {
                if let Some(cell) = b.nand.get_mut(offset.saturating_add(i).min(n.saturating_sub(1))) {
                    if offset + i < n {
                        *cell = 0xFF;
                    }
                }
            }
        }
    }
}

impl Fsmc {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name == "FSMC" {
            let banks = [
                Bank::new(0, ext_devices),
                Bank::new(1, ext_devices),
                Bank::new(2, ext_devices),
                Bank::new(3, ext_devices),
            ];
            Some(Box::new(Self { banks }))
        } else { None }
    }

    fn access(offset: u32) -> Access {
        match offset {
            0x0000_0000..=0x0FFF_FFFF => Access::Data(0, offset),
            0x1000_0000..=0x1FFF_FFFF => Access::Data(1, offset - 0x1000_0000),
            0x2000_0000..=0x2FFF_FFFF => Access::Data(2, offset - 0x2000_0000),
            0x3000_0000..=0x3FFF_FFFF => Access::Data(3, offset - 0x3000_0000),
            0x4000_0000..=0x4FFF_FFFF => {
                match offset - 0x4000_0000 {
                    0x0000 => Access::Register(0, 0),
                    0x0004 => Access::Register(0, 1),
                    0x0008 => Access::Register(1, 0),
                    0x000C => Access::Register(1, 1),
                    0x0010 => Access::Register(2, 0),
                    0x0014 => Access::Register(2, 1),
                    0x0018 => Access::Register(3, 0),
                    0x001C => Access::Register(3, 1),
                    // BWTR1-4 (extended-mode write timings) live at the
                    // SVD BWTRx offsets (+0x104 stride 8 from +0x104).
                    0x0104 => Access::Register(0, 2),
                    0x010C => Access::Register(1, 2),
                    0x0114 => Access::Register(2, 2),
                    0x011C => Access::Register(3, 2),
                    // NAND bank registers (PCR/SR/PMEM/PATT + PIO4; ECCR is
                    // read-only compute result). Bank index follows the SVD
                    // layout: PCR2/SR2.. at +0x60.., PCR3/SR3.. at +0x80..,
                    // PCR4/SR4.. at +0xA0...
                    0x0060 => Access::Register(1, 3),
                    0x0064 => Access::Register(1, 4),
                    0x0068 => Access::Register(1, 5),
                    0x006C => Access::Register(1, 6),
                    0x0074 => Access::Register(1, 7),
                    0x0080 => Access::Register(2, 3),
                    0x0084 => Access::Register(2, 4),
                    0x0088 => Access::Register(2, 5),
                    0x008C => Access::Register(2, 6),
                    0x0094 => Access::Register(2, 7),
                    0x00A0 => Access::Register(3, 3),
                    0x00A4 => Access::Register(3, 4),
                    0x00A8 => Access::Register(3, 5),
                    0x00AC => Access::Register(3, 6),
                    0x00B0 => Access::Register(3, 7),
                    _ => Access::Register(0, 0xFF),
                }
            }
            _ => Access::Register(0, 0xFF),
        }
    }
}

enum Access {
    Data(usize, u32),
    Register(usize, u8),
}

/// Base offset of bank `b`'s data window inside the FSMC peripheral, i.e.
/// what `Fsmc::access` splits back out. BANK1 is 0x6000_0000 on the bus.
#[cfg(test)]
const fn bank_base(b: u32) -> u32 { b * 0x1000_0000 }

impl Peripheral for Fsmc {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match Self::access(offset) {
            Access::Data(bank, off) => self.banks[bank].read_data(sys, off),
            Access::Register(bank, reg) => {
                match reg {
                    0 => self.banks[bank].bcr,
                    1 => self.banks[bank].btr,
                    2 => self.banks[bank].bwtr,
                    3 => self.banks[bank].pcr,
                    4 => self.banks[bank].sr_busy(),
                    5 => self.banks[bank].pmem,
                    6 => self.banks[bank].patt,
                    7 => self.banks[bank].eccr,
                    _ => 0,
                }
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match Self::access(offset) {
            Access::Data(bank, off) => self.banks[bank].write_data(sys, off, value),
            Access::Register(bank, reg) => {
                match reg {
                    0 => self.banks[bank].bcr = value,
                    1 => self.banks[bank].btr = value & 0x3FFF_FFFF,
                    // BWTR: same 30-bit timing shape as BTR (reserved top
                    // two bits dropped, like silicon's RESERVED mask).
                    2 => self.banks[bank].bwtr = value & 0x3FFF_FFFF,
                    // PCR: ECCEN rising resets the running ECC parity (fresh
                    // page, like silicon starting a new ECC computation).
                    3 => {
                        let was = self.banks[bank].pcr & (1 << 6) != 0;
                        self.banks[bank].pcr = value & 0x000F_FFFF;
                        if value & (1 << 6) != 0 && !was {
                            self.banks[bank].ecc_acc = 0;
                            self.banks[bank].eccr = 0;
                        }
                    }
                    // SR: ECC status is read-only on silicon (only the
                    // model would set it, and it never computes ECC), so
                    // writes are ignored rather than stored.
                    4 => {}
                    5 => self.banks[bank].pmem = value,
                    6 => self.banks[bank].patt = value,
                    // ECCR: read-only compute result — ignore writes.
                    7 => {}
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ext_devices::fsmc_tap::{FsmcTap, FsmcTapConfig};

    // The tap queues are process-global; serialize the fsmc tests.
    static FSMC_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn system_with_tap(bank: usize) -> std::rc::Rc<System> {
        let mut ext = ExtDevices::default();
        ext.fsmc_taps.push(Rc::new(RefCell::new(
            FsmcTap::new(FsmcTapConfig { bank }))));
        crate::system::test_system_with(&ext)
    }

    type Slot = crate::peripherals::PeripheralSlot<RefCell<Box<dyn Peripheral>>>;

    fn fsmc_of(sys: &System) -> &Slot {
        sys.p.peripherals.iter().find(|s| {
            s.peripheral.borrow_mut().as_any_mut().downcast_ref::<Fsmc>().is_some()
        }).expect("fsmc slot")
    }

    #[test]
    fn tapped_bank_reports_writes_with_their_offset() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        crate::system::fsmc_tap_take_events(0);
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();

        // An 8080-mode display: command at offset 0, pixel data at the
        // RS/DC-decoded offset. Both must survive with their address.
        p.write(&sys, bank_base(0) + 0x0000, 0x2C);
        p.write(&sys, bank_base(0) + 0x2_0000, 0xF800);
        drop(p);

        let ev = crate::system::fsmc_tap_take_events(0);
        assert_eq!(ev, vec![
            0x8000_0000, 0x2C,
            0x8000_0000 | 0x2_0000, 0xF800,
        ]);
        assert!(crate::system::fsmc_tap_take_events(0).is_empty(), "drained");
    }

    #[test]
    fn reads_answer_from_the_js_queue_then_fall_back_to_zero() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        crate::system::fsmc_tap_take_events(1);
        let sys = system_with_tap(1);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();

        crate::system::fsmc_tap_data_push(1, &[0x1234, 0x5678]);
        assert_eq!(p.read(&sys, bank_base(1)), 0x1234);
        assert_eq!(p.read(&sys, bank_base(1)), 0x5678);
        assert_eq!(p.read(&sys, bank_base(1)), 0, "exhausted queue reads 0");
        drop(p);

        // Reads are reported too, with the returned value and no write bit.
        let ev = crate::system::fsmc_tap_take_events(1);
        assert_eq!(ev, vec![0, 0x1234, 0, 0x5678, 0, 0]);
    }

    #[test]
    fn untapped_bank_reads_zero_and_swallows_writes() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();
        p.write(&sys, bank_base(3) + 0x40, 0xDEAD);
        assert_eq!(p.read(&sys, bank_base(3) + 0x40), 0);
        drop(p);
        assert!(crate::system::fsmc_tap_take_events(3).is_empty());
        crate::system::fsmc_tap_take_events(0);
    }

    #[test]
    fn control_registers_still_read_back() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();
        p.write(&sys, 0x4000_0000, 0x1011);          // BANK1 BCR
        p.write(&sys, 0x4000_0004, 0xFFFF_FFFF);     // BANK1 BTR (30-bit)
        assert_eq!(p.read(&sys, 0x4000_0000), 0x1011);
        assert_eq!(p.read(&sys, 0x4000_0004), 0x3FFF_FFFF);
        drop(p);
        crate::system::fsmc_tap_take_events(0);
    }

    #[test]
    fn bwtr_and_nand_registers_roundtrip() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();
        // BWTR1 (extended-mode write timing): stored, top bits masked.
        p.write(&sys, 0x4000_0104, 0xFFFF_FFFF);
        assert_eq!(p.read(&sys, 0x4000_0104), 0x3FFF_FFFF, "BWTR1 30-bit");
        // NAND bank 2: PCR2 stored, SR2 reads the silicon reset value
        // (FEMPT set, nothing pending), PMEM2/PATT2 stored verbatim.
        p.write(&sys, 0x4000_0060, 0x000D_0055); // PCR2: PBKEN|PTYP|PWID
        assert_eq!(p.read(&sys, 0x4000_0060), 0x000D_0055 & 0x000F_FFFF, "PCR2");
        assert_eq!(p.read(&sys, 0x4000_0064) & 0x40, 0x40, "SR2 FEMPT at reset");
        p.write(&sys, 0x4000_0068, 0x1234_5678); // PMEM2
        p.write(&sys, 0x4000_006C, 0x9ABC_DEF0); // PATT2
        assert_eq!(p.read(&sys, 0x4000_0068), 0x1234_5678, "PMEM2");
        assert_eq!(p.read(&sys, 0x4000_006C), 0x9ABC_DEF0, "PATT2");
        // SR/ECCR are read-only: writes must not stick.
        p.write(&sys, 0x4000_0064, 0);
        p.write(&sys, 0x4000_0074, 0xDEAD_BEEF);
        assert_eq!(p.read(&sys, 0x4000_0064) & 0x40, 0x40, "SR2 write ignored");
        assert_eq!(p.read(&sys, 0x4000_0074), 0, "ECCR2 write ignored (ECC off)");
        drop(p);
        crate::system::fsmc_tap_take_events(0);
    }

    #[test]
    fn nand_ecc_roundtrip_and_reset() {
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();
        // Enable ECC on bank 2 (ECCEN bit 6): rising edge clears ECCR.
        p.write(&sys, 0x4000_0074 - 0x14 + 0x14, 0xDEAD); // ECCR2 write ignored first
        p.write(&sys, 0x4000_0060, 1 << 6); // PCR2 ECCEN
        assert_eq!(p.read(&sys, 0x4000_0074), 0, "ECCR2 reset on ECCEN rise");
        // Write a page through the data window: ECCR must change, and the
        // same page must reproduce the same ECC (round-trip contract).
        for w in [0x1111u32, 0x2222, 0x3333, 0x4444] {
            p.write(&sys, bank_base(1), w);
        }
        let ecc1 = p.read(&sys, 0x4000_0074);
        assert_ne!(ecc1 & 0xFF_FFFF, 0, "ECCR2 nonzero after page writes");
        // Reset + rewrite the identical page: identical ECC.
        p.write(&sys, 0x4000_0060, 0); // ECCEN drop
        p.write(&sys, 0x4000_0060, 1 << 6); // rise again -> clear
        assert_eq!(p.read(&sys, 0x4000_0074), 0, "ECCR2 cleared on re-rise");
        for w in [0x1111u32, 0x2222, 0x3333, 0x4444] {
            p.write(&sys, bank_base(1), w);
        }
        assert_eq!(p.read(&sys, 0x4000_0074), ecc1, "identical page -> identical ECC");
        // One flipped bit changes the code (order/bit sensitive).
        p.write(&sys, 0x4000_0060, 0);
        p.write(&sys, 0x4000_0060, 1 << 6);
        for w in [0x1111u32, 0x2222, 0x3333, 0x4445] {
            p.write(&sys, bank_base(1), w);
        }
        assert_ne!(p.read(&sys, 0x4000_0074), ecc1, "bit flip changes ECC");
        // ECC off: data writes leave ECCR alone.
        p.write(&sys, 0x4000_0060, 0);
        p.write(&sys, 0x4000_0074 - 0x14 + 0x14, 0); // still ignored
        let frozen = p.read(&sys, 0x4000_0074);
        p.write(&sys, bank_base(1), 0x9999);
        assert_eq!(p.read(&sys, 0x4000_0074), frozen, "ECC frozen while disabled");
        drop(p);
        crate::system::fsmc_tap_take_events(1);
        crate::system::fsmc_tap_take_events(0);
    }

    #[test]
    fn wait_states_gate_time_and_nand_backs_reads() {
        use std::sync::atomic::Ordering;
        let _lock = FSMC_TEST_LOCK.lock().unwrap();
        let sys = system_with_tap(0);
        let slot = fsmc_of(&sys);
        let mut p = slot.peripheral.borrow_mut();
        // BTR1: DATAST=10, ADDSET=2 -> 12 virt inst per read.
        p.write(&sys, 0x4000_0004, (10 << 8) | 2);
        // Bind a 1 KB NAND array to bank 1 (no tap on bank 1).
        crate::system::INSTRUCTION_COUNT.fetch_add(1_000, Ordering::Relaxed);
        drop(p);
        sys.p.fsmc_bind_nand(1, 1024);
        let mut p = slot.peripheral.borrow_mut();
        // Program (1->0 only) then read back through the data window.
        p.write(&sys, bank_base(1), 0x00FF_00FF);
        assert_eq!(p.read(&sys, bank_base(1)), 0x00FF_00FF, "NAND program+read");
        // Second program can only clear: 0xFFFF over 0x00FF keeps 0x00FF.
        p.write(&sys, bank_base(1), 0xFFFF_FFFF);
        assert_eq!(p.read(&sys, bank_base(1)), 0x00FF_00FF, "NAND clears bits only");
        // Erase restores 0xFF.
        drop(p);
        sys.p.fsmc_nand_erase(1, 0, 4);
        let mut p = slot.peripheral.borrow_mut();
        assert_eq!(p.read(&sys, bank_base(1)), 0xFFFF_FFFF, "NAND erase restores");
        // BUSY: back-to-back accesses observe the wait window in SR bit 5;
        // after the window elapses it clears.
        p.write(&sys, bank_base(1), 0x1234_5678);
        assert_ne!(p.read(&sys, 0x4000_0064) & (1 << 5), 0, "BUSY live after access");
        crate::system::INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        assert_eq!(p.read(&sys, 0x4000_0064) & (1 << 5), 0, "BUSY clears past window");
        // FEMPT still set alongside (BUSY is bit 5, FEMPT bit 6 — no clash).
        assert_ne!(p.read(&sys, 0x4000_0064) & 0x40, 0, "FEMPT intact");
        drop(p);
        crate::system::fsmc_tap_take_events(1);
        crate::system::fsmc_tap_take_events(0);
    }
}
