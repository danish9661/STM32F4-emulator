use crate::system::System;
use super::Peripheral;

/// MPU (SVD `MPU` peripheral at 0xE000ED90). Region programming is stored
/// faithfully (TYPE/RNR/RBAR/RASR read back what a bring-up sequence
/// writes) and protection IS enforced: CTRL.ENABLE latches the model
/// sticky (`set_mpu_enabled`); the CPU gates every access through
/// `check_range` (privilege from the CURRENT_PRIV/HFNMI context, XN on
/// fetch, AP-field permissions, highest-numbered-region wins, subregions
/// for sizes >= 256B). Violations raise MemManage (or escalate to
/// HardFault when SHCSR.MEMFAULTENA is clear) with exact MMFSR/MMFAR.
/// No-region background is priv-only iff PRIVDEFENA (approximation: the
/// real default map has per-address XN; documented at the call site).
pub struct Mpu {
    ctrl: u32,          // +0x4 (ENABLE/HFNMIENA/PRIVDEFENA only)
    rnr: u32,           // +0x8
    rbar: [u32; 8],     // +0xC (per selected region)
    rasr: [u32; 8],     // +0x10
}

impl Default for Mpu {
    fn default() -> Self {
        Self { ctrl: 0, rnr: 0, rbar: [0; 8], rasr: [0; 8] }
    }
}

impl Mpu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "MPU" { Some(Box::new(Self::default())) } else { None }
    }

    fn region(&self) -> usize {
        (self.rnr & 7) as usize
    }

    /// Whether region `idx` covers byte `addr` (RASR.ENABLE, power-of-2
    /// size >= 32B, RBAR-aligned base, SRD subregions for sizes >= 256B).
    /// Undersized regions and nonzero-SRD-on-small are UNPREDICTABLE on
    /// silicon; both read as never-matching here (deny-closed).
    fn region_covers(&self, idx: usize, addr: u32) -> bool {
        let rasr = self.rasr[idx];
        if rasr & 1 == 0 {
            return false;
        }
        let sizef = ((rasr >> 1) & 0x1F) as u64;
        if sizef < 4 {
            return false;
        }
        let size = 1u64 << (sizef + 1);
        let base = (self.rbar[idx] & 0xFFFF_FFE0) as u64 & !(size - 1);
        let a = addr as u64;
        if a < base || a >= base + size {
            return false;
        }
        if size >= 256 {
            let sub = size / 8;
            if ((rasr >> 8) & 0xFF) >> ((a - base) / sub) & 1 != 0 {
                return false;
            }
        }
        true
    }

    fn highest_match(&self, addr: u32) -> Option<usize> {
        for i in (0..8).rev() {
            if self.region_covers(i, addr) {
                return Some(i);
            }
        }
        None
    }

    /// Device-memory attribute of the highest matching region, from RASR
    /// TEX/S/C/B: shareable Device (TEX=0,C=0,B=1) or Device (TEX=2).
    /// The only observable use of memory types in an interpreter is the
    /// unaligned-Device rule (no caches exist to do anything else with
    /// them); everything else stays type-blind by design.
    fn region_is_device(&self, idx: usize) -> bool {
        let rasr = self.rasr[idx];
        let tex = (rasr >> 19) & 7;
        let c = (rasr >> 17) & 1;
        let b = (rasr >> 16) & 1;
        (tex == 0 && c == 0 && b == 1) || tex == 2
    }

    /// Whether `addr` is Device memory under the highest matching region
    /// (false with the MPU off or unmatched — background Normal for the
    /// memory paths that consult this).
    pub fn is_device(&self, addr: u32) -> bool {
        match self.highest_match(addr) {
            Some(i) => self.region_is_device(i),
            None => false,
        }
    }

    /// AP-field permission: (read/write x priv) per ARMv7-M. 0b100
    /// (UNPREDICTABLE) and 0b111 (reserved) deny everything.
    fn ap_allows(ap: u32, write: bool, priv_: bool) -> bool {
        match ap {
            0 | 4 | 7 => false,
            1 => priv_,
            2 => !write || priv_,
            3 => true,
            5 => !write && priv_,
            _ => !write, // 6: read-only both
        }
    }

    /// Check one byte. Returns false when access is denied.
    /// Fetch additionally requires XN clear (execute needs readability).
    fn byte_allowed(&self, addr: u32, write: bool, exec: bool, priv_: bool) -> bool {
        match self.highest_match(addr) {
            Some(i) => {
                let rasr = self.rasr[i];
                let ap = (rasr >> 24) & 7;
                if exec {
                    (rasr >> 28) & 1 == 0 && Self::ap_allows(ap, false, priv_)
                } else {
                    Self::ap_allows(ap, write, priv_)
                }
            }
            None => {
                // No region: privileged background iff PRIVDEFENA.
                // (Approximation: the real default map has per-address XN;
                // here background is priv-all/unpriv-none. Documented.)
                priv_ && self.ctrl & 0x4 != 0
            }
        }
    }

    /// Check range [addr, addr+size). Returns Some(true) for an execute
    /// violation, Some(false) for data, None when fully allowed. HFNMI
    /// bypass (active HardFault/NMI with HFNMIENA=0) allows everything.
    pub fn check_range(
        &self,
        addr: u32,
        size: u32,
        write: bool,
        exec: bool,
        priv_: bool,
        hfnmi: bool,
    ) -> Option<bool> {
        if hfnmi && self.ctrl & 0x2 == 0 {
            return None;
        }
        for i in 0..size {
            if !self.byte_allowed(addr.wrapping_add(i), write, exec, priv_) {
                return Some(exec);
            }
        }
        None
    }
}

impl Peripheral for Mpu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            // TYPE: unified map, 8 data + 8 instruction regions (M4F).
            0x0 => 0x0008_0800,
            0x4 => self.ctrl,
            0x8 => self.rnr & 7,
            0xC => self.rbar[self.region()],
            0x10 => self.rasr[self.region()],
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x4 => {
                self.ctrl = value & 0x7;
                crate::system::set_mpu_enabled(value & 1 != 0);
            }
            0x8 => self.rnr = value & 7,
            0xC => {
                let r = self.region();
                self.rbar[r] = value;
            }
            0x10 => {
                let r = self.region();
                self.rasr[r] = value;
            }
            _ => {}
        }
    }
}
