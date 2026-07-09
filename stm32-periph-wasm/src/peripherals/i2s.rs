use crate::system::System;
use super::Peripheral;

fn i2s_irq(name: &str) -> Option<i32> {
    match name {
        "I2S2ext" => Some(36),
        "I2S3ext" => Some(51),
        _ => None,
    }
}

pub struct I2s {
    name: String,
    cr1: u32, cr2: u32, srm: u32, dr: u32,
    i2scfgr: u32, i2spr: u32,
    rx_buffer: u32, sr: u32,
    irq_num: i32,
}

impl I2s {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let irq_num = i2s_irq(name)?;
        Some(Box::new(Self {
            name: name.to_string(),
            irq_num,
            sr: 0x03,
            ..Self::default()
        }))
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let tx_en = if (self.cr2 >> 1) & 1 != 0 && self.sr & (1 << 1) != 0 { 1 } else { 0 };
        let rx_en = if self.cr2 & 1 != 0 && self.sr & 1 != 0 { 1 } else { 0 };
        if tx_en != 0 || rx_en != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }
}

impl Default for I2s {
    fn default() -> Self {
        Self {
            name: String::new(),
            cr1: 0, cr2: 0, srm: 0, dr: 0,
            i2scfgr: 0, i2spr: 0,
            rx_buffer: 0, sr: 0x03,
            irq_num: 0,
        }
    }
}

impl Peripheral for I2s {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            0x0C => {
                let v = self.rx_buffer;
                self.rx_buffer = 0;
                self.sr |= 1;
                self.sr &= !(1 << 1);
                self.fire_interrupts(sys);
                v
            }
            0x10 => self.dr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value,
            0x04 => {
                self.cr2 = value;
                self.fire_interrupts(sys);
            }
            0x0C => {
                self.rx_buffer = 0xFF;
                self.sr |= 1;
            }
            0x10 => {
                self.dr = value;
                self.sr |= 1 << 1;
                self.sr &= !1;
                self.fire_interrupts(sys);
            }
            0x1C => self.i2scfgr = value & 0xFFF,
            0x20 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}
