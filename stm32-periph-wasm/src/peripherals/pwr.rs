use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
}
// PVD/VOSRDY model note: PLS[7:5] is observable — PVDO (CSR bit 2) follows
// it against a fixed emulated supply. The emulator has no analog rail, so
// the supply is pinned above every threshold: with PVDE set, PVDO reads 0
// (supply above threshold — the normal powered-board case); with PVDE
// clear, PVDO reads 1 (detection off, output forced high — exactly what
// RM0090 documents for PVDE=0). VOSRDY (CSR bit 14) reads 1: the regulator
// is always settled here (no voltage-scaling transients are modeled).

impl Default for Pwr {
    fn default() -> Self {
        Self {
            cr: 0x0000_0000,
            csr: 0x0000_0000,
        }
    }
}

impl Pwr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "PWR" { Some(Box::new(Self::default())) } else { None }
    }

    /// Set the wakeup flag (WUF, CSR bit 0). The emulator sets this when
    /// the core wakes from a WFI/WFE low-power state so firmware can
    /// confirm the wakeup source by reading PWR->CSR.
    /// NOTE: WUF is CSR bit 0 on silicon (SVD: PWR_CSR.WUF @0; SBF @1).
    pub fn wakeup(&mut self) {
        self.csr |= 1 << 0;
    }

    /// Enter standby entry bookkeeping (PDDS=1 + SLEEPDEEP WFI path): set
    /// the standby flag (SBF, CSR bit 1) so firmware waking later can tell
    /// STANDBY apart from STOP. Called by the driver when it observes the
    /// guest enter WFI sleep with PDDS set.
    pub fn enter_standby(&mut self) {
        self.csr |= 1 << 1;
    }

    /// Wakeup from standby: set WUF (CSR bit 0). SBF stays set until the
    /// guest clears it via CR CSBF (bit 3) — silicon behavior.
    pub fn wakeup_standby(&mut self) {
        self.csr |= 1 << 0;
    }

    /// Clear WUF (CR CWUF, bit 2) / SBF (CR CSBF, bit 3). Called from the
    /// CR write path below.
    fn clear_flags(&mut self, value: u32) {
        if value & (1 << 2) != 0 {
            self.csr &= !(1 << 0);
        }
        if value & (1 << 3) != 0 {
            self.csr &= !(1 << 1);
        }
    }
}

impl Peripheral for Pwr {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            // CSR: live PVD/VOSRDY bits ORed over the latched WUF/SBF/EWUP
            // state. PVDO (bit 2): 0 while PVDE watches a healthy rail,
            // 1 when detection is off. VOSRDY (bit 14): always 1 (regulator
            // settled — voltage scaling itself is still not modeled).
            0x04 => {
                let mut v = self.csr;
                if self.cr & (1 << 4) != 0 { v &= !(1 << 2); } else { v |= 1 << 2; }
                v |= 1 << 14;
                v
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.clear_flags(value);
                self.cr = (self.cr & 0xE000) | (value & 0x1FFF);
            }
            // CSR bit 0 (WUF) clears via CR CWUF (bit 2); bit 1 (SBF)
            // clears via CR CSBF (bit 3). Direct CSR writes only OR the
            // VOSRDY-adjacent reserved bit the old code allowed (0x100);
            // WUF/SBF themselves are NOT writable (silicon: read-only,
            // cleared via CR).
            0x04 => {
                self.csr |= value & 0x100;
            }
            _ => {}
        }
    }
}
