use crate::system::System;
use super::Peripheral;

pub struct Syscfg {
    memrm: u32, pmc: u32,
    exticr: [u32; 4],
    cmpc_read: bool,
}

impl Default for Syscfg {
    fn default() -> Self {
        Self { memrm: 0, pmc: 0, exticr: [0; 4], cmpc_read: false }
    }
}

impl Syscfg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SYSCFG" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Syscfg {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.memrm & 0x03,
            0x04 => self.pmc & 0x80_0000,
            0x08 => self.exticr[0],
            0x0C => self.exticr[1],
            0x10 => self.exticr[2],
            0x14 => self.exticr[3],
            0x20 => {
                let v = if self.cmpc_read { 0x100 } else { 0 };
                self.cmpc_read = true;
                v
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.memrm = value & 0x03,
            0x04 => self.pmc = value & 0x80_0000,
            0x08 => self.exticr[0] = value,
            0x0C => self.exticr[1] = value,
            0x10 => self.exticr[2] = value,
            0x14 => self.exticr[3] = value,
            0x20 => self.cmpc_read = false,
            _ => {}
        }
    }
}
