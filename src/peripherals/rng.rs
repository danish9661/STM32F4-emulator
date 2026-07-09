use crate::system::System;
use super::Peripheral;

pub struct Rng {
    cr: u32,
    sr: u32,
    dr: u32,
    lfsr: u32,
}

impl Rng {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RNG" {
            Some(Box::new(Rng { cr: 0, sr: 0, dr: 0, lfsr: 0xDEAD_BEEF }))
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
                    // Generate random number via LFSR
                    let bit = ((self.lfsr >> 31) ^ (self.lfsr >> 21) ^ (self.lfsr >> 1) ^ (self.lfsr >> 0)) & 1;
                    self.lfsr = (self.lfsr << 1) | bit;
                    self.dr = self.lfsr.wrapping_mul(0x9E37_79B9) ^ 0x1234_5678;
                    self.sr |= 1;
                } else {
                    self.sr &= !1;
                }
            }
            _ => {}
        }
    }
}
