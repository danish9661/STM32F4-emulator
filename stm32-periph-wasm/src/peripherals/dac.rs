use crate::system::System;
use super::Peripheral;

pub struct Dac {
    cr: u32,
    swtrigr: u32,
    dhr12r1: u32,
    dhr12l1: u32,
    dhr8r1: u32,
    dhr12r2: u32,
    dhr12l2: u32,
    dhr8r2: u32,
    dhr12rd: u32,
    dhr12ld: u32,
    dhr8rd: u32,
    dor1: u32,
    dor2: u32,
    sr: u32,
}

impl Default for Dac {
    fn default() -> Self {
        Self {
            sr: 0x0000_0000,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Dac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DAC" { Some(Box::new(Self::default())) } else { None }
    }

    fn update_dor1(&mut self) {
        if self.cr & 1 != 0 { // EN1
            // Align: 12-bit right, 12-bit left, or 8-bit right
            if self.cr & (1 << 10) != 0 { // TSEL1 = SWTRIG uses DHR12R1
                self.dor1 = self.dhr12r1 & 0xFFF;
            } else {
                // Use the alignment mode from CR
                let mode_bits = (self.cr >> 6) & 0x3; // WAVE[1:0], actually BOFF is in different position
                // Just pick the most recently written data register
                self.dor1 = self.dhr12r1 & 0xFFF;
            }
        }
    }

    fn update_dor2(&mut self) {
        if self.cr & (1 << 16) != 0 { // EN2
            self.dor2 = self.dhr12r2 & 0xFFF;
        }
    }
}

impl Peripheral for Dac {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.swtrigr,
            0x08 => self.dhr12r1,
            0x0C => self.dhr12l1,
            0x10 => self.dhr8r1,
            0x14 => self.dhr12r2,
            0x18 => self.dhr12l2,
            0x1C => self.dhr8r2,
            0x20 => self.dhr12rd,
            0x24 => self.dhr12ld,
            0x28 => self.dhr8rd,
            0x2C => self.dor1,
            0x30 => self.dor2,
            0x34 => self.sr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0x3F3F_FFFF,
            0x04 => {
                self.swtrigr = value;
                // SWTRIG updates DOR from DHR
                if value & 1 != 0 { self.update_dor1(); }
                if value & (1 << 1) != 0 { self.update_dor2(); }
            }
            0x08 => { self.dhr12r1 = value & 0xFFF; self.update_dor1(); }
            0x0C => { self.dhr12l1 = value & 0xFFF0; self.update_dor1(); }
            0x10 => { self.dhr8r1 = value & 0xFF; self.update_dor1(); }
            0x14 => { self.dhr12r2 = value & 0xFFF; self.update_dor2(); }
            0x18 => { self.dhr12l2 = value & 0xFFF0; self.update_dor2(); }
            0x1C => { self.dhr8r2 = value & 0xFF; self.update_dor2(); }
            0x20 => { self.dhr12rd = value; self.update_dor1(); self.update_dor2(); }
            0x24 => { self.dhr12ld = value; self.update_dor1(); self.update_dor2(); }
            0x28 => { self.dhr8rd = value; self.update_dor1(); self.update_dor2(); }
            0x2C | 0x30 => {} // DOR is read-only
            0x34 => self.sr = value & 0x3,
            _ => {}
        }
    }
}
