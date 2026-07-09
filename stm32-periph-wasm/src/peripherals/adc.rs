use crate::system::System;
use super::Peripheral;

#[derive(Default)]
pub struct Adc {
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    jsqr: u32,
    jdr: [u32; 4],
    dr: u32,
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("ADC") {
            Some(Box::new(Adc {
                cr2: 0x0000_0001,
                ..Default::default()
            }))
        } else { None }
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
            0x14..=0x20 => self.jofr[((offset - 0x14) / 4) as usize],
            0x24 => self.htr,
            0x28 => self.ltr,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x38 => self.jsqr,
            0x3C..=0x48 => self.jdr[((offset - 0x3C) / 4) as usize],
            0x4C => {
                self.dr
            }
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
            0x14..=0x20 => self.jofr[((offset - 0x14) / 4) as usize] = value,
            0x24 => self.htr = value,
            0x28 => self.ltr = value,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x38 => self.jsqr = value,
            _ => {}
        }
    }
}
