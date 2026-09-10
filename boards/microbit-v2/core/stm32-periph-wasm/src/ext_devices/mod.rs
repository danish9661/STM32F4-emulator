pub mod spi_flash;
pub mod i2c_eeprom;
pub mod spi_tap;
pub mod i2c_tap;
pub mod i2c_regfile;
pub mod fsmc_tap;

pub use spi_flash::SpiFlash;
pub use i2c_eeprom::I2cEeprom;
pub use spi_tap::SpiTap;
pub use i2c_tap::I2cTap;
pub use i2c_regfile::I2cRegFile;
pub use fsmc_tap::FsmcTap;

use std::{rc::Rc, cell::RefCell};

pub struct SpiDeviceEntry {
    pub cs: Option<(u8, u8)>,
    pub device: Rc<RefCell<dyn ExtDevice<(), u8>>>,
    pub name: String,
}

#[derive(Clone)]
pub struct I2cDeviceEntry {
    pub address: u8,
    pub device: Rc<RefCell<dyn ExtDevice<(), u8>>>,
    pub name: String,
}

#[derive(Default)]
pub struct ExtDevices {
    pub spi_flashes: Vec<Rc<RefCell<SpiFlash>>>,
    pub i2c_eeproms: Vec<Rc<RefCell<I2cEeprom>>>,
    /// Protocol-agnostic SPI bus taps: every byte shifted while the device
    /// is CS-selected is queued for the JS hardware layer, and bytes pushed
    /// from JS are returned on the MISO line. Chip-side plumbing only — no
    /// device protocol knowledge lives here.
    pub spi_taps: Vec<Rc<RefCell<SpiTap>>>,
    /// Protocol-agnostic I2C slaves: an address acknowledged on the bus
    /// routes its bytes to the JS hardware layer. Chip-side plumbing only.
    pub i2c_taps: Vec<Rc<RefCell<I2cTap>>>,
    /// Pointer-addressed register files (DS3231 RTC style): first write byte
    /// = register pointer, then data auto-increments; reads follow the
    /// current pointer. Device-side protocol, model-side chip.
    pub i2c_regfiles: Vec<Rc<RefCell<I2cRegFile>>>,
    /// Protocol-agnostic FSMC bank taps: every data-space access to a tapped
    /// bank is queued for the JS hardware layer and reads are answered from a
    /// JS-pushed queue. Chip-side plumbing only.
    pub fsmc_taps: Vec<Rc<RefCell<FsmcTap>>>,
}

impl ExtDevices {
    pub fn find_serial_devices(&self, peri_name: &str) -> Vec<SpiDeviceEntry> {
        let mut result: Vec<SpiDeviceEntry> = Vec::new();
        for d in &self.spi_flashes {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: d.borrow().config.cs.as_ref().map(|s| parse_pin(s)),
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} spi-flash", peri_name),
                });
            }
        }
        for d in &self.spi_taps {
            if d.borrow().config.peripheral == peri_name {
                result.push(SpiDeviceEntry {
                    cs: d.borrow().config.cs.as_ref().map(|s| parse_pin(s)),
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} spi-tap", peri_name),
                });
            }
        }
        result
    }

    pub fn find_serial_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        self.spi_flashes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
        .or_else(||
        self.spi_taps.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>))
    }

    pub fn find_i2c_devices(&self, peri_name: &str) -> Vec<I2cDeviceEntry> {
        let mut out: Vec<I2cDeviceEntry> = self.i2c_eeproms.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .map(|d| I2cDeviceEntry {
                address: d.borrow().config.address,
                device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                name: format!("{} i2c-eeprom", peri_name),
            })
            .collect();
        for d in &self.i2c_taps {
            if d.borrow().config.peripheral == peri_name {
                out.push(I2cDeviceEntry {
                    address: d.borrow().config.address,
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} i2c-tap", peri_name),
                });
            }
        }
        for d in &self.i2c_regfiles {
            if d.borrow().config.peripheral == peri_name {
                out.push(I2cDeviceEntry {
                    address: d.borrow().config.address,
                    device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                    name: format!("{} i2c-regfile", peri_name),
                });
            }
        }
        out
    }

    /// The tap attached to FSMC bank `bank` (0 = BANK1), if any.
    pub fn find_mem_device(&self, bank: usize) -> Option<Rc<RefCell<dyn ExtDevice<u32, u32>>>> {
        self.fsmc_taps.iter()
            .find(|d| d.borrow().config.bank == bank)
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<u32, u32>>>)
    }
}

pub trait ExtDevice<A, T> {
    fn connect_peripheral(&mut self, peri_name: &str) -> String;
    fn read(&mut self, sys: &crate::system::System, addr: A) -> T;
    fn write(&mut self, sys: &crate::system::System, addr: A, v: T);
    fn reset(&mut self) {}
    /// Called when the SPI bus selects/deselects this device (CS edge).
    fn cs_changed(&mut self, _sys: &crate::system::System, _asserted: bool) {}
}

pub fn parse_pin(s: &str) -> (u8, u8) {
    let b = s.as_bytes();
    if b.len() >= 3 {
        let port = (b[1] as char).to_ascii_uppercase() as u8 - b'A';
        let pin: u8 = s[2..].trim_start_matches('0').parse().unwrap_or(0);
        (port, pin)
    } else {
        (0, 0)
    }
}

// SAFETY: WASM is single-threaded; Rc/RefCell are safe
unsafe impl Send for ExtDevices {}
unsafe impl Sync for ExtDevices {}
