use crate::system::System;
use super::Peripheral;

#[derive(Clone)]
pub struct Adc {
    pub name: String,
    adc_idx: u8,
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    dr: u32,
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let adc_idx = match name {
            "ADC1" => Some(0),
            "ADC2" => Some(1),
            "ADC3" => Some(2),
            _ => None,
        }?;
        Some(Box::new(Self { name: name.to_string(), adc_idx, ..Self::default() }))
    }
}

impl Default for Adc {
    fn default() -> Self {
        Self {
            name: String::new(), adc_idx: 0,
            sr: 0, cr1: 0, cr2: 0, smpr1: 0, smpr2: 0,
            sqr1: 0, sqr2: 0, sqr3: 0, dr: 0,
        }
    }
}

impl Peripheral for Adc {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.sr,
            0x04 => self.cr1,
            0x08 => self.cr2,
            0x0C => self.smpr1,
            0x10 => self.smpr2,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x4C => {
                let channel = (self.sqr3 & 0x1F) as u8;
                self.dr = {
                    if let Some(ref n) = sys.n {
                        let n = n.borrow();
                        let analog = n.read_analog(self.adc_idx, channel);
                        (analog * 4095.0) as u32
                    } else { 0 }
                };
                self.sr |= 2;
                self.dr
            }
            0x50 => self.dr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.sr = value,
            0x04 => self.cr1 = value,
            0x08 => self.cr2 = value,
            0x0C => self.smpr1 = value,
            0x10 => self.smpr2 = value,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x4C => self.dr = value,
            0x50 => self.dr = value,
            _ => {}
        }
    }
}
