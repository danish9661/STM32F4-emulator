use crate::system::System;
use super::Peripheral;

#[derive(Clone, Copy)]
struct Mailbox {
    tir: u32, tdtr: u32, tdlr: u32, tdhr: u32,
}

pub struct Can {
    mcr: u32, msr: u32, tsr: u32, rf0r: u32, rf1r: u32,
    ier: u32, esr: u32, btr: u32,
    tx: [Mailbox; 3],
    rx: [Mailbox; 2],
    fmr: u32, fm1r: u32, fs1r: u32, ffa1r: u32, fa1r: u32,
    filter: [u32; 56],
}

impl Can {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CAN1" || name == "CAN2" {
            Some(Box::new(Can {
                mcr: 0x0001_0002, msr: 0x0000_0C02, tsr: 0x1C00_0000,
                ..Self::default()
            }))
        } else { None }
    }
}

impl Default for Can {
    fn default() -> Self {
        Self {
            mcr: 0, msr: 0, tsr: 0, rf0r: 0, rf1r: 0, ier: 0, esr: 0, btr: 0,
            tx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 3],
            rx: [Mailbox { tir: 0, tdtr: 0, tdlr: 0, tdhr: 0 }; 2],
            fmr: 0x2A1C_0E01, fm1r: 0, fs1r: 0xFFFF_FFFF, ffa1r: 0, fa1r: 0,
            filter: [0; 56],
        }
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
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return 0; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir,
                    0x04 => self.tx[i].tdtr,
                    0x08 => self.tx[i].tdlr,
                    0x0C => self.tx[i].tdhr,
                    _ => 0,
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return 0; }
                match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir,
                    0x04 => self.rx[i].tdtr,
                    0x08 => self.rx[i].tdlr,
                    0x0C => self.rx[i].tdhr,
                    _ => 0,
                }
            }
            0x200 => self.fmr,
            0x204 => self.fm1r,
            0x20C => self.fs1r,
            0x214 => self.ffa1r,
            0x21C => self.fa1r,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize;
                self.filter.get(i).copied().unwrap_or(0)
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => {
                let mask = 0x7F3F;
                self.mcr = (self.mcr & !mask) | (value & mask);
                let inrq = value & 1;
                let sleep = (value >> 1) & 1;
                if inrq != 0 {
                    self.msr |= 1; self.msr &= !2;
                } else {
                    self.msr &= !1; self.msr |= 2;
                }
                if sleep != 0 { self.msr |= 2; }
                else if inrq == 0 { self.msr &= !2; }
            }
            0x004 => self.msr = (self.msr & !0x0C0B) | (value & 0x0C0B),
            0x008 => self.tsr &= !(value & 0x0007_0707),
            0x00C => self.rf0r = (self.rf0r & 0xFFFF_0000) | (value & 0x3F),
            0x010 => self.rf1r = (self.rf1r & 0xFFFF_0000) | (value & 0x3F),
            0x014 => self.ier = value & 0x7FF,
            0x01C => self.btr = value & 0x3FFF_FFFF,
            0x180..=0x1AC => {
                let i = ((offset - 0x180) / 0x10) as usize;
                if i >= 3 { return; }
                match (offset - 0x180) % 0x10 {
                    0x00 => self.tx[i].tir = value,
                    0x04 => self.tx[i].tdtr = value,
                    0x08 => self.tx[i].tdlr = value,
                    0x0C => self.tx[i].tdhr = value,
                    _ => {}
                }
                // Tx request: complete immediately with success
                if (offset - 0x180) % 0x10 == 0 && value & 1 != 0 {
                    self.tsr |= 1 << i;
                    self.tsr |= 0x2 << (8 + i * 4);
                    let rqcp = (self.tsr >> 26) & 7;
                    self.tsr = (self.tsr & !(7 << 26)) | ((rqcp & !(1 << i)) << 26);
                    self.tsr |= 1 << (16 + i);
                    if (self.ier >> (i + 15)) & 1 != 0 {
                        self.tsr |= 1 << (16 + i);
                    }
                }
            }
            0x1B0..=0x1CC => {
                let i = ((offset - 0x1B0) / 0x10) as usize;
                if i >= 2 { return; }
                match (offset - 0x1B0) % 0x10 {
                    0x00 => self.rx[i].tir = value,
                    0x04 => self.rx[i].tdtr = value,
                    0x08 => self.rx[i].tdlr = value,
                    0x0C => self.rx[i].tdhr = value,
                    _ => {}
                }
                // Reading rx mailbox releases it: decrement FIFO count
                if (offset - 0x1B0) % 0x10 == 0 && i == 0 && self.rf0r & 0x3 != 0 {
                    self.rf0r = (self.rf0r & !0x3) | ((self.rf0r & 0x3) - 1);
                    self.rf0r |= 1 << 3;
                }
            }
            0x200 => {
                if value & 1 != 0 {
                    self.fm1r = 0; self.fs1r = 0xFFFF_FFFF; self.ffa1r = 0; self.fa1r = 0;
                }
                self.fmr = value & 0x3F;
            }
            0x204 => self.fm1r = value,
            0x20C => self.fs1r = value,
            0x214 => self.ffa1r = value,
            0x21C => self.fa1r = value,
            0x240..=0x31C => {
                let i = ((offset - 0x240) / 4) as usize;
                if let Some(f) = self.filter.get_mut(i) { *f = value; }
            }
            _ => {}
        }
    }
}
