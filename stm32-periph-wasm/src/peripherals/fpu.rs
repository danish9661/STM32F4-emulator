use crate::system::System;
use super::Peripheral;

/// FPU system registers (SVD `FPU` peripheral at 0xE000EF34: FPCCR/FPCAR/
/// FPDSCR + the M4F MVFR0-2 ID values, which the SVD omits). The S0-S31
/// file and FPSCR live in the CPU core (`cpu::regs`); this owns only the
/// memory-mapped system side. No lazy stacking in v1 (see the CPACR-gate
/// note in `cpu::thumb`): FPCCR ASPEN/LSPEN reset set like hardware, but
/// exception entry stacks the 8-word integer frame only.
pub struct Fpu {
    fpccr: u32,  // +0x0 (reset ASPEN|LSPEN)
    fpcar: u32,  // +0x4
    fpdscr: u32, // +0x8 (default FPSCR; stored only in v1)
}

/// Cortex-M4F (VFPv4-SP, 32xS + 16xD alias) ID values (M4F TRM; pinned by
/// cpu test `fpu_mvfr_and_cpacr_reset`).
pub const MVFR0: u32 = 0x1011_0021;
pub const MVFR1: u32 = 0x1100_0011;
pub const MVFR2: u32 = 0x0000_0040;

impl Default for Fpu {
    fn default() -> Self {
        Self { fpccr: 0xC000_0000, fpcar: 0, fpdscr: 0 }
    }
}

impl Fpu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FPU" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Fpu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0 => self.fpccr,
            0x4 => self.fpcar,
            0x8 => self.fpdscr,
            0xC => MVFR0,
            0x10 => MVFR1,
            0x14 => MVFR2,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            // ASPEN|LSPEN + THREAD|USER|LSPACT only.
            0x0 => self.fpccr = value & 0xC000_0007,
            0x4 => self.fpcar = value & !7,
            // AHP|DN|FZ|RMode only.
            0x8 => self.fpdscr = value & 0x07C0_0000,
            // MVFR is read-only.
            _ => {}
        }
    }
}
