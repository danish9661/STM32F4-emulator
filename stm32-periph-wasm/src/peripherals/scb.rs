use crate::system::System;
use super::{Peripheral, nvic::irq};

#[derive(Default)]
pub struct Scb;

impl Scb {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SCB" { Some(Box::new(Self)) } else { None }
    }
}

impl Peripheral for Scb {
    fn read(&mut self, _sys: &System, _offset: u32) -> u32 { 0 }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0004 => {
                if value & (1 << 26) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq::SYSTICK);
                }
                if value & (1 << 28) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq::PENDSV);
                }
            }
            _ => {}
        }
    }
}
