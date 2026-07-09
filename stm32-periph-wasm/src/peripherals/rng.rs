use crate::system::System;
use super::Peripheral;

pub struct Rng { cr: u32, sr: u32, dr: u32 }

impl Rng {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RNG" { Some(Box::new(Rng { cr: 0, sr: 0x40, dr: 0 })) } else { None }
    }
}

impl Peripheral for Rng {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset { 0x00 => self.cr, 0x04 => self.sr, 0x08 => self.dr, _ => 0 }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset { 0x00 => self.cr = value, _ => {} }
    }
}
