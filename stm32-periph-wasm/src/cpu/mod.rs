pub mod regs;
pub mod mem;
pub(crate) mod thumb;
#[cfg(test)]
mod tests;
pub use regs::Regs;
pub use mem::Memory;
use crate::system::WasmSystem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// EXC_RETURN values we support (thread mode). F1 (return to handler) is
/// nested-interrupt territory and faults loudly for now.
pub const EXC_RETURN_MSP: u32 = 0xFFFFFFF9;
pub const EXC_RETURN_PSP: u32 = 0xFFFFFFFD;
pub const EXC_RETURN_HANDLER: u32 = 0xFFFFFFF1;
/// Same, with an FP-extended frame (EXC_RETURN bit 4 == 0): issued when the
/// thread uses the FPU (CONTROL.FPCA) and FPCCR.ASPEN is set.
pub const EXC_RETURN_MSP_FP: u32 = 0xFFFFFFE9;
pub const EXC_RETURN_PSP_FP: u32 = 0xFFFFFFED;

/// PC trace for execution debugging: when
/// enabled, every executed instruction appends its PC. Bounded by the
/// harness (enable late, drain early); a runaway buffer just costs memory.
static TRACE_ON: AtomicBool = AtomicBool::new(false);
static TRACE_BUF: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// Enable PC tracing. Cheap when off (one atomic load per instruction).
pub fn trace_start() {
    TRACE_ON.store(true, Ordering::Relaxed);
}
/// Disable PC tracing (buffer keeps whatever was recorded).
pub fn trace_stop() {
    TRACE_ON.store(false, Ordering::Relaxed);
}
/// Drain and return the recorded PCs.
pub fn take_trace() -> Vec<u32> {
    std::mem::take(&mut TRACE_BUF.lock().unwrap())
}
/// The CPU stopped because of this (unknown instruction, BKPT, branch to
/// ARM state, ...). `pc` is the faulting instruction address (without thumb
/// bit); `op1`/`op2` are the raw halfwords; `len` is 2 or 4.
#[derive(Clone, Copy, Debug, Default)]
pub struct CpuFault {
    pub pc: u32,
    pub op1: u16,
    pub op2: u16,
    pub len: u8,
}

/// Saved IT-block state across an exception (pushed on entry, popped on
/// return — the handler runs with a clean ITSTATE, per ARM).
#[derive(Clone, Copy, Debug, Default)]
struct SavedIt {
    cond: u8,
    mask: u8,
    n: u8,
    idx: u8,
}

/// Saved lazy-FP-stacking state across an exception (pushed on entry,
/// popped on return — parallel to it_stack, which is what makes NESTED
/// FPU use safe: an inner reserve clobbers the model's LSPACT/FPCAR, and
/// the pop restores the outer frame's).
#[derive(Clone, Copy, Debug, Default)]
struct SavedFp {
    lspact: bool,
    fpcar: u32,
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
    /// True while executing a predicated (in-IT-block) instruction. Snapshot
    /// at exec top before `it_ok` consumes/resets the slot: T1 MOVS/ADD-reg/
    /// SUB-reg preserve flags when predicated (GCC/Unicorn/vanilla semantics;
    /// e.g. D_PageTicker's `itt lt; movlt; strlt` and S_Start's `addle` need
    /// N live for the next slot). Unpredicated behavior is unchanged.
    pub it_pred: bool,
    /// Exception number currently executing (0 = thread mode). Mirrors IPSR.
    pub ipsr: u32,
    /// IRQ numbers of entered exceptions (for NVIC active-bit hygiene).
    exc_stack: Vec<i32>,
    /// Saved IT states, parallel to exc_stack.
    it_stack: Vec<SavedIt>,
    /// Saved lazy-FP states, parallel to exc_stack.
    fp_stack: Vec<SavedFp>,
    /// Break `run()` when the model has a pending interrupt, so a driver
    /// with guest exception delivery can take it. Off by default: polling
    /// firmware (and the plain JS driver stepping loop) must run full
    /// budgets, where pending model IRQs never stop execution.
    pub deliver_irqs: bool,
    /// Halted in WFI/WFE (low-power). JS advances virtual time and wakes
    /// via `wake()` when an interrupt is pending. Only set when
    /// `deliver_irqs` is on; otherwise WFI is a nop.
    pub sleeping: bool,
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
            it_pred: false,
            ipsr: 0,
            exc_stack: Vec::new(),
            it_stack: Vec::new(),
            fp_stack: Vec::new(),
            deliver_irqs: false,
            sleeping: false,
        }
    }
    pub fn reset(&mut self, sp: u32, pc: u32) {
        self.regs = Regs::new(sp, pc);
        self.fault = None;
        self.it_cond = 0;
        self.it_mask = 0;
        self.it_n = 0;
        self.it_idx = 0;
        self.ipsr = 0;
        self.exc_stack.clear();
        self.it_stack.clear();
        self.fp_stack.clear();
        self.sleeping = false;
    }

    /// Current stack pointer (r13 always mirrors it).
    #[inline]
    pub fn sp(&self) -> u32 {
        self.regs.r[13]
    }

    /// Read the MSP (banked). In handler mode MSP == r13; in thread+MSP
    /// mode r13 == msp too. Only thread+PSP mode differs.
    pub fn read_msp(&self) -> u32 {
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.msp
        } else {
            self.regs.r[13]
        }
    }
    /// Read the PSP (banked).
    pub fn read_psp(&self) -> u32 {
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.r[13]
        } else {
            self.regs.psp
        }
    }
    /// Write the MSP (banked): updates r13 too when MSP is current.
    pub fn write_msp(&mut self, v: u32) {
        self.regs.msp = v;
        if self.ipsr != 0 || self.regs.control & 2 == 0 {
            self.regs.r[13] = v;
        }
    }
    /// Write the PSP (banked): updates r13 too when PSP is current.
    /// This is the FreeRTOS task-switch primitive (`msr psp, rX` in the
    /// PendSV handler while in handler mode only updates the bank).
    pub fn write_psp(&mut self, v: u32) {
        self.regs.psp = v;
        if self.ipsr == 0 && self.regs.control & 2 != 0 {
            self.regs.r[13] = v;
        }
    }

    /// Take an exception: stack the context, load the handler from the
    /// vector table (via VTOR), set EXC_RETURN. Works for system exceptions
    /// (negative irq) and external IRQs. No nesting in v1 (only called from
    /// thread mode), but the stacking is fully hardware-shaped so ISRs run
    /// unmodified, including FreeRTOS SVC/PendSV/SysTick handlers.
    pub fn take_exception(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, irq: i32) {
        let vector = (16 + irq) as u32;
        // Save IT state; the handler starts with a clean ITSTATE.
        self.it_stack.push(SavedIt {
            cond: self.it_cond,
            mask: self.it_mask,
            n: self.it_n,
            idx: self.it_idx,
        });
        self.it_n = 0;
        self.it_idx = 0;
        self.exc_stack.push(irq);
        // Save the lazy-FP state for nesting (an inner reserve clobbers the
        // model's LSPACT/FPCAR; the pop on return restores the outer frame).
        let prev_fpccr = sys.p.read(sys, 0xE000EF34, 4);
        self.fp_stack.push(SavedFp {
            lspact: prev_fpccr & 1 != 0,
            fpcar: sys.p.read(sys, 0xE000EF38, 4),
        });
        // Bank the thread stack, then run the handler on MSP. The frame
        // goes onto the CURRENT stack (PSP if thread+PSP, else MSP) — this
        // is what makes FreeRTOS task stacks work.
        let was_psp = self.ipsr == 0 && self.regs.control & 2 != 0;
        if was_psp {
            self.regs.psp = self.regs.r[13];
        } else if self.ipsr == 0 {
            self.regs.msp = self.regs.r[13];
        }
        let mut sp = self.regs.r[13];
        // Lazy FP stacking decision FIRST (model read, no mem writes yet):
        // CONTROL.FPCA (thread uses the FPU) + FPCCR.ASPEN select the
        // 26-word extended frame; otherwise the classic 8-word frame.
        let fpccr0 = sys.p.read(sys, 0xE000EF34, 4);
        let fp_ext = self.regs.control & 4 != 0 && fpccr0 & (1 << 31) != 0;
        // ARM frame layout (low->high): R0-R3, R12, LR, PC, xPSR (+0..28),
        // then (extended only) S0-S15 (+32..92), FPSCR (+96), RESERVED.
        sp = sp.wrapping_sub(if fp_ext { 104 } else { 32 });
        mem.write32(sp, self.regs.r[0]);
        mem.write32(sp.wrapping_add(4), self.regs.r[1]);
        mem.write32(sp.wrapping_add(8), self.regs.r[2]);
        mem.write32(sp.wrapping_add(12), self.regs.r[3]);
        mem.write32(sp.wrapping_add(16), self.regs.r[12]);
        mem.write32(sp.wrapping_add(20), self.regs.r[14]);
        mem.write32(sp.wrapping_add(24), self.regs.r[15]);
        // xPSR with the T-bit set (R0 landed lowest, xPSR highest).
        let xpsr = self.regs.xpsr | 0x01000000;
        mem.write32(sp.wrapping_add(28), xpsr);
        // Handler mode always runs on MSP.
        self.regs.r[13] = self.regs.msp;
        // LR = EXC_RETURN selecting the thread stack we came from, with
        // bit 4 (FType) clear when an FP-extended frame was reserved.
        self.regs.r[14] = match (was_psp, fp_ext) {
            (false, false) => EXC_RETURN_MSP,
            (true, false) => EXC_RETURN_PSP,
            (false, true) => EXC_RETURN_MSP_FP,
            (true, true) => EXC_RETURN_PSP_FP,
        };
        // ^ BUG: r13 must be the POST-PUSH sp, not stale msp! Fix below.
        self.regs.r[13] = sp;
        self.regs.msp = sp;
        // Hardware also advances the THREAD bank past the pushed frame, so a
        // later `mrs psp` (PendSV save) points BELOW the entry frame and the
        // stmdb doesn't overwrite it. Without this the entry frame is
        // clobbered and the switch-back unstacks garbage (FreeRTOS slide).
        if was_psp {
            self.regs.psp = sp;
        }
        // Lazy FP stacking (VFPv4-SP): with CONTROL.FPCA (thread uses the
        // FPU) and FPCCR.ASPEN, reserve the 26-word extended frame now.
        // Lazy (LSPEN=1, the reset state): write FPSCR at frame offset 96
        // only, point FPCAR at the S0 slot (offset 32), set LSPACT — S0-S15
        // land on the first handler FPU use (see the thumb.rs FPU hook).
        // Eager (LSPEN=0): stack S0-S15 + FPSCR immediately, LSPACT stays 0.
        // Without FPCA/ASPEN the 8-word integer frame above is the whole
        // story (all pre-FPU firmware, incl. FreeRTOS, is unaffected).
        // (sp already spans the full frame, so the bank sync above and the
        // fp_stack nesting push both cover it as-is.)
        if fp_ext {
            if fpccr0 & (1 << 30) != 0 {
                mem.write32(sp.wrapping_add(96), self.regs.fpscr);
                sys.p.write(sys, 0xE000EF38, 4, sp.wrapping_add(32));
                sys.p.write(sys, 0xE000EF34, 4, fpccr0 | 1);
            } else {
                for i in 0..16 {
                    mem.write32(sp.wrapping_add(32 + 4 * i as u32), self.regs.s[i]);
                }
                mem.write32(sp.wrapping_add(96), self.regs.fpscr);
            }
        }
        self.ipsr = vector;
        sys.p.nvic.borrow_mut().set_in_interrupt(true);
        // Load handler PC through VTOR (model SCB, default 0x08000000).
        let vtor = sys.p.read(sys, 0xE000ED08, 4);
        let handler = mem.read32(vtor.wrapping_add(vector * 4));
        self.regs.r[15] = handler | 1;
    }

    /// Perform an exception return for an EXC_RETURN value in `exc`.
    /// Returns false (with fault recorded) for unsupported returns.
    pub fn exception_return(
        &mut self,
        sys: &WasmSystem,
        mem: &mut dyn Memory,
        exc: u32,
        pc: u32,
    ) -> bool {
        if exc == EXC_RETURN_HANDLER {
            // Return to handler mode (nested) — not supported in v1.
            self.fault = Some(CpuFault { pc, op1: 0x4770, op2: 0, len: 2 });
            return false;
        }
        // EXC_RETURN bit 4 (FType): 0 = FP-extended 26-word frame.
        let extended = exc & 0x10 == 0;
        if exc != EXC_RETURN_MSP
            && exc != EXC_RETURN_PSP
            && exc != EXC_RETURN_MSP_FP
            && exc != EXC_RETURN_PSP_FP
        {
            self.fault = Some(CpuFault { pc, op1: 0x4770, op2: 0, len: 2 });
            return false;
        }
        // Unstack from the bank selected by EXC_RETURN (using CURRENT bank
        // values — a PendSV task switch updates PSP mid-handler). The FP
        // variants (ED/E9) select the same bank as their FType=1 twins.
        let to_psp = exc == EXC_RETURN_PSP || exc == EXC_RETURN_PSP_FP;
        let mut sp = if to_psp { self.regs.psp } else { self.regs.msp };
        // In handler mode r13 == MSP; if returning to MSP it must match.
        // (If a buggy handler moved MSP, trust the bank per ARM.)
        let r0 = mem.read32(sp);
        let r1 = mem.read32(sp.wrapping_add(4));
        let r2 = mem.read32(sp.wrapping_add(8));
        let r3 = mem.read32(sp.wrapping_add(12));
        let r12 = mem.read32(sp.wrapping_add(16));
        let lr = mem.read32(sp.wrapping_add(20));
        let retpc = mem.read32(sp.wrapping_add(24));
        let xpsr = mem.read32(sp.wrapping_add(28));
        sp = sp.wrapping_add(32);
        if extended {
            // FP-extended frame: S0-S15 at sp+0..60, FPSCR at sp+64 (sp
            // already advanced past the integer 8 words). Lazy-never-
            // stacked (LSPACT set): FPSCR only; else the full S file.
            // Either way the frame is 104 bytes total.
            let fpccr = sys.p.read(sys, 0xE000EF34, 4);
            if fpccr & 1 != 0 {
                self.regs.fpscr = mem.read32(sp.wrapping_add(64));
            } else {
                for i in 0..16 {
                    self.regs.s[i] = mem.read32(sp.wrapping_add(4 * i as u32));
                }
                self.regs.fpscr = mem.read32(sp.wrapping_add(64));
            }
            sp = sp.wrapping_add(72);
            // Pop the nesting state back into the model (an inner reserve
            // clobbered LSPACT/FPCAR; the outer frame owns them again now).
            if let Some(saved) = self.fp_stack.pop() {
                let cur = sys.p.read(sys, 0xE000EF34, 4);
                let restored = if saved.lspact { cur | 1 } else { cur & !1 };
                sys.p.write(sys, 0xE000EF34, 4, restored);
                sys.p.write(sys, 0xE000EF38, 4, saved.fpcar);
            }
        }
        self.regs.r[0] = r0;
        self.regs.r[1] = r1;
        self.regs.r[2] = r2;
        self.regs.r[3] = r3;
        self.regs.r[12] = r12;
        self.regs.r[14] = lr;
        // Restore flags (APSR) + IT/ICI bits live in xPSR; T-bit stays set.
        self.regs.xpsr = (xpsr & 0xF8000000) | 0x01000000;
        self.regs.r[13] = sp;
        if to_psp {
            self.regs.psp = sp;
        } else {
            self.regs.msp = sp;
        }
        // Exception return selects the thread stack AND updates CONTROL.SPSEL
        // to match (hardware keeps them coherent; without this every
        // CONTROL-gated bank decision after the first return is wrong and
        // PendSV saves to a stale PSP — the FreeRTOS wedge). Bit0
        // (privilege) is preserved.
        if to_psp {
            self.regs.control |= 2;
        } else {
            self.regs.control &= !2;
        }
        // Restore the pre-exception IT state.
        if let Some(saved) = self.it_stack.pop() {
            self.it_cond = saved.cond;
            self.it_mask = saved.mask;
            self.it_n = saved.n;
            self.it_idx = saved.idx;
        }
        self.exc_stack.pop();
        self.ipsr = 0;
        sys.p.nvic.borrow_mut().set_in_interrupt(false);
        // Chained PendSV/SVC tail? No tail-chaining in v1; the run loop
        // delivers the next pending exception on the next iteration.
        self.regs.r[15] = retpc | 1;
        // NOTE: no even-retpc fault here. FreeRTOS's M4 port deliberately
        // stores the task entry with bit0 CLEAR (`bic r1, #1` in
        // pxPortInitialiseStack) and relies on exception return forcing
        // Thumb state; Unicorn accepts this and the firmware is proven on
        // it, so we force |1 like hardware does for the PC load.
        true
    }

    pub fn run(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, budget: u32) -> u32 {
        let mut done = 0;
        // Publish executed-instruction progress to the shared virtual clock
        // in small chunks, so model reads mid-step (polled timer counters,
        // watchdog edges) observe time advancing instead of a frozen count.
        // Chunked rather than per-instruction to keep the atomic off the
        // hottest path; the remainder flushes at step end. Model-side delta
        // bookkeeping (last_tick) partitions the interval exactly, so chunked
        // publishing neither gains nor loses ticks vs one batch.
        while done < budget {
            if self.fault.is_some() {
                break;
            }
            if self.sleeping {
                break;
            }
            let pc = self.regs.r[15] & !1;
            if TRACE_ON.load(Ordering::Relaxed) {
                TRACE_BUF.lock().unwrap().push(pc);
            }
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
            if done & 15 == 0 {
                crate::system::INSTRUCTION_COUNT.fetch_add(16u64, Ordering::Relaxed);
            }
            // Keep the inactive... no — keep the CURRENT stack bank in sync
            // with r13 after every thread-mode instruction. PUSH/POP/ADD-SP
            // and LDM/STM writeback move r13 directly; without this the bank
            // goes stale and the next `mrs psp` (PendSV context switch) saves
            // r4-r11 at the wrong address, stranding the live stack (this
            // wedged FreeRTOS: high_top saved as stale_psp-32). Handler mode
            // (ipsr != 0) is skipped: take_exception/exception_return manage
            // the banks explicitly there, and r13 == MSP throughout.
            if self.ipsr == 0 {
                if self.regs.control & 2 != 0 {
                    self.regs.psp = self.regs.r[13];
                } else {
                    self.regs.msp = self.regs.r[13];
                }
            }
            self.cycles += 1;
            // Inline interrupt delivery (no ISR pump needed): take the next
            // deliverable exception when in thread mode with PRIMASK clear.
            // Stacking is exact, so the mid-`str` PENDSVSET hazard of
            // AGENTS.md §9 cannot occur — the store completes, PC advances,
            // then we stack the next PC.)
            if self.deliver_irqs && self.ipsr == 0 && self.regs.primask == 0 {
                let pending = sys.p.nvic.borrow().has_pending();
                if pending {
                    // Bind first: `if let` would extend the borrow_mut guard
                    // through the body and take_exception would re-borrow.
                    let next = sys.p.nvic.borrow_mut().get_and_clear_next_intr_pending();
                    if let Some(irq) = next {
                        self.take_exception(sys, mem, irq);
                    }
                }
            }
        }
        crate::system::INSTRUCTION_COUNT.fetch_add((done & 15) as u64, Ordering::Relaxed);
        done
    }
}
