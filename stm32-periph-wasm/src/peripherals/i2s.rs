use crate::system::System;
use super::Peripheral;

/// I2S extended peripheral (I2S2ext, I2S3ext).
/// Same register layout as SPI/I2S, used for full-duplex I2S.
#[derive(Default)]
pub struct I2s {
    cr1: u32, cr2: u32, srm: u32, dr: u32,
    i2scfgr: u32, i2spr: u32,
    rx_buffer: u32, ready_toggle: bool,
}

impl I2s {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "I2S2ext" || name == "I2S3ext" {
            Some(Box::new(Self::default()))
        } else { None }
    }
}

impl Peripheral for I2s {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => {
                self.ready_toggle = !self.ready_toggle;
                if self.ready_toggle { 0b11 } else { 0 }
            }
            0x0C => { let v = self.rx_buffer; self.rx_buffer = 0; v }
            0x10 => self.dr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value,
            0x04 => self.cr2 = value,
            0x0C => self.rx_buffer = 0xFF,
            0x10 => self.dr = value,
            0x1C => self.i2scfgr = value & 0xFFF,
            0x20 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}
