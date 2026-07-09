use crate::system::System;
use super::Peripheral;

pub struct Rcc {
    cr: u32,
    pllcfgr: u32,
    cfer: u32,
}

impl Rcc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" {
            Some(Box::new(Rcc {
                cr: 0x03,
                ..Default::default()
            }))
        } else {
            None
        }
    }
}

impl Default for Rcc {
    fn default() -> Self {
        Self { cr: 0, pllcfgr: 0, cfer: 0 }
    }
}

impl Peripheral for Rcc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.pllcfgr,
            0x08 => self.cfer,
            _ => 0
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let hseon = value & (1 << 16);
                let pllon = value & (1 << 24);
                let mut cr = value;
                // Set ready flags when enabled
                if hseon != 0 { cr |= 1 << 17; }
                if pllon != 0 { cr |= 1 << 25; }
                self.cr = cr;
            }
            0x04 => {
                self.pllcfgr = value;
            }
            0x08 => {
                // SW bits at 1:0, SWS at 3:2; SWS follows SW
                let sw = value & 0x3;
                self.cfer = (value & !(0x3 << 2)) | (sw << 2);
            }
            _ => {}
        }
    }
}
