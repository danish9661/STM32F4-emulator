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
    bcr: u32,
    btr: u32,
    /// Extended-mode write timing (BWTRx). Stored only — the emulator has
    /// no bus clock to time against, so timing values are accepted and
    /// readable but never gate an access (same policy as BTR).
    bwtr: u32,
    /// NAND bank control/status (PCR/SR/PMEM/PATT/ECCR/PIO): stored register
    /// file only. The model has no NAND flash behind these banks, so ECC is
    /// never computed and no READY/BUSY line is simulated; the registers
    /// exist so NAND drivers can configure and poll without faulting.
    pcr: u32,
    sr: u32,
    pmem: u32,
    patt: u32,
    eccr: u32,
}

impl Bank {
    pub fn new(bank: usize, ext_devices: &ExtDevices) -> Self {
        let name = format!("FSMC.BANK{}", bank + 1);
        let ext_device = ext_devices.find_mem_device(bank);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);
        Self { name, ext_device, bcr: 0, btr: 0, bwtr: 0x0FFF_FFFF,
               pcr: 0, sr: 0x40, pmem: 0xFCFC_FCFC, patt: 0xFCFC_FCFC, eccr: 0 }
    }

    fn read_data(&mut self, sys: &System, offset: u32) -> u32 {
        self.ext_device.as_ref().map(|d| d.borrow_mut().read(sys, offset)).unwrap_or(0)
    }

    fn write_data(&mut self, sys: &System, offset: u32, value: u32) {
        if let Some(d) = self.ext_device.as_ref() {
            d.borrow_mut().write(sys, offset, value);
        }
    }
}

pub struct Fsmc {
    banks: [Bank; 4],
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
                    4 => self.banks[bank].sr,
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
                    3 => self.banks[bank].pcr = value & 0x000F_FFFF,
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
        assert_eq!(p.read(&sys, 0x4000_0074), 0, "ECCR2 write ignored");
        drop(p);
        crate::system::fsmc_tap_take_events(0);
    }
}
