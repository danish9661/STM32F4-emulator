use crate::system::System;
use super::Peripheral;

pub struct Flash {
    acr: u32,
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("FLASH") {
            Some(Box::new(Flash { acr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x0C => 0, // SR: no error
            _ => 0
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => { self.acr = value & 0x7FF; }
            0x04 | 0x08 => {} // KEYR, OPTKEYR: ignored
            _ => {}
        }
    }
}
