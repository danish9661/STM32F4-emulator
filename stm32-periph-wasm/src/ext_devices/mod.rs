pub mod spi_flash;
pub mod i2c_eeprom;

pub use spi_flash::SpiFlash;
pub use i2c_eeprom::I2cEeprom;

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
}

impl ExtDevices {
    pub fn find_serial_devices(&self, peri_name: &str) -> Vec<SpiDeviceEntry> {
        self.spi_flashes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .map(|d| SpiDeviceEntry {
                cs: d.borrow().config.cs.as_ref().map(|s| parse_pin(s)),
                device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                name: format!("{} spi-flash", peri_name),
            })
            .collect()
    }

    pub fn find_serial_device(&self, peri_name: &str) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        self.spi_flashes.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .next()
            .map(|d| d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>)
    }

    pub fn find_i2c_devices(&self, peri_name: &str) -> Vec<I2cDeviceEntry> {
        self.i2c_eeproms.iter()
            .filter(|d| d.borrow().config.peripheral == peri_name)
            .map(|d| I2cDeviceEntry {
                address: d.borrow().config.address,
                device: d.clone() as Rc<RefCell<dyn ExtDevice<(), u8>>>,
                name: format!("{} i2c-eeprom", peri_name),
            })
            .collect()
    }
}

pub trait ExtDevice<A, T> {
    fn connect_peripheral(&mut self, peri_name: &str) -> String;
    fn read(&mut self, sys: &crate::system::System, addr: A) -> T;
    fn write(&mut self, sys: &crate::system::System, addr: A, v: T);
    fn reset(&mut self) {}
}

// SAFETY: WASM is single-threaded; Rc/RefCell are safe
unsafe impl Send for ExtDevices {}
unsafe impl Sync for ExtDevices {}

fn parse_pin(s: &str) -> (u8, u8) {
    let re = regex::Regex::new(r"^P?([A-Za-z])(\d+)$").unwrap();
    let caps = re.captures(s).expect("Invalid pin format");
    let port = caps.get(1).unwrap().as_str().to_uppercase().chars().next().unwrap();
    let port = port as u8 - b'A';
    let pin: u8 = caps.get(2).unwrap().as_str().parse().unwrap();
    (port, pin)
}
