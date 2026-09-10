use crate::system::{System, INSTRUCTION_COUNT};
use std::sync::atomic::Ordering;
use super::Peripheral;

/// DWT cycle counter (0xE0001000 block; CTRL + CYCCNT only). CYCCNT advances
/// with the shared instruction clock (1 count/inst) while CTRL.CYCCNTENA
/// (bit 0) and DEMCR.TRCENA are both set — the silicon gating rule. Reads
/// sync lazily from INSTRUCTION_COUNT, so counting costs nothing on the hot
/// path; parallel-test clock skew is absorbed with saturating_sub (same
/// pattern as the SysTick model). NOCYCCNT (CTRL bit 25) reads 0: the
/// counter is implemented. FOLDCNT/CPICNT/etc. read 0 (unmodeled).
pub struct Dwt {
    ctrl: u32,
    cyccnt: u32,
    last: u64,
    /// Exception-overhead event counter (0x100C): exact take count, gated
    /// on TRCENA like the rest of the unit. FOLD/CPI/SLEEP/LSU are NOT
    /// modeled (fold/branch penalties would be fabricated in an
    /// interpreter; sleep isn't cycle-counted; LSU would tax every
    /// access for a counter nobody reads yet).
    exccnt: u8,
}

impl Dwt {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DWT" {
            Some(Box::new(Self { ctrl: 0, cyccnt: 0, last: 0, exccnt: 0 }))
        } else {
            None
        }
    }

    /// Count one exception entry (called from every take path).
    pub fn count_exc(&mut self) {
        self.exccnt = self.exccnt.wrapping_add(1);
    }

    fn trcena(sys: &System) -> bool {
        sys.p.read(sys, 0xE000EDFC, 4) & (1 << 24) != 0
    }

    /// Bank elapsed counts into cyccnt and rebase `last` on now. Called on
    /// every CTRL/CYCCNT access (the only points the guest can observe).
    fn sync(&mut self, sys: &System) {
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        if self.ctrl & 1 != 0 && Self::trcena(sys) {
            self.cyccnt = self.cyccnt.wrapping_add(now.saturating_sub(self.last) as u32);
        }
        self.last = now;
    }
}

impl Peripheral for Dwt {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x0 => self.ctrl & !(1 << 25),
            0x4 => {
                self.sync(sys);
                self.cyccnt
            }
            0xC => self.exccnt as u32,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            // Sync under the OLD enable first so pre-enable time is never
            // counted and disabling banks the tail.
            0x0 => {
                self.sync(sys);
                self.ctrl = value;
            }
            0x4 => {
                self.cyccnt = value;
                self.last = INSTRUCTION_COUNT.load(Ordering::Relaxed);
            }
            _ => {}
        }
    }
}

/// DEMCR (0xE000EDFC): only TRCENA (bit 24) is modeled — it gates the DWT
/// cycle counter (and, on silicon, the ITM/DWT trace block). Lives in its
/// own 4-byte slot: the SCB slots in both maps end before EDFC, and a
/// shared slot would overlap the MPU/FPU windows (the map asserts).
pub struct Demcr {
    reg: u32,
}

impl Demcr {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DEMCR" {
            Some(Box::new(Self { reg: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Demcr {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, _offset: u32) -> u32 {
        self.reg
    }
    fn write(&mut self, _sys: &System, _offset: u32, value: u32) {
        self.reg = value & (1 << 24);
    }
}
