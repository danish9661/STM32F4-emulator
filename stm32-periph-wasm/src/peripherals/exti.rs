use crate::system::System;
use super::Peripheral;

pub struct Exti {
    imr: u32, emr: u32, rtsr: u32, ftsr: u32, swier: u32, pr: u32,
}

impl Default for Exti {
    fn default() -> Self { Self { imr: 0, emr: 0, rtsr: 0, ftsr: 0, swier: 0, pr: 0 } }
}

impl Exti {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "EXTI" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Exti {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => { let v = self.swier; self.swier = 0; v }
            0x14 => self.pr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.imr = value & 0x7F_FFFF,
            0x04 => self.emr = value & 0x7F_FFFF,
            0x08 => self.rtsr = value & 0x7F_FFFF,
            0x0C => self.ftsr = value & 0x7F_FFFF,
            0x10 => {
                self.swier = value & 0x7F_FFFF;
                self.pr |= self.swier;
            }
            0x14 => self.pr &= !(value & 0x7F_FFFF),
            _ => {}
        }
    }
}
