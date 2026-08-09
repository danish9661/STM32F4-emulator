use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
}

impl Default for Pwr {
    fn default() -> Self {
        Self {
            cr: 0x0000_0000,
            csr: 0x0000_0000,
        }
    }
}

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Pwr {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = (self.cr & 0xE000) | (value & 0x1FFF);
                if value & 0x10 != 0 && value & 0x0F != 0 && value & !0x1F == 0 {
                    self.csr |= 1 << 1;
                }
            }
            0x04 => {
                self.csr |= value & 0x100;
            }
            _ => {}
        }
    }
}
