use crate::system::System;
use super::Peripheral;

pub struct Dac { cr: u32, swtrigr: u32, dhr12r1: u32, dhr12l1: u32, dhr8r1: u32, dhr12r2: u32, dhr12l2: u32, dhr8r2: u32, dhr12rd: u32, dhr12ld: u32, dhr8rd: u32, dor1: u32, dor2: u32, sr: u32 }

impl Dac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DAC" { Some(Box::new(Self { ..Default::default() })) } else { None }
    }
}

impl Default for Dac { fn default() -> Self { unsafe { std::mem::zeroed() } } }

impl Peripheral for Dac {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr, 0x04 => self.swtrigr, 0x08 => self.dhr12r1, 0x0C => self.dhr12l1,
            0x10 => self.dhr8r1, 0x14 => self.dhr12r2, 0x18 => self.dhr12l2, 0x1C => self.dhr8r2,
            0x20 => self.dhr12rd, 0x24 => self.dhr12ld, 0x28 => self.dhr8rd, 0x2C => self.dor1,
            0x30 => self.dor2, 0x34 => self.sr, _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value, 0x04 => self.swtrigr = value,
            0x08 => self.dhr12r1 = value, 0x0C => self.dhr12l1 = value, 0x10 => self.dhr8r1 = value,
            0x14 => self.dhr12r2 = value, 0x18 => self.dhr12l2 = value, 0x1C => self.dhr8r2 = value,
            0x20 => self.dhr12rd = value, 0x24 => self.dhr12ld = value, 0x28 => self.dhr8rd = value,
            _ => {}
        }
    }
}
