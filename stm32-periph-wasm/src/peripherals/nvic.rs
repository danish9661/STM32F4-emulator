use crate::system::System;
use crate::system::INSTRUCTION_COUNT;
use std::sync::atomic::Ordering;
use super::Peripheral;

#[derive(Default)]
pub struct Nvic {
    pub systick_period: Option<u32>,
    pub last_systick_trigger: u64,
    pending: u128,
    in_interrupt: bool,
}

const IRQ_OFFSET: i32 = 16;

pub mod irq {
    pub const PENDSV: i32 = -2;
    pub const SYSTICK: i32 = -1;
}

impl Nvic {
    pub fn set_intr_pending(&mut self, irq: i32) {
        let bit = IRQ_OFFSET + irq;
        if bit >= 0 {
            self.pending |= 1 << bit;
        }
    }

    pub fn get_and_clear_next_intr_pending(&mut self) -> Option<i32> {
        if self.pending != 0 {
            let bit = self.pending.trailing_zeros();
            self.pending &= !(1 << bit);
            let irq = (bit as i32) - IRQ_OFFSET;
            Some(irq)
        } else {
            None
        }
    }

    pub fn maybe_set_systick_intr_pending(&mut self) {
        if let Some(systick_period) = self.systick_period {
            let n = INSTRUCTION_COUNT.load(Ordering::Relaxed);
            let delta = n - self.last_systick_trigger;
            if delta > systick_period as u64 {
                self.last_systick_trigger = n;
                self.set_intr_pending(irq::SYSTICK);
            }
        }
    }

    pub fn is_in_interrupt(&self) -> bool {
        self.in_interrupt
    }

    pub fn set_in_interrupt(&mut self, v: bool) {
        self.in_interrupt = v;
    }
}

impl Peripheral for Nvic {
    fn read(&mut self, _sys: &System, _offset: u32) -> u32 {
        0
    }

    fn write(&mut self, _sys: &System, _offset: u32, _value: u32) {
    }
}

pub struct NvicWrapper;

impl NvicWrapper {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "NVIC" {
            Some(Box::new(Self))
        } else {
            None
        }
    }
}

impl Peripheral for NvicWrapper {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        sys.p.nvic.borrow_mut().read(sys, offset)
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        sys.p.nvic.borrow_mut().write(sys, offset, value)
    }
}
