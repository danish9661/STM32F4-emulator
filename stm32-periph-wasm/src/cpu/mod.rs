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

/// EXC_RETURN values we support: thread returns (F9/FD, +FP twins E9/ED)
/// and handler returns for nested preemption (F1/E1).
pub const EXC_RETURN_MSP: u32 = 0xFFFFFFF9;
pub const EXC_RETURN_PSP: u32 = 0xFFFFFFFD;
pub const EXC_RETURN_HANDLER: u32 = 0xFFFFFFF1;
/// Same, with an FP-extended frame (EXC_RETURN bit 4 == 0): issued when the
/// stacked frame reserved S0-S15.
pub const EXC_RETURN_MSP_FP: u32 = 0xFFFFFFE9;
pub const EXC_RETURN_PSP_FP: u32 = 0xFFFFFFED;
/// Return to handler mode (nested preemption): unstacks from MSP and
/// resumes the outer handler. FP variant for an FP-extended frame.
pub const EXC_RETURN_HANDLER_FP: u32 = 0xFFFFFFE1;

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

/// One entered exception: the IRQ (for IPSR restore on nested returns and
/// NVIC active-bit hygiene), the stack its frame went onto, and the frame
/// size the eventual return must unstack (tail-chain reuses it instead).
#[derive(Clone, Copy, Debug)]
struct ExcEntry {
    irq: i32,
    to_psp: bool,
    fp_ext: bool,
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
    /// Entered exceptions (innermost last): drives IPSR, execution
    /// priority, and tail-chain frame reuse.
    exc_stack: Vec<ExcEntry>,
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
    /// Local event register for WFE/SEV: SEV sets it; WFE with it set
    /// clears it and skips sleep; exception entry/return set it (silicon
    /// rule — an ISR running means a later WFE must not nap). Single-core,
    /// so no cross-core event fabric is needed.
    pub event_register: bool,
}

impl Cpu {
    pub fn new(sp: u32, pc: u32) -> Self {
        // Fresh CPU boots privileged in thread mode (CONTROL reset = 0).
        crate::system::set_cpu_context(true, false);
        crate::system::set_current_ipsr(0);
        // Native tests share one process: a previous test's MPU state must
        // not leak into this instance (the fresh SYS has CTRL=0 regions).
        crate::system::set_mpu_enabled(false);
        crate::system::set_mpu_force_unpriv(false);
        crate::system::set_unalign_trp(false);
        let _ = crate::system::take_mpu_fault();
        let _ = crate::system::take_align_fault();
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
            event_register: false,
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
        self.event_register = false;
        crate::system::set_cpu_context(true, false);
        crate::system::set_current_ipsr(0);
        // A reboot must not inherit a deferred fault from the old run.
        let _ = crate::system::take_mpu_fault();
        let _ = crate::system::take_align_fault();
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

    /// Programmed priority of an exception (lower number = more urgent).
    /// Fixed -2/-1 for NMI/HardFault; SHPR bytes for system handlers;
    /// NVIC IPR bytes for external IRQs. Raw bytes throughout: firmware
    /// writes its shifted values to both IPR and BASEPRI, so raw compare
    /// orders identically to silicon's top-bits compare.
    fn exc_priority(sys: &WasmSystem, irq: i32) -> i32 {
        match irq {
            -14 => -2, // NMI
            -13 => -1, // HardFault
            -12 | -11 | -10 => {
                let shpr1 = sys.p.read(sys, 0xE000ED18, 4);
                ((shpr1 >> (8 * (irq + 12) as u32)) & 0xFF) as i32
            }
            -5 => ((sys.p.read(sys, 0xE000ED1C, 4) >> 24) & 0xFF) as i32, // SVCall
            -4 => ((sys.p.read(sys, 0xE000ED20, 4)) & 0xFF) as i32,       // DebugMon
            -2 => ((sys.p.read(sys, 0xE000ED20, 4) >> 16) & 0xFF) as i32, // PendSV
            -1 => ((sys.p.read(sys, 0xE000ED20, 4) >> 24) & 0xFF) as i32, // SysTick
            _ if irq >= 0 => sys.p.nvic.borrow().ext_priority(irq) as i32,
            _ => 0, // reserved system slots (-3,-6..-9): never generated
        }
    }

    /// AIRCR.PRIGROUP split point (0-7): subpriority occupies the low
    /// PRIGROUP+1 bits, group (preemption) priority the rest. Reset 0
    /// (7 group + 1 sub); FreeRTOS writes 7 (all subpriority within the
    /// implemented bits, so nothing preempts and BASEPRI masks uniformly).
    fn aircr_prigroup(sys: &WasmSystem) -> u32 {
        (sys.p.read(sys, 0xE000ED0C, 4) >> 8) & 7
    }

    /// Group (preemption) priority: raw byte shifted past the subpriority
    /// field. Fixed negatives pass through (NMI/HardFault have no split).
    /// Only the group decides preemption and BASEPRI masking (silicon
    /// rule); the subpriority only tie-breaks simultaneously-pending
    /// exceptions of the same group.
    fn exc_group(sys: &WasmSystem, irq: i32) -> i32 {
        let raw = Self::exc_priority(sys, irq);
        if raw < 0 {
            raw
        } else {
            (raw as u32 >> (Self::aircr_prigroup(sys) + 1)) as i32
        }
    }

    /// Subpriority (tie-break within a group): the low PRIGROUP+1 bits.
    fn exc_sub(sys: &WasmSystem, irq: i32) -> u32 {
        let raw = Self::exc_priority(sys, irq);
        if raw < 0 {
            return 0;
        }
        let bits = Self::aircr_prigroup(sys) + 1;
        if bits >= 8 {
            raw as u32
        } else {
            (raw as u32) & ((1 << bits) - 1)
        }
    }

    /// BASEPRI compared in group space (same split as priorities: firmware
    /// writes shifted values to both, so group-vs-group matches silicon's
    /// masked-set exactly under the CMSIS convention).
    fn basepri_group(&self, sys: &WasmSystem) -> i32 {
        (self.regs.basepri as u32 >> (Self::aircr_prigroup(sys) + 1)) as i32
    }

    /// Execution priority: the innermost active handler's group, or 256
    /// (idle, lower than any programmable 0..255) in thread mode.
    fn execution_priority(&self, sys: &WasmSystem) -> i32 {
        match self.exc_stack.last() {
            Some(e) => Self::exc_group(sys, e.irq),
            None => 256,
        }
    }

    /// Whether an exception is masked right now (PRIMASK/FAULTMASK/BASEPRI).
    /// NMI is never masked; HardFault (-1) is only stopped by FAULTMASK.
    fn exception_masked(&self, sys: &WasmSystem, irq: i32) -> bool {
        if irq == -14 {
            return false;
        }
        if self.regs.faultmask {
            return true;
        }
        let group = Self::exc_group(sys, irq);
        if group < 0 {
            return false;
        }
        if self.regs.primask != 0 {
            return true;
        }
        if self.regs.basepri != 0 && group >= self.basepri_group(sys) {
            return true;
        }
        false
    }

    /// Best pending exception: enabled, unmasked, and urgent enough to
    /// preempt current execution. Group asc, then subpriority asc, then
    /// exception number asc (silicon order). Late arrival needs no special
    /// case: selection runs at every instruction boundary, so the most
    /// urgent pending exception always wins before any handler's first
    /// instruction.
    fn select_pending(&self, sys: &WasmSystem) -> Option<i32> {
        let exec_prio = self.execution_priority(sys);
        let bits = sys.p.nvic.borrow().pending_bits();
        let mut best: Option<(i32, u32, i32)> = None; // (group, sub, irq)
        let mut b = bits;
        while b != 0 {
            let bit = b.trailing_zeros() as i32;
            b &= !(1u128 << bit);
            let irq = bit - 16;
            // Enabled gate (external IRQs need ISER; system always on).
            let enabled = if irq < 0 {
                true
            } else {
                sys.p.nvic.borrow().is_enabled(irq)
            };
            if !enabled {
                continue;
            }
            // Mask gates (PRIMASK/FAULTMASK/BASEPRI) + preemption gate.
            if self.exception_masked(sys, irq) {
                continue;
            }
            let group = Self::exc_group(sys, irq);
            if group >= exec_prio {
                continue;
            }
            let sub = Self::exc_sub(sys, irq);
            let better = match best {
                None => true,
                Some((bg, bs, bi)) => (group, sub, irq) < (bg, bs, bi),
            };
            if better {
                best = Some((group, sub, irq));
            }
        }
        best.map(|(_, _, irq)| irq)
    }

    /// Raise a synchronous exception (SVC insn, MPU fault, UsageFault):
    /// take it when its priority permits activation, else escalate to
    /// HardFault (a synchronous fault cannot wait — silicon escalates; the
    /// model's one-instruction deferral of data faults does not change
    /// that). A blocked HardFault is lockup: loud halt. With delivery off
    /// everything halts loudly (polling firmware never raises).
    /// `pub(crate)` for the thumb decoder's SVC/NOCP arms.
    pub(crate) fn raise_sync(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, irq: i32) {
        if !self.deliver_irqs {
            self.fault = Some(CpuFault { pc: self.regs.r[15] & !1, op1: 0xDEAD, op2: 0, len: 2 });
            return;
        }
        // External synchronous raises must be enabled; else pend and wait.
        if irq >= 0 && !sys.p.nvic.borrow().is_enabled(irq) {
            sys.p.nvic.borrow_mut().set_intr_pending(irq);
            return;
        }
        let blocked =
            self.exception_masked(sys, irq) || Self::exc_group(sys, irq) >= self.execution_priority(sys);
        if blocked {
            if irq == -13 || irq == -14 {
                // Even HardFault is blocked (FAULTMASK): silicon lockup.
                self.fault = Some(CpuFault { pc: self.regs.r[15] & !1, op1: 0xDEAD, op2: 0, len: 2 });
                return;
            }
            self.raise_sync(sys, mem, -13);
            return;
        }
        sys.p.nvic.borrow_mut().take_pending_irq(irq);
        self.take_exception(sys, mem, irq);
    }

    /// WFI/WFE sleep-entry rule for the thumb decoder (`pub(crate)`): sleep
    /// unless an exception could be taken right now. Wake is deliberately
    /// eager (any pending, via the driver's has_pending check): silicon
    /// wakes WFI on pending-enabled interrupts even when masked, then
    /// continues past the WFI without entering them.
    pub(crate) fn select_pending_for_sleep(&self, sys: &WasmSystem) -> bool {
        self.select_pending(sys).is_none()
    }

    /// Take an exception: stack the context, load the handler from the
    /// vector table (via VTOR), set EXC_RETURN. Works for system exceptions
    /// (negative irq) and external IRQs, thread or nested (an IRQ taken in
    /// handler mode runs on MSP and returns via F1 to the outer handler).
    pub fn take_exception(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, irq: i32) {
        let vector = (16 + irq) as u32;
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
        // Lazy FP stacking decision (model read, no mem writes yet):
        // CONTROL.FPCA (thread uses the FPU) + FPCCR.ASPEN select the
        // 26-word extended frame; otherwise the classic 8-word frame.
        let fpccr0 = sys.p.read(sys, 0xE000EF34, 4);
        let fp_ext = self.regs.control & 4 != 0 && fpccr0 & (1 << 31) != 0;
        // ARM frame layout (low->high): R0-R3, R12, LR, PC, xPSR (+0..28),
        // then (extended only) S0-S15 (+32..92), FPSCR (+96), RESERVED.
        sp = sp.wrapping_sub(if fp_ext { 104 } else { 32 });
        // STKALIGN (CCR bit 9, silicon reset 1): 8-byte-align the frame,
        // padding one word below it and flagging xPSR bit 9 (the return
        // path skips the pad from the stacked xPSR).
        let mut pad = 0u32;
        if sys.p.read(sys, 0xE000ED14, 4) & (1 << 9) != 0 && sp & 4 != 0 {
            sp = sp.wrapping_sub(4);
            pad = 4;
        }
        // Stacking is privileged even when the thread is unprivileged
        // (silicon rule), so publish handler context BEFORE the
        // pre-validation: the frame check must use handler privilege.
        crate::system::set_cpu_context(true, irq == -13 || irq == -14);
        // Pre-validate the whole frame when the MPU is on: a stacking
        // write into denied memory latches MSTKERR (no MMFAR) and halts
        // loudly BEFORE anything is pushed or written (delivery is
        // impossible — the stack needed to take an exception is itself
        // broken, unrecoverable by construction).
        if crate::system::is_mpu_enabled()
            && sys.p.mpu_check(sp, (if fp_ext { 104 } else { 32 }) + pad, true, false).is_some()
        {
            crate::system::latch_memmanage_fault(sys, 1 << 4, None);
            self.fault = Some(CpuFault { pc: self.regs.r[15] & !1, op1: 0xDEAD, op2: 0, len: 2 });
            return;
        }
        // Frame base sits above the pad word (all store offsets below are
        // from here; r13/MSP keep the post-pad SP so the return advance
        // covers frame+pad together).
        // Save IT state; the handler starts with a clean ITSTATE.
        self.it_stack.push(SavedIt {
            cond: self.it_cond,
            mask: self.it_mask,
            n: self.it_n,
            idx: self.it_idx,
        });
        self.it_n = 0;
        self.it_idx = 0;
        let nested = self.ipsr != 0;
        self.exc_stack.push(ExcEntry { irq, to_psp: was_psp, fp_ext });
        // Save the lazy-FP state for nesting (an inner reserve clobbers the
        // model's LSPACT/FPCAR; the pop on return restores the outer frame).
        let prev_fpccr = sys.p.read(sys, 0xE000EF34, 4);
        self.fp_stack.push(SavedFp {
            lspact: prev_fpccr & 1 != 0,
            fpcar: sys.p.read(sys, 0xE000EF38, 4),
        });
        mem.write32(sp, self.regs.r[0]);
        mem.write32(sp.wrapping_add(4), self.regs.r[1]);
        mem.write32(sp.wrapping_add(8), self.regs.r[2]);
        mem.write32(sp.wrapping_add(12), self.regs.r[3]);
        mem.write32(sp.wrapping_add(16), self.regs.r[12]);
        mem.write32(sp.wrapping_add(20), self.regs.r[14]);
        mem.write32(sp.wrapping_add(24), self.regs.r[15]);
        // xPSR with the T-bit set (R0 landed lowest, xPSR highest), plus
        // the ALIGN pad flag (bit 9) when STKALIGN padded above.
        let xpsr = self.regs.xpsr | 0x01000000 | if pad != 0 { 0x200 } else { 0 };
        mem.write32(sp.wrapping_add(28), xpsr);
        // Handler mode always runs on MSP.
        self.regs.r[13] = self.regs.msp;
        // LR = EXC_RETURN for where this handler returns to: a nested take
        // (preemption) returns via F1 to the outer handler; a thread take
        // selects the thread stack it came from. FType bit follows the
        // reserved frame size.
        self.regs.r[14] = match (nested, was_psp, fp_ext) {
            (true, _, false) => EXC_RETURN_HANDLER,
            (true, _, true) => EXC_RETURN_HANDLER_FP,
            (false, false, false) => EXC_RETURN_MSP,
            (false, true, false) => EXC_RETURN_PSP,
            (false, false, true) => EXC_RETURN_MSP_FP,
            (false, true, true) => EXC_RETURN_PSP_FP,
        };
        // FAULTMASK clears on every exception entry except NMI (silicon:
        // a fault handler must be able to fault again, or nothing would
        // ever escalate twice).
        if irq != -14 {
            self.regs.faultmask = false;
        }
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
        // Exception entry sets the local event register (a WFE after this
        // ISR must observe the event and skip sleep).
        self.event_register = true;
        crate::system::set_current_ipsr(vector);
        Self::set_shcsr_active(sys, irq);
        // Activation clears a stored ICSR SET-pending bit (the model
        // pending bit cleared at take time; without this ICSR reads stale).
        if irq == -2 {
            sys.p.write(sys, 0xE000ED04, 4, 1 << 27); // PENDSVCLR
        } else if irq == -1 {
            sys.p.write(sys, 0xE000ED04, 4, 1 << 25); // SYSTICKCLR
        }
        // (CPU privilege context was already published as handler-privileged
        // before the stacking pre-validation above.)
        sys.p.nvic.borrow_mut().set_in_interrupt(true);
        // Load handler PC through VTOR (model SCB, default 0x08000000).
        let vtor = sys.p.read(sys, 0xE000ED08, 4);
        let handler = mem.read32(vtor.wrapping_add(vector * 4));
        self.regs.r[15] = handler | 1;
    }

    /// Perform an exception return for an EXC_RETURN value in `exc`.
    /// Handles thread returns (F9/FD + FP twins, IPSR back to 0) and nested
    /// handler returns (F1/E1, IPSR back to the outer vector). Returns false
    /// (with fault recorded) for unsupported values. On a return that
    /// leaves a pendable exception ready, tail-chains straight into it
    /// instead of unstacking (the frame is reused, like silicon).
    pub fn exception_return(
        &mut self,
        sys: &WasmSystem,
        mem: &mut dyn Memory,
        exc: u32,
        pc: u32,
    ) -> bool {
        // EXC_RETURN bit 4 (FType): 0 = FP-extended 26-word frame.
        let extended = exc & 0x10 == 0;
        let to_handler = exc == EXC_RETURN_HANDLER || exc == EXC_RETURN_HANDLER_FP;
        if exc != EXC_RETURN_MSP
            && exc != EXC_RETURN_PSP
            && exc != EXC_RETURN_MSP_FP
            && exc != EXC_RETURN_PSP_FP
            && !to_handler
        {
            self.fault = Some(CpuFault { pc, op1: 0x4770, op2: 0, len: 2 });
            return false;
        }
        // The top entry owns the top frame: FType must agree with the
        // recorded frame size (silicon INVPC), and a return needs a frame.
        let recorded_ext = match self.exc_stack.last() {
            Some(e) => e.fp_ext,
            None => {
                Self::latch_invpc(sys);
                self.raise_sync(sys, mem, Self::usage_target(sys));
                return self.fault.is_none();
            }
        };
        if extended != recorded_ext {
            Self::latch_invpc(sys);
            self.raise_sync(sys, mem, Self::usage_target(sys));
            return self.fault.is_none();
        }
        // Unstack from the bank selected by EXC_RETURN (using CURRENT bank
        // values — a PendSV task switch updates PSP mid-handler). The FP
        // variants (ED/E9) select the same bank as their FType=1 twins;
        // F1/E1 always unstack from MSP (handler mode runs on MSP).
        let to_psp = exc == EXC_RETURN_PSP || exc == EXC_RETURN_PSP_FP;
        let mut sp = if to_psp { self.regs.psp } else { self.regs.msp };
        // Pop the returning entry's saved state first (its IT/FP context is
        // done; the outer frame owns the model FP state again). The FP pop
        // is unconditional: every take pushes exactly one save, so every
        // return pops one — 8-word returns used to leak these.
        if let Some(saved) = self.it_stack.pop() {
            self.it_cond = saved.cond;
            self.it_mask = saved.mask;
            self.it_n = saved.n;
            self.it_idx = saved.idx;
        }
        if let Some(saved) = self.fp_stack.pop() {
            let cur = sys.p.read(sys, 0xE000EF34, 4);
            let restored = if saved.lspact { cur | 1 } else { cur & !1 };
            sys.p.write(sys, 0xE000EF34, 4, restored);
            sys.p.write(sys, 0xE000EF38, 4, saved.fpcar);
        }
        let returned = self.exc_stack.pop().unwrap();
        sys.p.nvic.borrow_mut().clear_active(returned.irq);
        Self::clear_shcsr_active(sys, returned.irq);
        // Tail-chain: a ready exception reuses this frame instead of paying
        // unstack + re-stack (silicon skips both). Only when the incoming
        // handler needs the same frame size the returnee reserved; an FP
        // mismatch falls through to the normal unstack and the run loop
        // delivers next iteration (always correct, just one stack cycle).
        if self.deliver_irqs {
            if let Some(irq) = self.select_pending(sys) {
                let fpccr = sys.p.read(sys, 0xE000EF34, 4);
                let need_fp = self.regs.control & 4 != 0 && fpccr & (1 << 31) != 0;
                if need_fp == returned.fp_ext {
                    self.enter_chained(sys, mem, irq, returned.to_psp, returned.fp_ext);
                    return self.fault.is_none();
                }
            }
        }
        // A thread return (F9/FD) with a live outer handler means a forged
        // LR: fault (INVPC) before touching memory, so the escalated
        // handler stacks a clean context instead of half-unstacked regs.
        if !to_handler && !self.exc_stack.is_empty() {
            Self::latch_invpc(sys);
            self.raise_sync(sys, mem, Self::usage_target(sys));
            return self.fault.is_none();
        }
        // Pre-validate the whole frame when the MPU is on (same
        // unrecoverable-halt rule as entry: MUNSTKERR, no MMFAR). Unstack
        // READS the frame, so this is a read check.
        if crate::system::is_mpu_enabled()
            && sys.p.mpu_check(sp, if extended { 104 } else { 32 }, false, false).is_some()
        {
            crate::system::latch_memmanage_fault(sys, 1 << 3, None);
            self.fault = Some(CpuFault { pc, op1: 0xDEAD, op2: 0, len: 2 });
            return false;
        }
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
        // STKALIGN pad word below the frame, flagged by stacked xPSR bit 9
        // (set on entry when the pre-push SP was only 4-aligned).
        sp = sp.wrapping_add(32 + if xpsr & 0x200 != 0 { 4 } else { 0 });
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
        // Write the advanced SP back to its bank (both paths: the F1 branch
        // below returns before the thread-only CONTROL update, but the MSP
        // bank must still advance past the popped inner frame — otherwise a
        // later return unstacks the same frame twice).
        if to_psp {
            self.regs.psp = sp;
        } else {
            self.regs.msp = sp;
        }
        if to_handler {
            // Nested return: the outer handler is still active (r13 stays
            // MSP, CONTROL.SPSEL untouched — handler mode either way).
            match self.exc_stack.last() {
                Some(outer) => {
                    let outer_vector = (16 + outer.irq) as u32;
                    self.ipsr = outer_vector;
                    crate::system::set_current_ipsr(outer_vector);
                }
                None => {
                    // F1 with nothing active: forged LR, fault loudly.
                    Self::latch_invpc(sys);
                    self.raise_sync(sys, mem, Self::usage_target(sys));
                    return self.fault.is_none();
                }
            }
            crate::system::set_cpu_context(true, false);
            sys.p.nvic.borrow_mut().set_in_interrupt(true);
            // Exception return sets the event register (a WFE sequenced
            // after this return must not nap).
            self.event_register = true;
            self.regs.r[15] = retpc | 1;
            return true;
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
        self.ipsr = 0;
        crate::system::set_current_ipsr(0);
        // Thread privilege follows CONTROL.nPRIV from here on (handlers
        // always return to unprivileged-safe state only via explicit MSR).
        crate::system::set_cpu_context((self.regs.control & 1) == 0, false);
        sys.p.nvic.borrow_mut().set_in_interrupt(false);
        self.event_register = true;
        self.regs.r[15] = retpc | 1;
        // SLEEPONEXIT (SCR bit 1): a return to thread naps the core until
        // the next deliverable interrupt, like a WFI right after the `bx lr`.
        if self.deliver_irqs && sys.p.read(sys, 0xE000ED10, 4) & 2 != 0 {
            self.sleeping = true;
        }
        // NOTE: no even-retpc fault here. FreeRTOS's M4 port deliberately
        // stores the task entry with bit0 CLEAR (`bic r1, #1` in
        // pxPortInitialiseStack) and relies on exception return forcing
        // Thumb state; Unicorn accepts this and the firmware is proven on
        // it, so we force |1 like hardware does for the PC load.
        true
    }

    /// Latch UFSR.INVPC (attempted return with a bad EXC_RETURN/frame).
    fn latch_invpc(sys: &WasmSystem) {
        let cfsr = sys.p.read(sys, 0xE000ED28, 4);
        sys.p.write(sys, 0xE000ED28, 4, cfsr | (1 << 10));
    }

    /// SHCSR active bit for a system handler (MemManage bit 0, BusFault 1,
    /// UsageFault 3, SVCall 7, PendSV 10, SysTick 11). External IRQs use
    /// NVIC IABR instead (maintained separately).
    fn shcsr_act_bit(irq: i32) -> u32 {
        match irq {
            -12 => 1 << 0,
            -11 => 1 << 1,
            -10 => 1 << 3,
            -5 => 1 << 7,
            -2 => 1 << 10,
            -1 => 1 << 11,
            _ => 0,
        }
    }

    /// Set/clear the SHCSR active bit on entry/return (read-modify-write
    /// so guest-written SHCSR bits are preserved).
    fn set_shcsr_active(sys: &WasmSystem, irq: i32) {
        let b = Self::shcsr_act_bit(irq);
        if b != 0 {
            let s = sys.p.read(sys, 0xE000ED24, 4);
            sys.p.write(sys, 0xE000ED24, 4, s | b);
        }
    }

    fn clear_shcsr_active(sys: &WasmSystem, irq: i32) {
        let b = Self::shcsr_act_bit(irq);
        if b != 0 {
            let s = sys.p.read(sys, 0xE000ED24, 4);
            sys.p.write(sys, 0xE000ED24, 4, s & !b);
        }
    }

    /// UsageFault target honoring SHCSR.USGFAULTENA (bit 18): without it
    /// the fault escalates to HardFault (silicon rule). `pub(crate)` for
    /// the thumb decoder's fault arms.
    pub(crate) fn usage_target(sys: &WasmSystem) -> i32 {
        if sys.p.read(sys, 0xE000ED24, 4) & (1 << 18) != 0 {
            -10
        } else {
            -13
        }
    }

    /// Enter a tail-chained exception reusing the just-returned frame (no
    /// stacking, no unstacking): push fresh nesting state, point at the new
    /// handler, and set LR for the frame's eventual owner (outer handler
    /// via F1, else the thread stack the frame came from).
    fn enter_chained(
        &mut self,
        sys: &WasmSystem,
        mem: &mut dyn Memory,
        irq: i32,
        to_psp: bool,
        fp_ext: bool,
    ) {
        sys.p.nvic.borrow_mut().take_pending_irq(irq);
        self.it_stack.push(SavedIt {
            cond: self.it_cond,
            mask: self.it_mask,
            n: self.it_n,
            idx: self.it_idx,
        });
        self.it_n = 0;
        self.it_idx = 0;
        let prev_fpccr = sys.p.read(sys, 0xE000EF34, 4);
        self.fp_stack.push(SavedFp {
            lspact: prev_fpccr & 1 != 0,
            fpcar: sys.p.read(sys, 0xE000EF38, 4),
        });
        self.exc_stack.push(ExcEntry { irq, to_psp, fp_ext });
        // The chained handler needs no FP reserve of its own here: the
        // reused frame already spans it (sizes matched), and a lazy reserve
        // just reuses FPCAR (the first-FPU-use hook stacks into the frame).
        let vector = (16 + irq) as u32;
        // Outer handler still active (stack deeper than this entry) means
        // the eventual return goes back to handler mode via F1.
        let nested = self.exc_stack.len() > 1;
        self.regs.r[14] = if nested {
            if fp_ext {
                EXC_RETURN_HANDLER_FP
            } else {
                EXC_RETURN_HANDLER
            }
        } else if to_psp {
            if fp_ext {
                EXC_RETURN_PSP_FP
            } else {
                EXC_RETURN_PSP
            }
        } else if fp_ext {
            EXC_RETURN_MSP_FP
        } else {
            EXC_RETURN_MSP
        };
        self.ipsr = vector;
        self.event_register = true;
        crate::system::set_current_ipsr(vector);
        Self::set_shcsr_active(sys, irq);
        if irq == -2 {
            sys.p.write(sys, 0xE000ED04, 4, 1 << 27); // PENDSVCLR
        } else if irq == -1 {
            sys.p.write(sys, 0xE000ED04, 4, 1 << 25); // SYSTICKCLR
        }
        crate::system::set_cpu_context(true, irq == -13 || irq == -14);
        sys.p.nvic.borrow_mut().set_in_interrupt(true);
        let vtor = sys.p.read(sys, 0xE000ED08, 4);
        let handler = mem.read32(vtor.wrapping_add(vector * 4));
        self.regs.r[15] = handler | 1;
    }

    /// Raise a MemManage (or escalated HardFault) for an MPU violation.
    /// SHCSR.MEMFAULTENA clear escalates to HardFault (silicon rule);
    /// priority gating/escalation runs through raise_sync. With delivery
    /// off this is a loud CPU halt like SVC/NOCP. Any stale deferred fault
    /// is discarded first (a synchronous raise supersedes it).
    fn raise_memmanage(&mut self, sys: &WasmSystem, mem: &mut dyn Memory, addr: u32, exec: bool) {
        let _ = crate::system::take_mpu_fault();
        let (bits, mar) = if exec {
            (1 << 0, Some(addr)) // IACCVIOL + MMARVALID
        } else {
            (1 << 1, Some(addr)) // DACCVIOL + MMARVALID
        };
        crate::system::latch_memmanage_fault(sys, bits | (1 << 7), mar);
        let shcsr = sys.p.read(sys, 0xE000ED24, 4);
        let irq = if shcsr & (1 << 16) != 0 { -12 } else { -13 };
        self.raise_sync(sys, mem, irq);
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
            // Deferred MPU data fault from the previous instruction (see
            // mem.rs): raise before fetching the next one. Takes precedence
            // over new interrupt delivery (the fault is older).
            if let Some((addr, exec)) = crate::system::take_mpu_fault() {
                self.raise_memmanage(sys, mem, addr, exec);
                // raise_memmanage either takes an exception (continues
                // below) or records a loud halt (breaks next check).
                if self.fault.is_some() {
                    break;
                }
                continue;
            }
            // Same channel for CCR.UNALIGN_TRP: latch UNALIGNED and raise
            // through the UsageFault/HardFault routing (UsageFaults carry
            // no fault address register, so only presence matters).
            if crate::system::take_align_fault().is_some() {
                let cfsr = sys.p.read(sys, 0xE000ED28, 4);
                sys.p.write(sys, 0xE000ED28, 4, cfsr | (1 << 24));
                self.raise_sync(sys, mem, Self::usage_target(sys));
                if self.fault.is_some() {
                    break;
                }
                continue;
            }
            let pc = self.regs.r[15] & !1;
            if TRACE_ON.load(Ordering::Relaxed) {
                TRACE_BUF.lock().unwrap().push(pc);
            }
            let op = mem.read16(pc);
            let l = thumb::len(op);
            // Precise instruction-fetch XN check (MPU-enabled only; state
            // is clean here so violations raise synchronously, unlike the
            // deferred data path). Covers both halfwords of wide insns.
            if crate::system::is_mpu_enabled() {
                let bad = sys.p.mpu_check(pc, 2, false, true).is_some()
                    || (l == 4 && sys.p.mpu_check(pc.wrapping_add(2), 2, false, true).is_some());
                if bad {
                    self.raise_memmanage(sys, mem, pc, true);
                    if self.fault.is_some() {
                        break;
                    }
                    continue;
                }
            }
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
            // Inline interrupt delivery with priority preemption (no ISR
            // pump needed): after every instruction, take the best pending
            // exception that is enabled, unmasked, and urgent enough to
            // preempt — in thread mode or nested inside a handler. Stacking
            // is exact, so the mid-`str` PENDSVSET hazard of AGENTS.md §9
            // cannot occur — the store completes, PC advances, then we
            // stack the next PC.)
            if self.deliver_irqs {
                if let Some(irq) = self.select_pending(sys) {
                    // Bind first: `if let` would extend the borrow_mut guard
                    // through the body and take_exception would re-borrow.
                    sys.p.nvic.borrow_mut().take_pending_irq(irq);
                    self.take_exception(sys, mem, irq);
                }
            }
        }
        crate::system::INSTRUCTION_COUNT.fetch_add((done & 15) as u64, Ordering::Relaxed);
        done
    }
}
