use crate::system::{System, instruction_count};
use super::Peripheral;

pub struct Dcmi {
    cr: u32, sr: u32, ris: u32, ier: u32, icr: u32,
    escr: u32, esur: u32, cwstrt: u32, cwsize: u32,
    dr: u32, pixel_count: u32, pattern: u32, last_tick: u64,
}

const FRAME_SIZE: u32 = 76800;

impl Default for Dcmi {
    fn default() -> Self {
        Self {
            cr: 0, sr: 0, ris: 0, ier: 0, icr: 0,
            escr: 0, esur: 0, cwstrt: 0, cwsize: 0,
            dr: 0, pixel_count: 0, pattern: 0, last_tick: 0,
        }
    }
}

impl Dcmi {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DCMI" { Some(Box::new(Self::default())) } else { None }
    }

    fn tick(&mut self) {
        if self.cr & 1 == 0 { return; }
        let n = instruction_count();
        if n.wrapping_sub(self.last_tick) < 64 { return; }
        self.last_tick = n;
        self.pixel_count += 1;
        self.pattern = self.pattern.wrapping_add(0x01020304);
        self.dr = self.pattern;
        self.sr |= 4;
        if self.pixel_count >= FRAME_SIZE {
            self.pixel_count = 0;
            self.ris |= 1;
            if (self.cr >> 6) & 3 != 0 { self.cr &= !1; }
        }
    }
}

impl Peripheral for Dcmi {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        self.tick();
        match offset {
            0x00 => self.cr,
            0x04 => self.sr,
            0x08 => self.ris,
            0x0C => self.ier,
            0x10 => self.ris & self.ier,
            0x18 => self.escr,
            0x1C => self.esur,
            0x20 => self.cwstrt,
            0x24 => self.cwsize,
            0x28 => { let v = self.dr; self.sr &= !4; v }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let was = self.cr & 1;
                self.cr = value & 0x7FFF_3FFF;
                if self.cr & 1 != 0 && was == 0 {
                    self.pixel_count = 0; self.pattern = 0; self.sr |= 2;
                } else if self.cr & 1 == 0 && was != 0 {
                    self.sr &= !2;
                }
            }
            0x0C => self.ier = value & 0x1F,
            0x14 => { self.ris &= !value; }
            0x18 => self.escr = value & 0xF00F_0F0F,
            0x1C => self.esur = value & 0xFF00_FF00,
            0x20 => self.cwstrt = value & 0x3FFF_1FFF,
            0x24 => self.cwsize = value & 0x3FFF_1FFF,
            _ => {}
        }
    }
}
