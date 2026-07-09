use crate::system::System;
use super::Peripheral;

pub struct Wwdg {
    cr: u32,
    cfr: u32,
    sr: u32,
}

impl Wwdg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "WWDG" {
            Some(Box::new(Wwdg { cr: 0x7F, cfr: 0x7F, sr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Wwdg {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.cfr,
            0x08 => self.sr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0xFF,
            0x04 => self.cfr = value & 0x7FFF,
            0x08 => self.sr &= !(value & 1),
            _ => {}
        }
    }
}
