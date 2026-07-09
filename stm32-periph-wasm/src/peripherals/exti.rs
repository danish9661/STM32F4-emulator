use crate::system::System;
use super::Peripheral;

pub struct Exti {
    imr: u32, emr: u32, rtsr: u32, ftsr: u32, swier: u32, pr: u32,
}

impl Default for Exti {
    fn default() -> Self { Self { imr: 0, emr: 0, rtsr: 0, ftsr: 0, swier: 0, pr: 0 } }
}

impl Exti {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "EXTI" { Some(Box::new(Self::default())) } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let active = self.pr & self.imr;
        if active == 0 { return; }
        // Lines 0-4: IRQs 6,7,8,9,10
        for i in 0..=4 {
            if active & (1 << i) != 0 {
                sys.p.nvic.borrow_mut().set_intr_pending(6 + i as i32);
            }
        }
        // Lines 5-9: IRQ 23 (EXTI9_5)
        if active & 0x3E0 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(23);
        }
        // Lines 10-15: IRQ 40 (EXTI15_10)
        if active & 0xFC00 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(40);
        }
    }
}

impl Peripheral for Exti {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.imr,
            0x04 => self.emr,
            0x08 => self.rtsr,
            0x0C => self.ftsr,
            0x10 => 0,
            0x14 => self.pr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.imr = value & 0x7F_FFFF,
            0x04 => self.emr = value & 0x7F_FFFF,
            0x08 => self.rtsr = value & 0x7F_FFFF,
            0x0C => self.ftsr = value & 0x7F_FFFF,
            0x10 => {
                let bits = value & 0x7F_FFFF & self.imr;
                self.swier = bits;
                self.pr |= bits;
                if bits != 0 {
                    self.fire_interrupts(sys);
                }
            }
            0x14 => {
                self.pr &= !(value & 0x7F_FFFF);
            }
            _ => {}
        }
    }
}
