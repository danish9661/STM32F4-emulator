use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
}

impl Default for Pwr {
    fn default() -> Self {
        Self {
            cr: 0x0000_0000,
            csr: 0x0000_0200, // PVDO=1, SBF=0, WUF=0
        }
    }
}

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Pwr {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.csr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // CR: VOS, ADCDC1, FPDS, DBP, PLS, PVDE, CSBF, CWUF, PDDS, LPDS, FLPS
                self.cr = value & 0x0000_FEFE;
                // Clear wakeup flags when requested
                if value & (1 << 2) != 0 { self.csr &= !(1 << 0); } // CWUF
                if value & (1 << 3) != 0 { self.csr &= !(1 << 1); } // CSBF
                // DBP: enable RTC and backup registers access
                // PVDE: enable programmable voltage detector
            }
            0x04 => {
                // CSR: read-only, but WUF, SBF, PVDO, BRR, VOSF are status
                // Write is ignored or clears flags
                // Actually PWR_CSR is read-only; clearing is via CR bits (CWUF, CSBF)
                self.csr &= !(value & 0x3); // allow clearing WUF/SBF via direct write too
            }
            _ => {}
        }
    }
}
