use crate::{system::System, ext_devices::{ExtDevice, SpiDeviceEntry, ExtDevices}};
use super::Peripheral;

use std::{rc::Rc, cell::RefCell};

/// A hardware SPI peripheral with chip-select aware device routing.
#[derive(Default)]
pub struct Spi {
    pub name: String,
    pub cr1: u32,
    pub rx_buffer: u32,
    pub ready_toggle: bool,
    /// Devices attached to this SPI bus, each with an optional chip-select pin.
    devices: Vec<SpiDeviceEntry>,
}

impl Spi {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("SPI") {
            let mut devices = ext_devices.find_serial_devices(name);
            let name = if devices.is_empty() {
                // fallback: single-device lookup (backward compat)
                if let Some(d) = ext_devices.find_serial_device(name) {
                    let n = d.borrow_mut().connect_peripheral(name);
                    devices.push(SpiDeviceEntry { cs: None, device: d, name: n.clone() });
                    n
                } else {
                    name.to_string()
                }
            } else {
                for d in &mut devices {
                    d.name = d.device.borrow_mut().connect_peripheral(&d.name);
                }
                devices.first().map(|d| d.name.clone()).unwrap_or_default()
            };
            Some(Box::new(Self { name, devices, ..Default::default() }))
        } else {
            None
        }
    }

    pub fn is_16bits(&self) -> bool {
        self.cr1 & (1 << 11) != 0
    }

    /// Find the active device for this transfer.
    /// Returns the device whose CS pin is low, or the first device if no CS is specified.
    fn active_device(&self, sys: &System) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        let mut gpio = sys.p.gpio.borrow_mut();
        for d in &self.devices {
            let selected = match d.cs {
                Some((port, pin)) => {
                    (gpio.read_port(sys, port) >> pin) & 1 == 0
                }
                None => true,
            };
            if selected {
                return Some(d.device.clone());
            }
        }
        self.devices.first().map(|d| d.device.clone())
    }
}

impl Peripheral for Spi {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => self.cr1,
            0x0008 => {
                self.ready_toggle = !self.ready_toggle;
                if self.ready_toggle { 0b11 } else { 0 }
            }
            0x000C => {
                let v = self.rx_buffer;
                if self.is_16bits() {
                    trace!("{} read={:04x?}", self.name, v as u16);
                } else {
                    trace!("{} read={:02x?}", self.name, v as u8);
                }
                v
            }
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => self.cr1 = value,
            0x000C => {
                let device = self.active_device(sys);
                if let Some(ref d) = device {
                    let mut d = d.borrow_mut();
                    if self.is_16bits() {
                        d.write(sys, (), (value >> 8) as u8);
                        d.write(sys, (), value as u8);
                        trace!("{} write={:04x?}", self.name, value as u16);
                    } else {
                        let v = value as u8;
                        d.write(sys, (), v);
                        trace!("{} write={:02x?}", self.name, v);
                    }
                }
                self.rx_buffer = device.as_ref().and_then(|d| {
                    let mut d = d.borrow_mut();
                    Some(if self.is_16bits() {
                        let h = d.read(sys, ()) as u32;
                        let l = d.read(sys, ()) as u32;
                        (h << 8) | l
                    } else {
                        d.read(sys, ()) as u32
                    })
                }).unwrap_or(0);
            }
            _ => {}
        }
    }
}
