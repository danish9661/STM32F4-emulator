use crate::system::System;
use super::Peripheral;

const SAI_IRQ: i32 = 87;

#[derive(Clone, Copy, Default)]
struct SaiBlock {
    cr1: u32, cr2: u32, frcr: u32, slotr: u32,
    im: u32, sr: u32, clrfr: u32, dr: u32,
}

impl SaiBlock {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        if offset == 0x1C {
            let v = self.dr;
            self.sr = 0x08;
            self.fire_interrupts(sys);
            v
        } else {
            match offset {
                0x00 => self.cr1,
                0x04 => self.cr2,
                0x08 => self.frcr,
                0x0C => self.slotr,
                0x10 => self.im,
                0x14 => self.sr,
                0x18 => self.clrfr,
                _ => 0,
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x3F3F_FFFF,
            0x04 => self.cr2 = value & 0x7FFF,
            0x08 => self.frcr = value & 0x7_FFFF,
            0x0C => self.slotr = value & 0x1F_FFFF,
            0x10 => self.im = value & 0x7F,
            0x18 => {
                self.clrfr = value & 0x77;
                self.sr &= !self.clrfr;
                self.fire_interrupts(sys);
            }
            0x1C => {
                self.dr = value;
                self.sr = 0x13;
                self.fire_interrupts(sys);
            }
            _ => {}
        }
    }

    fn fire_interrupts(&self, sys: &System) {
        if self.sr & self.im != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(SAI_IRQ);
        }
    }
}

pub struct Sai {
    blocks: [SaiBlock; 2],
    gcr: u32,
}

impl Default for Sai {
    fn default() -> Self {
        Self {
            blocks: [
                SaiBlock { cr1: 0x40, frcr: 0x07, sr: 0x08, ..SaiBlock::default() },
                SaiBlock { cr1: 0x40, frcr: 0x07, sr: 0x08, ..SaiBlock::default() },
            ],
            gcr: 0,
        }
    }
}

impl Sai {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SAI1" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Sai {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x000 => self.gcr,
            0x004..=0x020 => self.blocks[0].read(_sys, offset - 0x004),
            0x024..=0x040 => self.blocks[1].read(_sys, offset - 0x024),
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x000 => self.gcr = value & 0xFF,
            0x004..=0x020 => self.blocks[0].write(_sys, offset - 0x004, value),
            0x024..=0x040 => self.blocks[1].write(_sys, offset - 0x024, value),
            _ => {}
        }
    }
}
