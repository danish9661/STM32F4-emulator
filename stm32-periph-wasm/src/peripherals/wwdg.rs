use std::sync::atomic::Ordering;
use crate::system::{System, INSTRUCTION_COUNT, request_watchdog_reset};
use super::Peripheral;

pub struct Wwdg {
    cr: u32,
    cfr: u32,
    sr: u32,
    last_tick: u64,
    initialized: bool,
}

impl Wwdg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "WWDG" {
            Some(Box::new(Wwdg { cr: 0x7F, cfr: 0x7F, sr: 0, last_tick: 0, initialized: false }))
        } else { None }
    }

    fn wdga_enabled(&self) -> bool { self.cr & 0x80 != 0 }
    fn prescaler(&self) -> u32 {
        match (self.cfr >> 7) & 3 { 0 => 1, 1 => 2, 2 => 4, 3 => 8, _ => 1 }
    }
    fn tick_instructions(&self) -> u64 { 256 * self.prescaler() as u64 }

    fn early_wakeup(&mut self, sys: &System) {
        self.sr |= 1;
        if self.cfr & 0x200 != 0 { sys.p.nvic.borrow_mut().set_intr_pending(0); }
    }

    /// Continuous countdown driven by the virtual clock (PCLK1/4096/prescaler),
    /// independent of CPU accesses to the WWDG registers.
    fn tick_counter(&mut self, sys: &System) {
        if !self.wdga_enabled() { return; }
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        if !self.initialized { self.last_tick = now; self.initialized = true; return; }
        let elapsed = now.saturating_sub(self.last_tick);
        let ticks = elapsed / self.tick_instructions();
        if ticks == 0 { return; }
        self.last_tick = now;
        let counter = self.cr & 0x7F;
        if counter < ticks as u32 {
            self.cr = 0x7F; // clear WDGA + counter; re-enabled only when firmware re-sets it
            self.initialized = false;
            self.early_wakeup(sys);
            request_watchdog_reset(2);
        } else {
            let new_counter = counter - ticks as u32;
            self.cr = (self.cr & !0x7F) | new_counter;
            if new_counter <= 0x3F && counter > 0x3F { self.early_wakeup(sys); }
        }
    }
}

impl Peripheral for Wwdg {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn tick(&mut self, sys: &System) { self.tick_counter(sys); }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset { 0x00 => self.cr, 0x04 => self.cfr, 0x08 => self.sr, _ => 0 }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                if value & 0x80 != 0 {
                    // Window violation: a refresh while the live counter is
                    // still above the window (counter > W) resets the MCU.
                    // The initial enable loads the counter and must not trip
                    // this; only subsequent refreshes are gated.
                    if self.initialized && (self.cr & 0x7F) > (self.cfr & 0x7F) {
                        request_watchdog_reset(2);
                    }
                }
                self.cr = value & 0xFF;
                self.last_tick = INSTRUCTION_COUNT.load(Ordering::Relaxed);
                self.initialized = true;
            }
            0x04 => self.cfr = value & 0x7FFF,
            0x08 => self.sr &= !(value & 1),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{
        clear_watchdog_reset_flags, test_dummy_system, wwdg_reset_flag, INSTRUCTION_COUNT,
    };
    use std::sync::atomic::Ordering;

    #[test]
    fn window_violation_reset() {
        clear_watchdog_reset_flags();
        let mut boxed = Wwdg::new("WWDG").unwrap();
        let w = boxed.as_any_mut().downcast_mut::<Wwdg>().unwrap();
        let sys = test_dummy_system();
        // WDGTB=3, window W=0x50 (early-window restriction active).
        w.write(&sys, 0x04, (3u32 << 7) | 0x50);
        // Enable + reload counter to 0x7F.
        w.write(&sys, 0x00, 0x80 | 0x7F);
        // Refresh again immediately while counter (0x7F) is still above the
        // window (0x50) -> window violation -> reset requested.
        w.write(&sys, 0x00, 0x80 | 0x7F);
        assert!(wwdg_reset_flag(), "window violation must request a WWDG reset");
    }

    #[test]
    fn ewi_fires_at_window() {
        clear_watchdog_reset_flags();
        let mut boxed = Wwdg::new("WWDG").unwrap();
        let w = boxed.as_any_mut().downcast_mut::<Wwdg>().unwrap();
        let sys = test_dummy_system();
        // WDGTB=3, W=0x50, EWI enabled (CFR bit 9).
        w.write(&sys, 0x04, (3u32 << 7) | 0x50 | (1u32 << 9));
        INSTRUCTION_COUNT.store(1_000_000, Ordering::Relaxed);
        // Enable + reload counter to 0x7F; last_tick anchored at 1_000_000.
        w.write(&sys, 0x00, 0x80 | 0x7F);
        // Advance the clock far enough that the counter drops to <= 0x3F,
        // crossing the window edge and firing the early-wakeup interrupt.
        INSTRUCTION_COUNT.fetch_add(200_000, Ordering::Relaxed);
        w.tick(&sys);
        assert!(w.read(&sys, 0x08) & 1 != 0, "EWIF should be set at the window edge");
        assert!(
            sys.p.nvic.borrow().irq_pending(0),
            "WWDG early-wakeup interrupt (IRQ 0) should be pending"
        );
    }
}
