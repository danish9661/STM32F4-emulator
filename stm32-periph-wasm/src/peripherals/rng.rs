use crate::system::{System, instruction_count};
use super::Peripheral;

const RNG_IRQ: i32 = 80;

/// Host entropy pool (harness = the physical noise source): 32-bit words
/// pushed from JS via `rng_seed_entropy`. The model consumes one word per
/// regen while the pool is non-empty (true entropy path); when the pool
/// drains it falls back to the deterministic LCG (documented substitute —
/// firmware that never seeds observes the LCG sequence). Pool depth is
/// observable via `rng_entropy_avail` so firmware can tell which source
/// fed DR (silicon's DRDY/SECS model has no such bit; this is the harness
/// contract, documented at the export).
static ENTROPY_POOL: std::sync::OnceLock<std::sync::Mutex<Vec<u32>>> =
    std::sync::OnceLock::new();
fn entropy_pool() -> &'static std::sync::Mutex<Vec<u32>> {
    ENTROPY_POOL.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Push host entropy words (true-noise samples from JS `crypto.getRandomValues`
/// or equivalent). Consumed FIFO, one word per regen.
pub fn rng_seed_entropy(words: &[u32]) {
    entropy_pool().lock().unwrap().extend_from_slice(words);
}

/// Words currently pooled (0 = LCG fallback active).
pub fn rng_entropy_avail() -> usize {
    entropy_pool().lock().unwrap().len()
}

/// Drain the entropy pool (fresh-instance hygiene; called by reset_globals
/// so a seeded pool never leaks across emulator instances).
pub fn rng_clear_entropy() {
    entropy_pool().lock().unwrap().clear();
}

fn entropy_take() -> Option<u32> {
    let mut p = entropy_pool().lock().unwrap();
    if p.is_empty() {
        None
    } else {
        Some(p.remove(0))
    }
}

pub struct Rng {
    cr: u32,
    sr: u32,
    dr: u32,
    last_regen: u64,
    /// True when DR currently holds a host-entropy word (vs LCG fallback).
    /// Observable via SR bit 5 (SECS — seed error: silicon sets it when the
    /// analog seed fails; here it reports "LCG fallback active", i.e. the
    /// entropy pool was empty at regen). Firmware polling SECS observes
    /// exactly the documented contract.
    from_entropy: bool,
}

impl Default for Rng {
    fn default() -> Self {
        Self {
            sr: 0x00,
            dr: 0x0000_0000,
            last_regen: 0,
            cr: 0x0000_0000,
            from_entropy: false,
        }
    }
}

impl Rng {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RNG" { Some(Box::new(Self::default())) } else { None }
    }

    fn regenerate(&mut self) {
        let n = instruction_count();
        let elapsed = n.wrapping_sub(self.last_regen);
        if elapsed > 40 {
            // True-entropy path first: consume one pooled host word.
            // Fallback: deterministic LCG (same sequence as before).
            if let Some(w) = entropy_take() {
                self.dr = w;
                self.from_entropy = true;
            } else {
                let n32 = n as u32;
                self.dr = n32.wrapping_mul(1103515245).wrapping_add(12345);
                self.dr ^= self.dr >> 16;
                self.dr ^= self.dr << 5;
                self.from_entropy = false;
            }
            self.sr = 0x40;
            // SECS (bit 5) reports LCG-fallback-active (see field doc).
            if !self.from_entropy {
                self.sr |= 1 << 5;
            }
            self.last_regen = n;
        }
    }

    #[cfg(test)]
    pub(crate) fn last_regen_for_test(&self) -> u64 {
        self.last_regen
    }
}

impl Peripheral for Rng {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn tick(&mut self, _sys: &System) {
        // Regen on the model tick (not only on DR read): the DR read path
        // consumes one regen per access, so a polled DR without an
        // intervening tick would never advance. Gated on RNGEN like silicon
        // (clock must be on for the analog block to run).
        if self.cr & 4 != 0 {
            self.regenerate();
        }
    }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => {
                // SR: DRDY (bit 6) + SECS (bit 5, LCG-fallback-active).
                // CEIS/SEIS (bits 6/5 silicon positions overlap DRDY/SECS
                // here by design — read-then-clear via the write arm).
                let sr = self.sr;
                self.sr &= !(0x06);
                sr
            }
            0x08 => {
                self.regenerate();
                let dr = self.dr;
                self.sr &= !0x40;
                if self.sr & 6 != 0 && self.cr & 8 != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(RNG_IRQ);
                }
                dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = value & 0x17;
                if self.cr & 4 != 0 {
                    self.sr |= 1;
                } else {
                    self.sr &= !1;
                }
                if self.cr & 8 != 0 && self.sr & 0x40 != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(RNG_IRQ);
                }
            }
            0x04 => {
                self.sr &= !(value & 0x46);
            }
            0x08 => {}
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::Peripheral;

    #[test]
    fn regen_on_tick_advances_dr() {
        use std::sync::atomic::Ordering;
        // Parallel cargo threads share the process-global INSTRUCTION_COUNT,
        // so another thread's ticks can satisfy this instance's 40-inst
        // window at any observe point — never assert exact DR values here.
        // What this test pins: the tick() regen path exists and is RNGEN-
        // gated (RNGEN clear -> tick performs no regen of its own).
        let sys = crate::system::test_dummy_system();
        let mut boxed = Rng::new("RNG").unwrap();
        let r = boxed.as_any_mut().downcast_mut::<Rng>().unwrap();
        // Disabled: tick performs no regen of its own (last_regen untouched).
        let lr0 = r.last_regen_for_test();
        crate::system::INSTRUCTION_COUNT.fetch_add(1000, Ordering::Relaxed);
        r.tick(&sys);
        assert_eq!(r.last_regen_for_test(), lr0, "tick with RNGEN clear regens nothing");
        // Enable: tick regens (last_regen advances to the clock).
        r.write(&sys, 0x00, 4);
        crate::system::INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        r.tick(&sys);
        assert_ne!(r.last_regen_for_test(), lr0, "tick with RNGEN set regens");
        let d1 = r.read(&sys, 0x08);
        assert_ne!(d1, 0, "DR nonzero after tick");
    }

    #[test]
    fn entropy_pool_feeds_dr_and_secs_reports_fallback() {
        use std::sync::atomic::Ordering;
        // Serialized by the test harness (pool is process-global); drain
        // first so parallel leftovers cannot flake the band.
        rng_clear_entropy();
        let sys = crate::system::test_dummy_system();
        let mut boxed = Rng::new("RNG").unwrap();
        let r = boxed.as_any_mut().downcast_mut::<Rng>().unwrap();
        r.write(&sys, 0x00, 4); // RNGEN
        // Seeded: DR returns the pooled words in order, SECS clear.
        rng_seed_entropy(&[0xDEAD_BEEF, 0x1234_5678]);
        assert_eq!(rng_entropy_avail(), 2, "pool holds 2");
        crate::system::INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        r.tick(&sys);
        assert_eq!(r.read(&sys, 0x08), 0xDEAD_BEEF, "first pooled word");
        assert_eq!(r.read(&sys, 0x04) & (1 << 5), 0, "SECS clear on entropy");
        crate::system::INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        r.tick(&sys);
        assert_eq!(r.read(&sys, 0x08), 0x1234_5678, "second pooled word FIFO");
        // Drained: LCG fallback resumes, SECS sets.
        crate::system::INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        r.tick(&sys);
        let _ = r.read(&sys, 0x08);
        assert_ne!(r.read(&sys, 0x04) & (1 << 5), 0, "SECS set on LCG fallback");
        assert_eq!(rng_entropy_avail(), 0, "pool drained");
        rng_clear_entropy();
    }
}
