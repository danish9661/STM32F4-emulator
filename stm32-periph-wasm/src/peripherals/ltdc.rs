use crate::system::System;
use super::Peripheral;

const LAYER_BASE: u32 = 0x84;
const LAYER_STRIDE: u32 = 0x80;
const NUM_LAYERS: u32 = 2;

#[derive(Clone, Copy, Default)]
struct Lx {
    cr: u32, whpcr: u32, wvpcr: u32, ckcr: u32,
    pfcr: u32, cacr: u32, dccr: u32, bfcr: u32,
    cfbar: u32, cfblr: u32, cfblnr: u32, clutwr: u32,
}

fn lx_bis(offset: u32) -> Option<(u32, u32)> {
    if offset >= LAYER_BASE {
        let layer_rel = offset - LAYER_BASE;
        let layer_idx = layer_rel / LAYER_STRIDE;
        if layer_idx < NUM_LAYERS {
            Some((layer_idx, layer_rel % LAYER_STRIDE))
        } else { None }
    } else { None }
}

pub struct Ltdc {
    sscr: u32, bpcr: u32, awcr: u32, twcr: u32,
    gcr: u32, srcr: u32, bccr: u32,
    ier: u32, isr: u32, lipcr: u32,
    layers: [Lx; NUM_LAYERS as usize],
}

impl Default for Ltdc {
    fn default() -> Self {
        Self {
            sscr: 0, bpcr: 0, awcr: 0, twcr: 0,
            gcr: 0x2220, srcr: 0, bccr: 0,
            ier: 0, isr: 0, lipcr: 0,
            layers: {
                let mut lx = [Lx::default(); NUM_LAYERS as usize];
                lx[0].bfcr = 0x0607;
                lx[1].bfcr = 0x0607;
                lx
            },
        }
    }
}

impl Ltdc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "LTDC" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Ltdc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        if let Some((_li, lr)) = lx_bis(offset) {
            return match lr {
                0x00 => self.layers[_li as usize].cr,
                0x04 => self.layers[_li as usize].whpcr,
                0x08 => self.layers[_li as usize].wvpcr,
                0x0C => self.layers[_li as usize].ckcr,
                0x10 => self.layers[_li as usize].pfcr,
                0x14 => self.layers[_li as usize].cacr,
                0x18 => self.layers[_li as usize].dccr,
                0x1C => self.layers[_li as usize].bfcr,
                0x28 => self.layers[_li as usize].cfbar,
                0x2C => self.layers[_li as usize].cfblr,
                0x30 => self.layers[_li as usize].cfblnr,
                0x40 => 0, // CLUTWR is write-only
                _ => 0,
            };
        }
        match offset {
            0x08 => self.sscr,
            0x0C => self.bpcr,
            0x10 => self.awcr,
            0x14 => self.twcr,
            0x18 => self.gcr,
            0x24 => self.srcr,
            0x2C => self.bccr,
            0x34 => self.ier,
            0x38 => self.isr,
            0x40 => self.lipcr,
            0x44 => 0, // CPSR: current (X,Y) = 0
            0x48 => 0x0F, // CDSR: all display statuses active
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        if let Some((_li, lr)) = lx_bis(offset) {
            match lr {
                0x00 => self.layers[_li as usize].cr = value & 0x13,
                0x04 => self.layers[_li as usize].whpcr = value & 0x0FFF_0FFF,
                0x08 => self.layers[_li as usize].wvpcr = value & 0x07FF_07FF,
                0x0C => self.layers[_li as usize].ckcr = value & 0x00FF_FFFF,
                0x10 => self.layers[_li as usize].pfcr = value & 0x07,
                0x14 => self.layers[_li as usize].cacr = value & 0xFF,
                0x18 => self.layers[_li as usize].dccr = value,
                0x1C => self.layers[_li as usize].bfcr = value & 0x0707,
                0x28 => self.layers[_li as usize].cfbar = value & 0xFFFF_FFFF,
                0x2C => self.layers[_li as usize].cfblr = value & 0x1FFF_1FFF,
                0x30 => self.layers[_li as usize].cfblnr = value & 0x07FF,
                0x40 => self.layers[_li as usize].clutwr = value & 0xFF_FFFF_FF,
                _ => {}
            }
            return;
        }
        match offset {
            0x08 => self.sscr = value & 0x0FFF_0FFF,
            0x0C => self.bpcr = value & 0x0FFF_0FFF,
            0x10 => self.awcr = value & 0x0FFF_0FFF,
            0x14 => self.twcr = value & 0x0FFF_0FFF,
            0x18 => self.gcr = value & 0xF331_1111,
            0x24 => self.srcr = value & 0x03,
            0x2C => self.bccr = value & 0x00FF_FFFF,
            0x34 => self.ier = value & 0x0F,
            0x3C => self.isr &= !(value & 0x0F), // ICR clears ISR bits
            0x40 => self.lipcr = value & 0x07FF,
            _ => {}
        }
    }
}
