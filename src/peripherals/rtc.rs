use crate::system::System;
use super::Peripheral;

pub struct Rtc {
    tr: u32,
    dr: u32,
    cr: u32,
    isr: u32,
    prer: u32,
    wutr: u32,
    calibr: u32,
    alrmar: u32,
    alrmbr: u32,
    wpr: u32,
    ssr: u32,
    shiftr: u32,
    tstr: u32,
    tsdr: u32,
    tsssr: u32,
    calr: u32,
    tafcr: u32,
    alrmassr: u32,
    alrmbssr: u32,
    bkp: [u32; 20],
}

impl Rtc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RTC" {
            Some(Box::new(Rtc {
                isr: 0x0000_0007,
                prer: 0x007F_00FF,
                ssr: 0x0000_7FFF,
                tr: 0x0000_2100,
                dr: 0x0000_2101,
                ..Default::default()
            }))
        } else {
            None
        }
    }
}

impl Default for Rtc {
    fn default() -> Self {
        Self {
            tr: 0, dr: 0, cr: 0, isr: 0, prer: 0, wutr: 0, calibr: 0,
            alrmar: 0, alrmbr: 0, wpr: 0, ssr: 0, shiftr: 0,
            tstr: 0, tsdr: 0, tsssr: 0, calr: 0, tafcr: 0,
            alrmassr: 0, alrmbssr: 0, bkp: [0; 20],
        }
    }
}

impl Peripheral for Rtc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.tr,
            0x04 => self.dr,
            0x08 => self.cr,
            0x0C => self.isr,
            0x10 => self.prer,
            0x14 => self.wutr,
            0x18 => self.calibr,
            0x1C => self.alrmar,
            0x20 => self.alrmbr,
            0x24 => self.wpr,
            0x28 => self.ssr,
            0x2C => self.shiftr,
            0x30 => self.tstr,
            0x34 => self.tsdr,
            0x38 => self.tsssr,
            0x3C => self.calr,
            0x40 => self.tafcr,
            0x44 => self.alrmassr,
            0x48 => self.alrmbssr,
            0x50..=0x9C => {
                let idx = ((offset - 0x50) / 4) as usize;
                if idx < 20 { self.bkp[idx] } else { 0 }
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.tr = value & 0x007F_7F7F,
            0x04 => self.dr = value & 0x00FF_3F3F,
            0x08 => self.cr = value & 0x003F_7FFF,
            0x0C => self.isr = (self.isr & 0x0000_007F) | (value & 0x0000_0073),
            0x10 => self.prer = value & 0x7F7F_FFFF,
            0x14 => self.wutr = value & 0xFFFF,
            0x18 => self.calibr = value & 0x1F80_001F,
            0x1C => self.alrmar = value & 0x7F7F_7FFF,
            0x20 => self.alrmbr = value & 0x7F7F_7FFF,
            0x24 => self.wpr = value,
            0x2C => self.shiftr = value & 0x1FFF_7FFF,
            0x3C => self.calr = value & 0x01FF_00FF,
            0x40 => self.tafcr = value & 0x3F0F_FFFF,
            0x44 => self.alrmassr = value & 0x0F00_7FFF,
            0x48 => self.alrmbssr = value & 0x0F00_7FFF,
            0x50..=0x9C => {
                let idx = ((offset - 0x50) / 4) as usize;
                if idx < 20 { self.bkp[idx] = value; }
            }
            _ => {}
        }
    }
}
