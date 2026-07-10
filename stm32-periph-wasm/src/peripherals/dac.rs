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
    // Noise/triangle state
    lfsr1: u16,
    lfsr2: u16,
    tri_cnt1: u16,
    tri_cnt2: u16,
    tri_dir1: bool,
    tri_dir2: bool,
}

impl Default for Dac {
    fn default() -> Self {
        Self {
            sr: 0x0000_0000,
            lfsr1: 0xAAAA,
            lfsr2: 0xAAAA,
            tri_dir1: true,
            tri_dir2: true,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Dac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DAC" { Some(Box::new(Self::default())) } else { None }
    }

    fn mamp1(&self) -> u32 { (self.cr >> 12) & 0xF }
    fn mamp2(&self) -> u32 { (self.cr >> 28) & 0xF }
    fn wave1(&self) -> u32 { (self.cr >> 8) & 0x3 }
    fn wave2(&self) -> u32 { (self.cr >> 24) & 0x3 }

    fn update_dor1(&mut self) {
        if self.cr & 1 != 0 { // EN1
            let raw = self.dhr12r1 & 0xFFF;
            self.dor1 = raw;
        }
    }

    fn update_dor2(&mut self) {
        if self.cr & (1 << 16) != 0 { // EN2
            let raw = self.dhr12r2 & 0xFFF;
            self.dor2 = raw;
        }
    }

    fn advance_waveform(&mut self, ch: u8) {
        let wave = if ch == 1 { self.wave1() } else { self.wave2() };
        let mamp = if ch == 1 { self.mamp1() } else { self.mamp2() };
        match wave {
            0b01 => {
                let lfsr = if ch == 1 { &mut self.lfsr1 } else { &mut self.lfsr2 };
                let bit = ((*lfsr >> 0) ^ (*lfsr >> 2) ^ (*lfsr >> 6) ^ (*lfsr >> 7)) & 1;
                *lfsr = ((*lfsr >> 1) | (bit << 10)) & 0x7FF;
            }
            0b10 | 0b11 => {
                let cnt = if ch == 1 { &mut self.tri_cnt1 } else { &mut self.tri_cnt2 };
                let dir = if ch == 1 { &mut self.tri_dir1 } else { &mut self.tri_dir2 };
                let lsb_mask = if mamp == 0 { 0 } else { (1 << (mamp - 1)) - 1 };
                let max_cnt = lsb_mask;
                if *dir {
                    *cnt = cnt.wrapping_add(1);
                    if *cnt >= max_cnt as u16 { *dir = false; }
                } else {
                    *cnt = cnt.wrapping_sub(1);
                    if *cnt == 0 { *dir = true; }
                }
            }
            _ => {}
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
                if value & 1 != 0 { self.advance_waveform(1); self.update_dor1(); }
                if value & (1 << 1) != 0 { self.advance_waveform(2); self.update_dor2(); }
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
