use crate::system::System;
use super::Peripheral;

pub struct Pwr {
    cr: u32,
    csr: u32,
    /// Regulator/overdrive state machine (F429 carry-over; the F407 SVD has
    /// no ODEN/ODSWEN/UDEN/VOS fields, but guests written against the family
    /// header still poke them — model the handshake so they observe silicon
    /// behavior instead of stuck bits):
    /// - VOSRDY (CSR bit 14): set after a voltage-scaling settle delay
    ///   (fixed instruction-count window after a VOS write), clear while
    ///   settling. F407 guests never write VOS (no field), so they always
    ///   see ready — same as before.
    /// - Overdrive (ODEN/ODSWEN/UDEN at CR bits 16/17/18, F429 layout):
    ///   ODEN starts a settle window, then ODRDY (CSR 16) sets; ODSWEN
    ///   starts a second window, then ODSWRDY (CSR 17) sets; UDEN trims to
    ///   under-drive (UDRDY CSR 18 follows the same window). Windows are
    ///   instruction-count based, like every other ready delay in the model.
    vos_settle_until: u64,
    od_settle_until: u64,
    odsw_settle_until: u64,
    ud_settle_until: u64,
    overdrive_en: bool,
    overdrive_sw_en: bool,
    underdrive_en: bool,
}
// PVD/VOSRDY model note: PLS[7:5] is observable — PVDO (CSR bit 2) follows
// it against a fixed emulated supply. The emulator has no analog rail, so
// the supply is pinned above every threshold: with PVDE set, PVDO reads 0
// (supply above threshold — the normal powered-board case); with PVDE
// clear, PVDO reads 1 (detection off, output forced high — exactly what
// RM0090 documents for PVDE=0). VOSRDY (CSR bit 14) reads 1 once the
// settle window after a VOS write elapses (see struct fields).

impl Default for Pwr {
    fn default() -> Self {
        Self {
            cr: 0x0000_0000,
            csr: 0x0000_0000,
            vos_settle_until: 0,
            od_settle_until: 0,
            odsw_settle_until: 0,
            ud_settle_until: 0,
            overdrive_en: false,
            overdrive_sw_en: false,
            underdrive_en: false,
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
            // CSR: live PVD/VOSRDY/overdrive bits ORed over the latched
            // WUF/SBF/EWUP state. PVDO (bit 2): 0 while PVDE watches a
            // healthy rail, 1 when detection is off. VOSRDY (bit 14): 1
            // once the post-VOS-write settle window elapses (F407 guests
            // never write VOS, so always 1 for them). ODRDY/ODSWRDY/UDRDY
            // (bits 16/17/18-19): 1 once their enable's settle window
            // elapses (F429 overdrive/under-drive handshake).
            0x04 => {
                use crate::system::instruction_count;
                let now = instruction_count();
                let mut v = self.csr;
                if self.cr & (1 << 4) != 0 { v &= !(1 << 2); } else { v |= 1 << 2; }
                if now >= self.vos_settle_until { v |= 1 << 14; } else { v &= !(1 << 14); }
                if self.overdrive_en && now >= self.od_settle_until { v |= 1 << 16; } else { v &= !(1 << 16); }
                if self.overdrive_sw_en && now >= self.odsw_settle_until { v |= 1 << 17; } else { v &= !(1 << 17); }
                if self.underdrive_en && now >= self.ud_settle_until { v |= 3 << 18; } else { v &= !(3 << 18); }
                v
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                use crate::system::instruction_count;
                self.clear_flags(value);
                // F407 mask (low 13 bits) PLUS the F429 family fields
                // (VOS[15:14], ODEN/ODSWEN/UDEN[18:16]) so family-header
                // guests observe the handshake instead of stuck bits.
                // VOS write re-arms the VOSRDY settle window.
                let old_vos = (self.cr >> 14) & 3;
                self.cr = (self.cr & 0xE000) | (value & 0x1FFF)
                    | (value & (0x3 << 14)) | (value & (0x7 << 16));
                if ((value >> 14) & 3) != old_vos {
                    self.vos_settle_until =
                        instruction_count().wrapping_add(1_000);
                }
                // Overdrive handshake edges (F429 §5.3): ODEN starts the
                // ODRDY window; ODSWEN (only after ODRDY) starts the
                // ODSWRDY window; UDEN (2-bit field, any nonzero) starts
                // the UDRDY window. Clearing an enable drops its ready at
                // once (silicon clears ready when the enable drops).
                let now = instruction_count();
                let oden = value & (1 << 16) != 0;
                let odswen = value & (1 << 17) != 0;
                let uden = (value >> 18) & 3 != 0;
                if oden && !self.overdrive_en {
                    self.od_settle_until = now.wrapping_add(5_000);
                }
                if odswen && !self.overdrive_sw_en {
                    self.odsw_settle_until = now.wrapping_add(5_000);
                }
                if uden && !self.underdrive_en {
                    self.ud_settle_until = now.wrapping_add(5_000);
                }
                self.overdrive_en = oden;
                self.overdrive_sw_en = odswen;
                self.underdrive_en = uden;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::Peripheral;

    #[test]
    fn vos_settle_and_overdrive_handshake() {
        use std::sync::atomic::Ordering;
        let sys = crate::system::test_dummy_system();
        let mut boxed = Pwr::new("PWR").unwrap();
        let p = boxed.as_any_mut().downcast_mut::<Pwr>().unwrap();
        // F407 path unchanged: VOSRDY reads 1 with no VOS write ever.
        assert_ne!(p.read(&sys, 0x04) & (1 << 14), 0, "VOSRDY settled at reset");
        // VOS write drops VOSRDY for the settle window, then it returns.
        p.write(&sys, 0x00, 1 << 14);
        assert_eq!(p.read(&sys, 0x04) & (1 << 14), 0, "VOSRDY clears during settle");
        crate::system::INSTRUCTION_COUNT.fetch_add(2_000, Ordering::Relaxed);
        assert_ne!(p.read(&sys, 0x04) & (1 << 14), 0, "VOSRDY returns after window");
        // Overdrive handshake: ODEN -> ODRDY after its window (not before).
        p.write(&sys, 0x00, (1 << 16) | (1 << 8)); // ODEN + DBP (keep PWR clocked look)
        assert_eq!(p.read(&sys, 0x04) & (1 << 16), 0, "ODRDY not yet");
        crate::system::INSTRUCTION_COUNT.fetch_add(6_000, Ordering::Relaxed);
        assert_ne!(p.read(&sys, 0x04) & (1 << 16), 0, "ODRDY after settle");
        // Clearing ODEN drops ODRDY at once.
        p.write(&sys, 0x00, 1 << 8);
        assert_eq!(p.read(&sys, 0x04) & (1 << 16), 0, "ODRDY drops with ODEN");
        // Under-drive: UDEN field nonzero -> UDRDY pair after window.
        p.write(&sys, 0x00, (1 << 18) | (1 << 8));
        crate::system::INSTRUCTION_COUNT.fetch_add(6_000, Ordering::Relaxed);
        assert_ne!(p.read(&sys, 0x04) & (3 << 18), 0, "UDRDY after settle");
    }
}
