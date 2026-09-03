pub mod regs;
pub mod mem;
pub(crate) mod thumb;
#[cfg(test)]
mod tests;
pub use regs::Regs;
pub use mem::Memory;
use crate::system::WasmSystem;

/// The CPU stopped because of this (unknown instruction, SVC/BKPT, branch to
/// ARM state, ...). `pc` is the faulting instruction address (without thumb
/// bit); `op1`/`op2` are the raw halfwords; `len` is 2 or 4.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFault {
    pub pc: u32,
    pub op1: u16,
    pub op2: u16,
    pub len: u8,
}

pub struct Cpu {
    pub regs: Regs,
    pub cycles: u64,
    pub fault: Option<CpuFault>,
    // IT-block state. `n == 0` means no active block. `idx` counts consumed
    // instructions (1-based). See the IT rule in thumb.rs (`it_ok`): j>=2
    // uses `cond` iff mask bit (5-j) equals cond bit 0, else the inverse.
    pub it_cond: u8,
    pub it_mask: u8,
    pub it_n: u8,
    pub it_idx: u8,
    /// Break `run()` when the model has a pending interrupt, so a driver
    /// with guest exception delivery can take it. Off by default: polling
    /// firmware (and the current JS wasm driver, which has no ISR pump)
    /// must run full budgets exactly like the Unicorn path, where pending
    /// model IRQs never stop `emu_start`.
    pub deliver_irqs: bool,
}

impl Cpu {
    pub fn new(sp: u32, pc: u32) -> Self {
        Self {
            regs: Regs::new(sp, pc),
            cycles: 0,
            fault: None,
            it_cond: 0,
            it_mask: 0,
            it_n: 0,
            it_idx: 0,
            deliver_irqs: false,
        }
    }
    pub fn reset(&mut self, sp: u32, pc: u32) {
        self.regs = Regs::new(sp, pc);
        self.fault = None;
        self.it_cond = 0;
        self.it_mask = 0;
        self.it_n = 0;
        self.it_idx = 0;
    }
    pub fn run(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, budget: u32) -> u32 {
        let mut done = 0;
        while done < budget {
            if self.fault.is_some() {
                break;
            }
            let pc = self.regs.r[15] & !1;
            let op = mem.read16(pc);
            let l = thumb::len(op);
            let ok = if l == 2 {
                thumb::exec16(self, sys, mem, op, pc)
            } else {
                let o2 = mem.read16(pc + 2);
                thumb::exec32(self, sys, mem, op, o2, pc)
            };
            if !ok {
                if self.fault.is_none() {
                    self.fault = Some(CpuFault { pc, op1: op, op2: 0, len: 2 });
                }
                break;
            }
            done += 1;
            self.cycles += 1;
            if self.deliver_irqs && sys.p.nvic.borrow().has_pending() {
                break;
            }
        }
        done
    }
}
