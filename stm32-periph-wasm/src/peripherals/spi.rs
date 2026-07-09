use crate::{system::System, ext_devices::{ExtDevice, SpiDeviceEntry, ExtDevices}};
use super::Peripheral;
use std::{rc::Rc, cell::RefCell};

#[derive(Default)]
pub struct Spi {
    pub name: String,
    pub cr1: u32,
    pub cr2: u32,
    pub srm: u32,
    pub dr: u32,
    pub rx_buffer: u32,
    pub ready_toggle: bool,
    pub i2scfgr: u32,
    pub i2spr: u32,
    devices: Vec<SpiDeviceEntry>,
}

impl Spi {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("SPI") {
            let mut devices = ext_devices.find_serial_devices(name);
            if devices.is_empty() {
                if let Some(d) = ext_devices.find_serial_device(name) {
                    let n = d.borrow_mut().connect_peripheral(name);
                    devices.push(SpiDeviceEntry { cs: None, device: d, name: n.clone() });
                }
            } else {
                for d in &mut devices {
                    d.name = d.device.borrow_mut().connect_peripheral(&d.name);
                }
            }
            Some(Box::new(Self { name: name.to_string(), devices, ..Default::default() }))
        } else { None }
    }

    pub fn is_16bits(&self) -> bool { self.cr1 & (1 << 11) != 0 }

    fn active_device(&self, sys: &System) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        let mut gpio = sys.p.gpio.borrow_mut();
        for d in &self.devices {
            let selected = match d.cs {
                Some((port, pin)) => (gpio.read_port(sys, port) >> pin) & 1 == 0,
                None => true,
            };
            if selected { return Some(d.device.clone()); }
        }
        self.devices.first().map(|d| d.device.clone())
    }
}

impl Peripheral for Spi {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => self.cr1,
            0x0004 => self.cr2,
            0x0008 => {
                self.ready_toggle = !self.ready_toggle;
                if self.ready_toggle { 0b11 } else { 0 }
            }
            0x000C => {
                let v = self.rx_buffer;
                self.rx_buffer = 0;
                v
            }
             0x0010 => self.dr,
             0x001C => self.i2scfgr,
             0x0020 => self.i2spr,
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => self.cr1 = value,
            0x0004 => self.cr2 = value,
            0x000C => {
                let device = self.active_device(sys);
                if let Some(ref d) = device {
                    let mut d = d.borrow_mut();
                    if self.is_16bits() {
                        d.write(sys, (), (value >> 8) as u8);
                        self.rx_buffer = (d.read(sys, ()) as u32) << 8;
                        d.write(sys, (), value as u8);
                        self.rx_buffer |= d.read(sys, ()) as u32;
                    } else {
                        let v = value as u8;
                        d.write(sys, (), v);
                        self.rx_buffer = d.read(sys, ()) as u32;
                    }
                } else {
                    self.rx_buffer = 0xFF;
                }
            }
             0x0010 => self.dr = value,
             0x001C => self.i2scfgr = value & 0xFFF,
             0x0020 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}
