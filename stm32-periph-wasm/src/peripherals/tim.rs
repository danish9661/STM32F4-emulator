use crate::system::{System, instruction_count};
use super::Peripheral;

fn tim_irq(name: &str) -> Option<i32> {
    match name {
        "TIM1" => Some(24), "TIM2" => Some(28), "TIM3" => Some(29),
        "TIM4" => Some(30), "TIM5" => Some(50), "TIM6" => Some(54),
        "TIM7" => Some(55), "TIM8" => Some(70), "TIM9" => Some(20),
        "TIM10" => Some(25), "TIM11" => Some(26), "TIM12" => Some(43),
        "TIM13" => Some(54), "TIM14" => Some(51),
        _ => None,
    }
}

/// Counter width. On the STM32F407 only TIM2 and TIM5 are 32-bit
/// (RM0090 §17 "TIM2 to TIM5" — TIM2/TIM5 have 32-bit counters, TIM3/TIM4
/// are 16-bit); every other timer is 16-bit.  This governs CNT, ARR and the
/// capture/compare registers together — they are all the same width as the
/// counter, so masking them differently is always a bug.
fn counter_mask(name: &str) -> u32 {
    match name {
        "TIM2" | "TIM5" => 0xFFFF_FFFF,
        _ => 0xFFFF,
    }
}

pub struct Timer {
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    sr: u32,
    egr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    cnt: u32,
    psc: u32,
    arr: u32,
    ccr: [u32; 4],
    rcr: u32,
    dcr: u32,
    dmar: u32,
    or_: u32,
    // Extended
    ccmr3: u32,
    ccr5: u32,
    ccr6: u32,
    pwm_duty: [u32; 4],
    last_tick: u64,
    irq_num: i32,
    name: String,
    one_pulse_active: bool,
}

impl Timer {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        tim_irq(name).map(|irq| {
            Box::new(Self {
                cr1: 0, cr2: 0, smcr: 0, dier: 0, sr: 0, egr: 0,
                ccmr1: 0, ccmr2: 0, ccer: 0, cnt: 0, psc: 0,
                // Free-running by default, but only as wide as the counter
                // actually is (0xFFFF on the 16-bit timers).
                arr: counter_mask(name),
                ccr: [0; 4], rcr: 0, dcr: 0, dmar: 0, or_: 0,
                ccmr3: 0, ccr5: 0, ccr6: 0, pwm_duty: [0; 4],
                last_tick: instruction_count(),
                irq_num: irq,
                name: name.to_string(),
                one_pulse_active: false,
            }) as Box<dyn Peripheral>
        })
    }

    fn prescaler(&self) -> u64 {
        (self.psc as u64).max(1)
    }

    fn elapsed_ticks(&self) -> u64 {
        let now = instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        delta / self.prescaler()
    }

    fn advance(&mut self, sys: &System) {
        let ticks = self.elapsed_ticks();
        if ticks == 0 { return; }
        self.last_tick = instruction_count();

        let enabled = self.cr1 & 1;
        if enabled == 0 { return; }

        let dir = (self.cr1 >> 4) & 1;
        let cms = (self.cr1 >> 5) & 0x3;

        for _ in 0..ticks.min(100) {
            match (cms, dir) {
                (0, 0) => { // Up-counting
                    // Counts 0..=ARR then reloads, so the period is ARR+1
                    // ticks (RM0090 §18.3.1).  This was `self.arr - 1`, which
                    // (a) made the period ARR and never let CNT reach ARR — so
                    // a compare at CCR==ARR could never match — and (b)
                    // underflowed when ARR==0 (a legal value): panic in debug,
                    // wrap to 0xFFFFFFFF in release, turning a
                    // fire-every-tick timer into a free-running one.
                    if self.cnt < self.arr { self.cnt += 1; }
                    else {
                        self.cnt = 0;
                        self.sr |= 1; // UIF
                        if self.dier & 1 != 0 { // UIE
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                        if self.dier & (1 << 8) != 0 { //UDE - DMA request
                            // would trigger DMA
                        }
                        // Update interrupt on overflow
                    }
                }
                (0, 1) => { // Down-counting
                    if self.cnt > 0 { self.cnt -= 1; }
                    else {
                        self.cnt = self.arr;
                        self.sr |= 1; // UIF
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
                _ => { // Center-aligned modes
                    // Simplified: just up-count (same ARR/ARR-1 fix as above)
                    if self.cnt < self.arr { self.cnt += 1; }
                    else {
                        self.cnt = 0;
                        self.sr |= 1;
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
            }

            // Output compare / PWM interrupts
            for ch in 0..4 {
                if self.ccer & (1 << (ch * 4)) != 0 { // CCxE
                    let ccr_val = self.ccr[ch];
                    if self.cnt == ccr_val {
                        // Capture/Compare match
                        self.sr |= 1 << (1 + ch); // CC1IF-CC4IF
                        let cc_irq_enable = (self.dier >> (1 + ch)) & 1;
                        if cc_irq_enable != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                    }
                }
            }
        }

        // Update PWM duty based on CCR/ARR
        for ch in 0..4 {
            if self.ccer & (1 << (ch * 4)) != 0 && self.arr > 0 {
                self.pwm_duty[ch] = self.ccr[ch] * 100 / (self.arr + 1);
            }
        }

        self.update_interrupt(sys);
    }

    fn update_interrupt(&self, sys: &System) {
        // UIF, CCxIF, TIF, etc. already trigger during advance
    }

    fn generate_update(&mut self, sys: &System) {
        self.cnt = 0;
        self.sr |= 1; // UIF
        if self.dier & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }
}

impl Peripheral for Timer {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn tick(&mut self, sys: &System) {
        self.advance(sys);
    }

    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        if offset == 0x24 { return self.cnt; }
        self.advance(sys);
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.smcr,
            0x0C => self.dier,
            0x10 => self.sr,
            0x14 => {
                // EGR reads as 0
                self.egr
            }
            0x18 => self.ccmr1,
            0x1C => self.ccmr2,
            0x20 => self.ccer,
            0x24 => self.cnt,
            0x28 => self.psc,
            0x2C => self.arr,
            0x30 => self.rcr,
            0x34..=0x40 => {
                let i = ((offset - 0x34) / 4) as usize;
                self.ccr.get(i).copied().unwrap_or(0)
            }
            0x48 => self.dcr,
            0x4C => self.dmar,
            0x50 => self.or_,
            0x54 => self.ccmr3,
            0x58 => self.ccr5,
            0x5C => self.ccr6,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.advance(sys);
        match offset {
            0x00 => {
                let was_enabled = self.cr1 & 1;
                self.cr1 = value & 0xFFFE_F17F;
                if self.cr1 & 1 != 0 && was_enabled == 0 {
                    // Enable: reset counter to 0
                    self.cnt = 0;
                }
            }
            0x04 => self.cr2 = value & 0x3F7F,
            0x08 => self.smcr = value & 0xFFFF,
            0x0C => {
                self.dier = value & 0xFFFF;
                self.update_interrupt(sys);
            }
            0x10 => self.sr &= value,
            0x14 => {
                self.egr = value & 0xFF;
                if value & 1 != 0 { self.generate_update(sys); } // UG
            }
            0x18 => self.ccmr1 = value,
            0x1C => self.ccmr2 = value,
            0x20 => self.ccer = value & 0xFFFF,
            // CNT/ARR/CCRx are all the counter's width: 32-bit on TIM2/TIM5,
            // 16-bit everywhere else.  This used to mask CNT and CCRx to
            // 16 bits while letting ARR take a full 32 — a combination that
            // matches NO real timer: TIM2/TIM5 (32-bit, used for long or
            // high-resolution timing) had their counter truncated at 0xFFFF,
            // and the 16-bit timers accepted an out-of-range ARR.
            0x24 => self.cnt = value & counter_mask(&self.name),
            0x28 => self.psc = value & 0xFFFF,
            0x2C => self.arr = value & counter_mask(&self.name),
            0x30 => self.rcr = value & 0xFF,
            0x34..=0x40 => {
                let mask = counter_mask(&self.name);
                let i = ((offset - 0x34) / 4) as usize;
                if let Some(ccr) = self.ccr.get_mut(i) {
                    *ccr = value & mask;
                }
            }
            0x48 => self.dcr = value & 0x1F1F,
            0x4C => self.dmar = value,
            0x50 => self.or_ = value & 0xFF,
            0x54 => self.ccmr3 = value,
            0x58 => self.ccr5 = value & 0xFFFF,
            0x5C => self.ccr6 = value & 0xFFFF,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Regression: CNT/ARR/CCR must follow the counter width. The model used
    // to mask CNT+CCR to 16 bits while ARR took 32 — no real timer behaves
    // that way, and it silently truncated TIM2/TIM5 (the 32-bit timers).
    #[test]
    fn counter_width_follows_the_timer() {
        let sys = crate::system::test_dummy_system();
        for (name, wide) in [("TIM2", true), ("TIM5", true), ("TIM3", false), ("TIM4", false)] {
            let mut boxed = Timer::new(name).unwrap();
            let t = boxed.as_any_mut().downcast_mut::<Timer>().unwrap();
            let expect: u32 = if wide { 0x1234_5678 } else { 0x5678 };
            t.write(&sys, 0x24, 0x1234_5678);            // CNT
            assert_eq!(t.read(&sys, 0x24), expect, "{name} CNT");
            t.write(&sys, 0x2C, 0x1234_5678);            // ARR
            assert_eq!(t.read(&sys, 0x2C), expect, "{name} ARR");
            t.write(&sys, 0x34, 0x1234_5678);            // CCR1
            assert_eq!(t.read(&sys, 0x34), expect, "{name} CCR1");
        }
    }

    // ARR == 0 is legal and means "reload every tick".  The counter used to
    // compute `self.arr - 1`, which underflows there: a debug panic, and in
    // release a wrap to 0xFFFFFFFF that silently turned it free-running.
    #[test]
    fn arr_zero_does_not_underflow() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Timer::new("TIM3").unwrap();
        let t = boxed.as_any_mut().downcast_mut::<Timer>().unwrap();
        t.write(&sys, 0x2C, 0);        // ARR = 0
        t.write(&sys, 0x00, 1);        // CR1: CEN
        crate::system::INSTRUCTION_COUNT.fetch_add(50, std::sync::atomic::Ordering::Relaxed);
        t.tick(&sys);                  // must not panic
        assert_eq!(t.read(&sys, 0x24), 0, "CNT stays 0 when ARR==0");
        assert_ne!(t.read(&sys, 0x10) & 1, 0, "UIF set");
    }
}
