use crate::system::System;
use super::Peripheral;

pub struct Dbgmcu {
    cr: u32, apb1_fz: u32, apb2_fz: u32,
}

impl Default for Dbgmcu {
    fn default() -> Self { Self { cr: 0, apb1_fz: 0, apb2_fz: 0 } }
}

impl Dbgmcu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DBGMCU" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Dbgmcu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => 0x10006411,
            0x04 => self.cr & 0x1F_0077,
            0x08 => self.apb1_fz,
            0x0C => self.apb2_fz,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x04 => self.cr = value & 0x1F_0077,
            0x08 => self.apb1_fz = value,
            0x0C => self.apb2_fz = value,
            _ => {}
        }
    }
}
