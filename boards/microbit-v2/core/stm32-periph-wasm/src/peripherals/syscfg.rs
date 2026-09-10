use crate::system::System;
use super::Peripheral;

pub struct Syscfg {
    pub memrm: u32,
    pub pmc: u32,
    pub exticr: [u32; 4],
    cmpc_read: bool,
}

impl Default for Syscfg {
    fn default() -> Self {
        Self { memrm: 0, pmc: 0, exticr: [0; 4], cmpc_read: false }
    }
}

impl Syscfg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        // Handled as a named field in Peripherals (EXTI needs EXTICR access)
        let _ = name;
        None
    }

    /// Port selected for EXTI line (0-15): 0=A, 1=B, ... 10=K
    pub fn line_port(&self, line: u8) -> Option<u8> {
        if line >= 16 { return None; }
        let port = (self.exticr[(line / 4) as usize] >> (4 * (line % 4))) & 0xF;
        if port <= 10 { Some(port as u8) } else { None }
    }
}

impl Peripheral for Syscfg {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
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
