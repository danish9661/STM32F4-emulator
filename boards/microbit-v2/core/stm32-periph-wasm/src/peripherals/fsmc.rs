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
}

impl Bank {
    pub fn new(bank: usize, ext_devices: &ExtDevices) -> Self {
        let name = format!("FSMC.BANK{}", bank + 1);
        let ext_device = ext_devices.find_mem_device(bank);
        let name = ext_device.as_ref()
            .map(|d| d.borrow_mut().connect_peripheral(&name))
            .unwrap_or(name);
        Self { name, ext_device, bcr: 0, btr: 0 }
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
                    0x0104 => Access::Register(0, 2),
                    0x010C => Access::Register(1, 2),
                    0x0114 => Access::Register(2, 2),
                    0x011C => Access::Register(3, 2),
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
                    2 => {}
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
}
