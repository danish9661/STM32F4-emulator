use crate::system::System;
use super::Peripheral;

#[derive(Clone, Copy, Default)]
struct SaiBlock {
    cr1: u32, cr2: u32, frcr: u32, slotr: u32,
    im: u32, sr: u32, clrfr: u32, dr: u32,
}

impl SaiBlock {
    fn read(&self, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.frcr,
            0x0C => self.slotr,
            0x10 => self.im,
            0x14 => self.sr,
            0x18 => self.clrfr,
            0x1C => self.dr,
            _ => 0,
        }
    }

    fn write(&mut self, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value & 0x3F3F_FFFF,
            0x04 => self.cr2 = value & 0x7FFF,
            0x08 => self.frcr = value & 0x7_FFFF,
            0x0C => self.slotr = value & 0x1F_FFFF,
            0x10 => self.im = value & 0x7F,
            0x18 => self.clrfr = value & 0x77,
            // DR (0x1C): write = tx data, read-only for status kept
            0x1C => self.dr = value,
            _ => {}
        }
    }
}

pub struct Sai {
    blocks: [SaiBlock; 2], // A=0, B=1
}

impl Default for Sai {
    fn default() -> Self {
        Self {
            blocks: [
                SaiBlock { cr1: 0x40, frcr: 0x07, sr: 0x08, ..SaiBlock::default() },
                SaiBlock { cr1: 0x40, frcr: 0x07, sr: 0x08, ..SaiBlock::default() },
            ],
        }
    }
}

impl Sai {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SAI1" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Sai {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x004..=0x020 => self.blocks[0].read(offset - 0x004),
            0x024..=0x040 => self.blocks[1].read(offset - 0x024),
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x004..=0x020 => self.blocks[0].write(offset - 0x004, value),
            0x024..=0x040 => self.blocks[1].write(offset - 0x024, value),
            _ => {}
        }
    }
}
