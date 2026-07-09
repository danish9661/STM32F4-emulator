use crate::system::System;
use super::Peripheral;

pub struct Rcc {
    cr: u32,
    pllcfgr: u32,
    cfgr: u32,
    cir: u32,
    ahb1rstr: u32,
    ahb2rstr: u32,
    ahb3rstr: u32,
    apb1rstr: u32,
    apb2rstr: u32,
    ahb1enr: u32,
    ahb2enr: u32,
    ahb3enr: u32,
    apb1enr: u32,
    apb2enr: u32,
    ahb1lpenr: u32,
    ahb2lpenr: u32,
    ahb3lpenr: u32,
    apb1lpenr: u32,
    apb2lpenr: u32,
    bdcr: u32,
    csr: u32,
    sscg: u32,
    plli2scfgr: u32,
    pllsai: u32,
    dckcfgr: u32,
    ckgatenr: u32,
    dckcfgr2: u32,
}

impl Default for Rcc {
    fn default() -> Self {
        Self {
            cr: 0x0000_0083, cfgr: 0x0000_0000,
            ahb1enr: 0x0010_0000, apb1enr: 0x0000_0000, apb2enr: 0x0000_0000,
            bdcr: 0x0000_0000, csr: 0x0C00_0000,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Rcc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Rcc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.pllcfgr,
            0x08 => self.cfgr,
            0x0C => self.cir,
            0x10 => self.ahb1rstr,
            0x14 => self.ahb2rstr,
            0x18 => self.ahb3rstr,
            0x20 => self.apb1rstr,
            0x24 => self.apb2rstr,
            0x30 => self.ahb1enr,
            0x34 => self.ahb2enr,
            0x38 => self.ahb3enr,
            0x40 => self.apb1enr,
            0x44 => self.apb2enr,
            0x50 => self.ahb1lpenr,
            0x54 => self.ahb2lpenr,
            0x58 => self.ahb3lpenr,
            0x60 => self.apb1lpenr,
            0x64 => self.apb2lpenr,
            0x70 => self.bdcr,
            0x74 => self.csr,
            0x80 => self.sscg,
            0x84 => self.plli2scfgr,
            0x88 => self.pllsai,
            0x8C => self.dckcfgr,
            0x90 => self.ckgatenr,
            0x94 => self.dckcfgr2,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value,
            0x04 => self.pllcfgr = value,
            0x08 => self.cfgr = value,
            0x0C => self.cir = value,
            0x10 => self.ahb1rstr = value,
            0x14 => self.ahb2rstr = value,
            0x18 => self.ahb3rstr = value,
            0x20 => self.apb1rstr = value,
            0x24 => self.apb2rstr = value,
            0x30 => self.ahb1enr = value,
            0x34 => self.ahb2enr = value,
            0x38 => self.ahb3enr = value,
            0x40 => self.apb1enr = value,
            0x44 => self.apb2enr = value,
            0x50 => self.ahb1lpenr = value,
            0x54 => self.ahb2lpenr = value,
            0x58 => self.ahb3lpenr = value,
            0x60 => self.apb1lpenr = value,
            0x64 => self.apb2lpenr = value,
            0x70 => self.bdcr = value,
            0x74 => self.csr = value,
            0x80 => self.sscg = value,
            0x84 => self.plli2scfgr = value,
            0x88 => self.pllsai = value,
            0x8C => self.dckcfgr = value,
            0x90 => self.ckgatenr = value,
            0x94 => self.dckcfgr2 = value,
            _ => {}
        }
    }
}
