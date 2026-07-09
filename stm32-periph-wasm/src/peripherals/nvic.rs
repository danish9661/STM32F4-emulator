use crate::system::{System, INSTRUCTION_COUNT};
use std::sync::atomic::Ordering;
use super::Peripheral;

const IRQ_COUNT: usize = 97;
const IRQ_OFFSET: i32 = 16;
const REG_WORDS: usize = (IRQ_COUNT + 31) / 32;

pub mod irq {
    pub const PENDSV: i32 = -2;
    pub const SYSTICK: i32 = -1;
}

#[derive(Clone)]
pub struct Nvic {
    pub systick_period: Option<u32>,
    pub last_systick_trigger: u64,
    pending: u128,
    in_interrupt: bool,
    enable: [u32; REG_WORDS],
    pending_reg: [u32; REG_WORDS],
    active: [u32; REG_WORDS],
    priority: [u8; IRQ_COUNT],
}

impl Default for Nvic {
    fn default() -> Self {
        Self {
            systick_period: None,
            last_systick_trigger: 0,
            pending: 0,
            in_interrupt: false,
            enable: [0; REG_WORDS],
            pending_reg: [0; REG_WORDS],
            active: [0; REG_WORDS],
            priority: [0; IRQ_COUNT],
        }
    }
}

impl Nvic {
    fn irq_bit(irq: i32) -> Option<(usize, u32)> {
        let bit = IRQ_OFFSET + irq;
        if bit < 0 { return None; }
        let idx = (bit as usize) / 32;
        let mask = 1u32 << (bit as usize % 32);
        if idx >= REG_WORDS { None } else { Some((idx, mask)) }
    }

    pub fn set_intr_pending(&mut self, irq: i32) {
        if let Some((idx, mask)) = Self::irq_bit(irq) {
            self.pending |= 1u128 << (IRQ_OFFSET + irq);
            self.pending_reg[idx] |= mask;
        }
    }

    pub fn clear_pending(&mut self, irq: i32) {
        if let Some((idx, mask)) = Self::irq_bit(irq) {
            self.pending &= !(1u128 << (IRQ_OFFSET + irq));
            self.pending_reg[idx] &= !mask;
        }
    }

    pub fn get_pending_vector(&self) -> u32 {
        if self.pending != 0 {
            let bit = self.pending.trailing_zeros();
            bit - IRQ_OFFSET as u32
        } else {
            0
        }
    }

    pub fn get_and_clear_next_intr_pending(&mut self) -> Option<i32> {
        if self.pending != 0 {
            let bit = self.pending.trailing_zeros();
            let irq = (bit as i32) - IRQ_OFFSET;
            // External IRQs (bit >= 16) need ISER enable; system exceptions always fire
            if bit >= 16 {
                let idx = (bit as usize) / 32;
                let mask = 1u32 << (bit as usize % 32);
                if self.enable[idx] & mask == 0 {
                    // Not enabled - skip and clear
                    self.pending &= !(1 << bit);
                    self.pending_reg[idx] &= !mask;
                    return None;
                }
            }
            self.pending &= !(1 << bit);
            if bit >= 16 {
                let idx = (bit as usize) / 32;
                let mask = 1u32 << (bit as usize % 32);
                self.pending_reg[idx] &= !mask;
                self.active[idx] |= mask;
            }
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
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x004 => {
                // Interrupt Controller Type Register: INTLines = IRQ_COUNT / 32
                let intlines = ((IRQ_COUNT + 31) / 32 - 1) as u32;
                intlines << 4
            }
            // ISER[0..2] at 0x100, 0x104, 0x108
            0x100..=0x11C if offset < 0x100 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x100) / 4) as usize;
                self.enable[i]
            }
            // ICER
            0x180..=0x19C if offset < 0x180 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x180) / 4) as usize;
                self.enable[i] // reads current enable state
            }
            // ICPR
            0x200..=0x21C if offset < 0x200 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x200) / 4) as usize;
                self.pending_reg[i]
            }
            // ISPR
            0x300..=0x31C if offset < 0x300 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x300) / 4) as usize;
                self.pending_reg[i]
            }
            // IABR
            0x380..=0x39C if offset < 0x380 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x380) / 4) as usize;
                self.active[i]
            }
            // IPR - byte accessible, 4 per IRQ word
            0x400..=0x5EF => {
                let byte_idx = (offset - 0x400) as usize;
                if byte_idx < IRQ_COUNT {
                    self.priority[byte_idx] as u32
                } else { 0 }
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x100..=0x11C if offset < 0x100 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x100) / 4) as usize;
                let was = self.enable[i];
                self.enable[i] |= value;
                // Set any newly enabled pending bits
                let newly_enabled = self.enable[i] & !was;
                let newly_pending = self.pending_reg[i] & newly_enabled;
                for b in 0..32 {
                    if newly_pending & (1 << b) != 0 {
                        self.pending |= 1u128 << (i * 32 + b);
                    }
                }
            }
            0x180..=0x19C if offset < 0x180 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x180) / 4) as usize;
                self.enable[i] &= !value;
            }
            0x200..=0x21C if offset < 0x200 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x200) / 4) as usize;
                let cleared = self.pending_reg[i] & value;
                self.pending_reg[i] &= !value;
                for b in 0..32 {
                    if cleared & (1 << b) != 0 {
                        self.pending &= !(1u128 << (i * 32 + b));
                    }
                }
            }
            0x300..=0x31C if offset < 0x300 + 4 * REG_WORDS as u32 => {
                let i = ((offset - 0x300) / 4) as usize;
                let new_pending = value & !self.pending_reg[i];
                self.pending_reg[i] |= value;
                // Set global pending for any newly set bits that are enabled
                for b in 0..32 {
                    if new_pending & (1 << b) != 0 && self.enable[i] & (1 << b) != 0 {
                        self.pending |= 1u128 << (i * 32 + b);
                    }
                }
            }
            0x400..=0x5EF => {
                let byte_idx = (offset - 0x400) as usize;
                if byte_idx < IRQ_COUNT {
                    // Write byte enables: IPR is accessed as 8/16/32 bit
                    // Each byte maps to one IRQ's priority
                    self.priority[byte_idx] = (value & 0xFF) as u8;
                }
            }
            _ => {}
        }
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
