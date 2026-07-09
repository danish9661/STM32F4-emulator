use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
}

impl Default for Pwr {
    fn default() -> Self {
        Self {
            cr: 0x0000_0000,
            csr: 0x0000_0000,
        }
    }
}

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Pwr {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = (self.cr & 0xE000) | (value & 0x1FFF),
            0x04 => {}
            _ => {}
        }
    }
}
