use crate::system::System;
use super::Peripheral;

/// STIR — software trigger interrupt register (0xE000EF00, write-only).
/// A write pends the external IRQ in the low 9 bits (out-of-range values
/// are ignored, deny-closed); reads return 0. Like the ICSR pend bits,
/// unprivileged writes are ignored unless CCR.USERSETMPEND is set.
pub struct Stir;

impl Stir {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "STIR" {
            Some(Box::new(Self))
        } else {
            None
        }
    }
}

impl Peripheral for Stir {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, _offset: u32) -> u32 {
        0
    }
    fn write(&mut self, sys: &System, _offset: u32, value: u32) {
        let irq = (value & 0x1FF) as i32;
        if irq >= super::nvic::NVIC_IRQ_COUNT as i32 {
            return;
        }
        if !crate::system::current_privileged() {
            let usersetmpend = sys.p.read(sys, 0xE000ED14, 4) & 2 != 0;
            if !usersetmpend {
                return;
            }
        }
        sys.p.nvic.borrow_mut().set_intr_pending(irq);
    }
}
