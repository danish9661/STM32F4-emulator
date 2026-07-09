use crate::system::System;
use super::Peripheral;

pub struct Flash {
    acr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    optcr: u32,
    optcr1: u32,
}

impl Default for Flash {
    fn default() -> Self {
        Self { acr: 0, keyr: 0, optkeyr: 0, sr: 0, cr: 0x8000_0000, optcr: 0x0C00_0000, optcr1: 0 }
    }
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FLASH" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.optkeyr,
            0x0C => self.sr,
            0x10 => self.cr,
            0x14 => self.optcr,
            0x18 => self.optcr1,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.acr = value,
            0x04 => self.keyr = value,
            0x08 => self.optkeyr = value,
            0x0C => self.sr = value,
            0x10 => self.cr = value,
            0x14 => self.optcr = value,
            0x18 => self.optcr1 = value,
            _ => {}
        }
    }
}
