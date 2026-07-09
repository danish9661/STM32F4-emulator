use crate::system::System;
use super::Peripheral;

pub struct Crc {
    dr: u32,
    idr: u32,
    cr: u32,
}

impl Crc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CRC" {
            Some(Box::new(Crc { dr: 0xFFFF_FFFF, idr: 0, cr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Crc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.dr,
            0x04 => self.idr,
            0x08 => self.cr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.dr = value,
            0x04 => self.idr = value & 0xFF,
            0x08 => {
                self.cr = value;
                if value & 1 != 0 {
                    self.dr = 0xFFFF_FFFF;
                    self.cr &= !1;
                }
            }
            _ => {}
        }
    }
}
