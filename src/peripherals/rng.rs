use crate::system::System;
use super::Peripheral;

pub struct Rng {
    cr: u32,
    sr: u32,
    dr: u32,
}

impl Rng {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RNG" {
            Some(Box::new(Rng { cr: 0, sr: 1, dr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Rng {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.sr,
            0x08 => {
                self.sr &= !1;
                self.dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = value & 0x17;
                if self.cr & 4 != 0 {
                    self.sr |= 1;
                    self.dr = 0xDEAD_BEEF;
                }
            }
            _ => {}
        }
    }
}
