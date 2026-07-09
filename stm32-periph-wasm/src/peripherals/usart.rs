use crate::{system::System, ext_devices::ExtDevices};
use super::Peripheral;

#[derive(Default)]
pub struct Usart {
    sr: u32,
    dr: u32,
    brr: u32,
    cr1: u32,
    cr2: u32,
    cr3: u32,
    gtp: u32,
    name: String,
}

impl Usart {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("USART") || name.starts_with("UART") {
            Some(Box::new(Self { name: name.to_string(), ..Default::default() }))
        } else { None }
    }
}

impl Peripheral for Usart {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                self.sr |= 0b11000000;
                self.sr
            }
            0x04 => {
                self.sr &= !0b10000000;
                self.sr |= 0b01000000;
                self.dr
            }
            0x08 => self.brr,
            0x0C => self.cr1,
            0x10 => self.cr2,
            0x14 => self.cr3,
            0x18 => self.gtp,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.sr = value,
            0x04 => {
                self.dr = value & 0xFF;
                self.sr |= 0b11000000;
            }
            0x08 => self.brr = value,
            0x0C => self.cr1 = value,
            0x10 => self.cr2 = value,
            0x14 => self.cr3 = value,
            0x18 => self.gtp = value,
            _ => {}
        }
    }
}
