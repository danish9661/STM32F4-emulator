use crate::system::{System, instruction_count};
use super::Peripheral;

pub struct Rng {
    cr: u32,
    sr: u32,
    dr: u32,
    last_regen: u64,
}

impl Default for Rng {
    fn default() -> Self {
        Self {
            sr: 0x40, // DRDY set
            dr: 0x0000_0000,
            last_regen: 0,
            cr: 0x0000_0000,
        }
    }
}

impl Rng {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RNG" { Some(Box::new(Self::default())) } else { None }
    }

    fn regenerate(&mut self) {
        let n = instruction_count();
        let elapsed = n.wrapping_sub(self.last_regen);
        if elapsed > 40 {
            let n32 = n as u32;
            self.dr = n32.wrapping_mul(1103515245).wrapping_add(12345);
            self.dr ^= self.dr >> 16;
            self.dr ^= self.dr << 5;
            self.sr = 0x40; // DRDY
            self.last_regen = n;
        }
    }
}

impl Peripheral for Rng {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => {
                let sr = self.sr;
                // Read SR clears error flags only
                self.sr &= !(0x06);
                sr
            }
            0x08 => {
                self.regenerate();
                let dr = self.dr;
                self.sr &= !0x40; // clear DRDY
                dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = value & 0x70007;
                if value & 0x40000 != 0 {
                    // CONDRST: reset the RNG
                    self.dr = 0;
                    self.sr = 0x40;
                }
                if value & 1 != 0 {
                    // RNGEN: enable - start generating
                    self.last_regen = instruction_count();
                    self.sr = 0x40;
                }
            }
            0x04 => {
                // Writing to SR clears specified bits
                self.sr &= !(value & 0x46);
            }
            0x08 => {} // DR is read-only
            _ => {}
        }
    }
}
