use crate::system::System;
use super::Peripheral;

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    fmr: u32, fm1r: u32, fs1r: u32, ffa1r: u32, fa1r: u32,
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0x0001_0002, msr: 0x0000_0C02, tsr: 0x1C00_0000,
            rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            fmr: 0x2A1C_0E01, fm1r: 0, fs1r: 0xFFFF_FFFF, ffa1r: 0, fa1r: 0,
        }
    }
}

impl Can {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CAN1" || name == "CAN2" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Can {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000 => self.mcr,
            0x004 => self.msr,
            0x008 => self.tsr,
            0x00C => self.rf0r,
            0x010 => self.rf1r,
            0x014 => self.ier,
            0x018 => self.esr,
            0x01C => self.btr,
            0x200 => self.fmr,
            0x204 => self.fm1r,
            0x20C => self.fs1r,
            0x214 => self.ffa1r,
            0x21C => self.fa1r,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => {
                let mask = 0x7F3F;
                self.mcr = (self.mcr & !mask) | (value & mask);
                if value & 1 != 0 { self.msr |= 1; self.msr &= !2; }
                else if value & 2 == 0 { self.msr &= !1; self.msr &= !2; }
                if value & 2 != 0 && value & 1 == 0 { self.msr |= 2; }
            }
            0x004 => self.msr = (self.msr & !0x0C0B) | (value & 0x0C0B),
            0x008 => self.tsr &= !(value & 0x0007_0707),
            0x00C => self.rf0r = value & 0x3F,
            0x010 => self.rf1r = value & 0x3F,
            0x014 => self.ier = value & 0x7FF,
            0x01C => self.btr = value & 0x3FFF_FFFF,
            0x200 => {
                let was = self.fmr;
                self.fmr = value & 0x3F;
                if self.fmr & 1 != 0 && was & 1 == 0 {
                    self.fm1r = 0; self.fs1r = 0; self.ffa1r = 0; self.fa1r = 0;
                }
            }
            0x204 => self.fm1r = value,
            0x20C => self.fs1r = value,
            0x214 => self.ffa1r = value,
            0x21C => self.fa1r = value,
            _ => {}
        }
    }
}
