use crate::system::{System, instruction_count};
use super::Peripheral;
use std::sync::atomic::Ordering;

fn adc_rand() -> u32 {
    let n = instruction_count();
    ((n.wrapping_mul(1103515245).wrapping_add(12345)) >> 12) as u32
}

fn adc_irq(name: &str) -> i32 {
    match name {
        "ADC3" => 47,
        _ => 18, // ADC1 and ADC2 share IRQ 18
    }
}

pub struct Adc {
    name: String,
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    jsqr: u32,
    jdr: [u32; 4],
    dr: u32,
    last_conv_start: u64,
}

impl Default for Adc {
    fn default() -> Self {
        Self {
            name: String::new(),
            sr: 0,
            cr1: 0,
            cr2: 0x0000_0001,
            smpr1: 0,
            smpr2: 0,
            jofr: [0; 4],
            htr: 0,
            ltr: 0,
            sqr1: 0,
            sqr2: 0,
            sqr3: 0,
            jsqr: 0,
            jdr: [0; 4],
            dr: 0,
            last_conv_start: 0,
        }
    }
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("ADC") { Some(Box::new(Self { name: name.to_string(), ..Self::default() })) } else { None }
    }

    fn eoc_enabled(&self) -> bool { self.cr1 & (1 << 5) != 0 }
    fn ovr_enabled(&self) -> bool { self.cr1 & (1 << 4) != 0 }

    fn fire_interrupts(&mut self, sys: &System) {
        let irq = adc_irq(&self.name);
        if (self.sr & (1 << 1) != 0 && self.eoc_enabled()) ||
           (self.sr & (1 << 5) != 0 && self.ovr_enabled()) {
            sys.p.nvic.borrow_mut().set_intr_pending(irq);
        }
    }

    fn set_eoc(&mut self, sys: &System) {
        self.sr |= 1 << 1; // EOC
        self.fire_interrupts(sys);
    }

    fn start_conversion(&mut self, sys: &System) {
        let n = instruction_count();
        let elapsed = n.saturating_sub(self.last_conv_start);
        if elapsed > 12 {
            let smp = if self.sqr3 & 0x1F < 7 {
                (self.smpr2 & 0x7)
            } else {
                (self.smpr1 & 0x7) >> ((self.sqr3 & 0x1F) % 6 * 3)
            };
            let sampling_cycles = match smp {
                0 => 3, 1 => 15, 2 => 28, 3 => 56,
                4 => 84, 5 => 112, 6 => 144, 7 => 480,
                _ => 3,
            };
            let conv_cycles = sampling_cycles + 12;
            if elapsed >= conv_cycles as u64 {
                let channel = self.sqr3 & 0x1F;
                let val = match channel {
                    16 | 17 => 1200 + (adc_rand() % 50),
                    18 => 1500,
                    _ => adc_rand() % 4096,
                };
                self.dr = val;
                self.set_eoc(sys);
            }
        }
    }
}

impl Peripheral for Adc {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let sr = self.sr;
                self.sr = 0;
                sr
            }
            0x04 => self.cr1,
            0x08 => {
                if self.cr2 & 1 != 0 {
                    self.start_conversion(sys);
                }
                self.cr2
            }
            0x0C => self.smpr1,
            0x10 => self.smpr2,
            0x14..=0x20 => {
                let i = ((offset - 0x14) / 4) as usize;
                self.jofr.get(i).copied().unwrap_or(0)
            }
            0x24 => self.htr,
            0x28 => self.ltr,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x38 => self.jsqr,
            0x3C..=0x48 => {
                let i = ((offset - 0x3C) / 4) as usize;
                self.jdr.get(i).copied().unwrap_or(0)
            }
            0x4C => {
                let dr = self.dr;
                self.sr &= !(1 << 1);
                dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.sr = value & 0x3F;
                self.fire_interrupts(sys);
            }
            0x04 => {
                self.cr1 = value & 0x7FFF_FFFF;
                self.fire_interrupts(sys);
            }
            0x08 => {
                let was_swstart = self.cr2 & (1 << 30);
                self.cr2 = value & 0x7FF0_0EFF;
                if value & (1 << 30) != 0 && was_swstart == 0 {
                    self.last_conv_start = instruction_count();
                }
            }
            0x0C => self.smpr1 = value,
            0x10 => self.smpr2 = value,
            0x14..=0x20 => {
                let i = ((offset - 0x14) / 4) as usize;
                if let Some(r) = self.jofr.get_mut(i) { *r = value & 0xFFF; }
            }
            0x24 => self.htr = value & 0xFFF,
            0x28 => self.ltr = value & 0xFFF,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x38 => self.jsqr = value,
            0x3C..=0x48 => {}
            0x4C => {}
            _ => {}
        }
    }
}
