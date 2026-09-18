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

    /// Master-mode trigger output level (CR2 MMS[6:4]): the TRGO signal this
    /// timer broadcasts on the internal trigger bus. Only the level-sensitive
    /// selections are modeled: 0 = reset (UG bit / counter reset — pulsed on
    /// update events), 1 = enable (CNT running), 2 = update (pulsed on every
    /// update event: overflow/underflow + UG). Compare-pulse / OCxREF
    /// selections (3-7) need edge-precise waveform timing the
    /// instruction-count clock cannot provide honestly — they read back via
    /// CR2 but never assert TRGO (documented, not silent).
    fn trgo_level(&self) -> bool {
        match (self.cr2 >> 4) & 0x7 {
            1 => self.cr1 & 1 != 0, // enable: level while running
            _ => false,             // reset/update are pulsed in advance()
        }
    }

    /// Pulse TRGO on an update event (overflow/underflow/UG) when MMS
    /// selects reset (0) or update (2). Routes to every slave timer whose
    /// SMCR TS points at this master (ITR mapping below), which then applies
    /// its own SMS mode (reset / gated / trigger). Deferred through the
    /// system queue: advance() runs with the master's slot borrow held
    /// (tick() borrows each slot in turn), so routing directly would
    /// double-borrow. The queue drains at the END of the system tick, after
    /// all slots are released (see WasmSystem::tick).
    fn pulse_trgo(&mut self, sys: &System) {
        match (self.cr2 >> 4) & 0x7 {
            0 | 2 => {}
            _ => return,
        }
        sys.queue_trigger(self.name.clone());
    }

    /// ITR mapping: which master drives ITRx of `slave` (RM0090 tables
    /// 40/41/43/45 — TIM1/8 row shown; TIM2-5/6-7 share the same shape).
    /// Returns the master name for TS value 0-3, None for external/ETRF.
    fn itr_master(slave: &str, ts: u32) -> Option<&'static str> {
        match slave {
            "TIM1" => match ts { 0 => Some("TIM5"), 1 => Some("TIM2"), 2 => Some("TIM3"), 3 => Some("TIM4"), _ => None },
            "TIM8" => match ts { 0 => Some("TIM1"), 1 => Some("TIM2"), 2 => Some("TIM4"), 3 => Some("TIM5"), _ => None },
            "TIM2" => match ts { 0 => Some("TIM1"), 1 => Some("TIM2"), 2 => Some("TIM3"), 3 => Some("TIM4"), _ => None },
            "TIM3" => match ts { 0 => Some("TIM1"), 1 => Some("TIM2"), 2 => Some("TIM5"), 3 => Some("TIM4"), _ => None },
            "TIM4" => match ts { 0 => Some("TIM1"), 1 => Some("TIM2"), 2 => Some("TIM3"), 3 => Some("TIM4"), _ => None },
            "TIM5" => match ts { 0 => Some("TIM2"), 1 => Some("TIM3"), 2 => Some("TIM4"), 3 => Some("TIM1"), _ => None },
            _ => None,
        }
    }

    /// Apply one master's TRGO pulse to one slave per the slave's SMS mode:
    /// 0 (disabled) = ignore; 4 (reset) = CNT=0 + UIF; 5 (gated) = enable
    /// counting while TRGO level holds; 6 (trigger) = start counting
    /// (CEN=1). Modes 1-3/7 (encoder/OC/reset-variants) are not modeled.
    /// pub(crate): called by the free route_trgo fan-out (same module).
    pub(crate) fn route_trigger_pub(sys: &System, master: &str, slave: &str) {
        Self::route_trigger(sys, master, slave);
    }

    fn route_trigger(sys: &System, master: &str, slave: &str) {
        for slot in &sys.p.peripherals {
            let mut b = slot.peripheral.borrow_mut();
            let Some(t) = b.as_any_mut().downcast_mut::<Timer>() else { continue };
            if t.name != slave {
                continue;
            }
            let sms = t.smcr & 0x7;
            let ts = (t.smcr >> 4) & 0x7;
            if sms == 0 || ts > 3 {
                return;
            }
            if Self::itr_master(slave, ts) != Some(master) {
                return;
            }
            match sms {
                4 => {
                    // Reset mode: counter reset + update flag (no IRQ unless
                    // UIE set — same path as generate_update).
                    t.cnt = 0;
                    t.sr |= 1;
                    if t.dier & 1 != 0 {
                        sys.p.nvic.borrow_mut().set_intr_pending(t.irq_num);
                    }
                }
                5 => {
                    // Gated mode: run while the master's TRGO level holds.
                    // The pulse call itself proves the level was asserted at
                    // least now; enable counting (CEN=1) so subsequent
                    // advance() ticks count, matching the "run while high"
                    // observable for a pulsed master.
                    t.cr1 |= 1;
                }
                6 => {
                    // Trigger mode: start the counter.
                    if t.cr1 & 1 == 0 {
                        t.cr1 |= 1;
                        t.cnt = 0;
                    }
                }
                _ => {}
            }
            return;
        }
    }
}

/// Fan one master's queued TRGO pulse out to every slaved timer.
/// Called from WasmSystem::tick via drain_triggers, with no slot
/// borrows held. Free function (not a method): the routing helpers live
/// in `impl Timer` taking &mut self borrows, which is exactly what the
/// deferred drain avoids holding.
pub(crate) fn route_trgo(sys: &System, master: &str) {
    for slave in ["TIM1", "TIM2", "TIM3", "TIM4", "TIM5", "TIM6", "TIM7", "TIM8"] {
        if slave != master {
            Timer::route_trigger_pub(sys, master, slave);
        }
    }
}

impl Timer {
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
        // DBGMCU freeze: while a debugger holds the core and this timer's
        // APBx freeze bit is set, the counter neither advances nor fires
        // (RM0090 §38.16). Wall time still passes (last_tick re-anchored
        // above), so no catch-up burst fires on resume.
        if crate::peripherals::dbgmcu::dbgmcu_frozen(sys, &self.name) {
            return;
        }
        // Encoder modes (SMS=001/010/011): the counter advances ONLY on
        // TI1/TI2 pin edges. Harness = the quadrature source: each call to
        // `encoder_step` advances (or retreats) the counter by one step,
        // honoring SMS direction gating (mode 1 = TI1 only, mode 2 = TI2
        // only, mode 3 = both) and the CCER TI1P/TI2P polarity (inverted
        // edge counts down in mode 3; in x1 modes polarity selects which
        // physical edge steps). With no steps the counter holds at
        // whatever the guest wrote (slave-mode reset/trigger routing
        // below still applies; only the free-running time-base is
        // suppressed). (RM0090 §18.3.3.)
        if (self.smcr & 0x7) >= 1 && (self.smcr & 0x7) <= 3 {
            return;
        }

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
                        // OPM (CR1 bit 3): one-pulse — CEN self-clears at
                        // the update event (counter stops until re-armed).
                        if self.cr1 & (1 << 3) != 0 {
                            self.cr1 &= !1;
                        }
                        if self.dier & 1 != 0 { // UIE
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                        if self.dier & (1 << 8) != 0 { //UDE - DMA request
                            // would trigger DMA
                        }
                        // Update event: pulse TRGO to slave timers (MMS
                        // reset/update selections) BEFORE the comment below
                        // so chained slaves observe the same event.
                        self.pulse_trgo(sys);
                        // Update interrupt on overflow
                    }
                }
                (0, 1) => { // Down-counting
                    if self.cnt > 0 { self.cnt -= 1; }
                    else {
                        self.cnt = self.arr;
                        self.sr |= 1; // UIF
                        if self.cr1 & (1 << 3) != 0 {
                            self.cr1 &= !1; // OPM: stop at update
                        }
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                        self.pulse_trgo(sys);
                    }
                }
                _ => { // Center-aligned modes
                    // Simplified: just up-count (same ARR/ARR-1 fix as above)
                    if self.cnt < self.arr { self.cnt += 1; }
                    else {
                        self.cnt = 0;
                        self.sr |= 1;
                        if self.cr1 & (1 << 3) != 0 {
                            self.cr1 &= !1; // OPM: stop at update
                        }
                        if self.dier & 1 != 0 {
                            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
                        }
                        self.pulse_trgo(sys);
                    }
                }
            }

            // Output compare / PWM interrupts (only in output mode; input
            // capture channels latch CNT on an external edge instead).
            for ch in 0..4 {
                if self.ccs(ch) == 0 && self.ccer & (1 << (ch * 4)) != 0 { // CCxE
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

        // Update PWM duty based on CCR/ARR. u64 math: ARR==0xFFFFFFFF
        // (reset default on 32-bit timers, or a transient config window with
        // CC already enabled) would otherwise make (arr+1) wrap to zero and
        // trap, and CCR*100 can overflow u32 for large CCR values. A full-
        // range period yields ~0% duty, which is the honest answer.
        for ch in 0..4 {
            if self.ccer & (1 << (ch * 4)) != 0 && self.arr > 0 {
                self.pwm_duty[ch] = ((self.ccr[ch] as u64 * 100) / ((self.arr as u64) + 1)) as u32;
            }
        }
    }

    fn generate_update(&mut self, sys: &System) {
        self.cnt = 0;
        self.sr |= 1; // UIF
        if self.dier & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
        // UG is an update event too: slaves in reset mode observe it.
        self.pulse_trgo(sys);
    }

    /// CCxS field (input/output selection) for a capture/compare channel.
    /// 0 = output compare; != 0 = input capture (from TIx / TRC).
    fn ccs(&self, ch: usize) -> u32 {
        match ch {
            0 => (self.ccmr1 >> 0) & 0x3,
            1 => (self.ccmr1 >> 8) & 0x3,
            2 => (self.ccmr2 >> 0) & 0x3,
            3 => (self.ccmr2 >> 8) & 0x3,
            _ => 0,
        }
    }

    fn is_input_capture(&self, ch: usize) -> bool {
        self.ccs(ch) != 0
    }

    /// Latch the current counter into CCR[ch] on a (simulated) capture edge.
    /// Mirrors the real TIM: on a match the counter is frozen into the
    /// capture register, CCxIF is set, and CCxOF is set if a previous capture
    /// was not yet serviced.
    fn capture_trigger(&mut self, ch: usize, sys: &System) {
        if ch >= 4 || !self.is_input_capture(ch) {
            return;
        }
        if self.sr & (1 << (1 + ch)) != 0 {
            self.sr |= 1 << (9 + ch); // CCxOF (overrun)
        }
        self.ccr[ch] = self.cnt & counter_mask(&self.name);
        self.sr |= 1 << (1 + ch); // CCxIF
        if (self.dier >> (1 + ch)) & 1 != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    /// Harness = the quadrature encoder: one TI edge step. `ti` selects
    /// the input (0 = TI1, 1 = TI2), `rising` the edge polarity.
    /// - SMS mode 1 (TI1-only): only TI1 steps count; mode 2 (TI2-only):
    ///   only TI2 steps count; mode 3 (both): every step counts.
    /// - Direction: CCER TI1P (bit 1) / TI2P (bit 5) invert the sense —
    ///   a rising edge on an inverted input (or falling on a normal one)
    ///   counts DOWN in mode 3; x1 modes count UP on their selected edge
    ///   (rising when non-inverted, falling when inverted) and ignore the
    ///   other edge. Counter wraps at ARR (up) / 0 (down, reload ARR).
    /// No-op unless the timer is in an encoder SMS mode.
    pub fn encoder_step(&mut self, ti: usize, rising: bool) {
        let sms = (self.smcr & 0x7) as u8;
        if sms < 1 || sms > 3 {
            return;
        }
        if sms == 1 && ti != 0 {
            return;
        }
        if sms == 2 && ti != 1 {
            return;
        }
        let inverted = if ti == 0 {
            self.ccer & (1 << 1) != 0 // TI1P
        } else {
            self.ccer & (1 << 5) != 0 // TI2P
        };
        let up = if sms == 3 {
            // x2/x4: inverted sense flips direction.
            rising != inverted
        } else {
            // x1: only the selected edge steps (up); other edge ignored.
            if rising == inverted {
                return;
            }
            true
        };
        if up {
            if self.cnt < self.arr {
                self.cnt += 1;
            } else {
                self.cnt = 0;
                self.sr |= 1; // UIF on wrap (silicon update event)
            }
        } else if self.cnt > 0 {
            self.cnt -= 1;
        } else {
            self.cnt = self.arr;
            self.sr |= 1;
        }
    }
}

/// Host/JS-driven capture edge: simulate a TIx edge on `name` channel `ch` and
/// latch the live counter into its capture register (only if the channel is
/// configured for input capture). Used by tests that have no external signal
/// source.
pub fn tim_inject_capture(sys: &System, name: &str, ch: u32) {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        let Some(t) = b.as_any_mut().downcast_mut::<Timer>() else { continue };
        if t.name == name {
            t.capture_trigger(ch as usize, sys);
        }
    }
}

/// Host/JS-driven quadrature step: one TI edge (`ti` 0 = TI1, 1 = TI2,
/// `rising` = edge polarity) on the encoder-mode timer `name`. Counts per
/// SMS/polarity rules in `encoder_step`; no-op outside encoder modes.
pub fn tim_encoder_step(sys: &System, name: &str, ti: u32, rising: bool) {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        let Some(t) = b.as_any_mut().downcast_mut::<Timer>() else { continue };
        if t.name == name {
            t.encoder_step(ti as usize, rising);
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
                // OPM (bit 3): when set, CEN self-clears at the next update
                // event (handled in advance()'s overflow arms below).
            }
            0x04 => self.cr2 = value & 0x3F7F,
            0x08 => self.smcr = value & 0xFFFF,
                0x0C => {
                    self.dier = value & 0xFFFF;
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

    // Input capture: a capture edge latches the live counter into CCR, sets
    // CCxIF, and sets CCxOF on a second edge before the first is cleared.
    #[test]
    fn input_capture_latches_counter() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Timer::new("TIM3").unwrap();
        let t = boxed.as_any_mut().downcast_mut::<Timer>().unwrap();
        t.write(&sys, 0x18, 0x01);     // CCMR1: CC1S = 0b01 (input capture TI1)
        assert!(t.is_input_capture(0));
        t.cnt = 1234;
        t.capture_trigger(0, &sys);
        assert_eq!(t.ccr[0], 1234, "capture latches CNT");
        assert!(t.sr & (1 << 1) != 0, "CC1IF set on capture");
        // Second edge without servicing -> overrun flag.
        t.capture_trigger(0, &sys);
        assert!(t.sr & (1 << 9) != 0, "CC1OF set on overrun");
    }

    // Output-compare channels must NOT latch on a capture trigger.
    #[test]
    fn output_compare_ignores_capture_trigger() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Timer::new("TIM3").unwrap();
        let t = boxed.as_any_mut().downcast_mut::<Timer>().unwrap();
        t.write(&sys, 0x18, 0x00);     // CCMR1: CC1S = 0 (output compare)
        assert!(!t.is_input_capture(0));
        t.cnt = 99;
        t.ccr[0] = 0;
        t.capture_trigger(0, &sys);
        assert_eq!(t.ccr[0], 0, "output channel is not captured");
        assert!(t.sr & (1 << 1) == 0, "no CC1IF for output channel");
    }

    // Master/slave trigger routing: TIM2 update (MMS=010) resets TIM3
    // (SMS=100, TS=001 -> ITR1 = TIM2). Uses the shared dummy system so
    // routing crosses real peripheral slots, not one detached Timer.
    #[test]
    fn trgo_update_resets_slaved_timer() {
        use std::sync::atomic::Ordering;
        let sys = crate::system::test_dummy_system();
        // TIM2: master, MMS=010 (update), ARR small, enabled.
        // TIM3: slave, SMS=100 (reset), TS=001 (ITR1 -> TIM2).
        for slot in &sys.p.peripherals {
            let mut b = slot.peripheral.borrow_mut();
            if let Some(t) = b.as_any_mut().downcast_mut::<Timer>() {
                if t.name == "TIM2" {
                    t.write(&sys, 0x04, 2 << 4); // CR2 MMS=010
                    t.write(&sys, 0x2C, 9);      // ARR=9 (period 10)
                    t.write(&sys, 0x00, 1);      // CEN
                } else if t.name == "TIM3" {
                    t.write(&sys, 0x08, (1 << 4) | 4); // SMCR TS=001,SMS=100
                    t.write(&sys, 0x2C, 0xFFFF);
                    t.write(&sys, 0x24, 0x1234); // CNT seeded nonzero
                }
            }
        }
        // Run TIM2 past one full period via the shared system tick (which
        // drains the deferred TRGO queue): its update must reset TIM3 CNT.
        crate::system::INSTRUCTION_COUNT.fetch_add(50, Ordering::Relaxed);
        sys.tick();
        let mut cnt3 = 0xFFFF;
        let mut uif3 = 0;
        for slot in &sys.p.peripherals {
            let mut b = slot.peripheral.borrow_mut();
            if let Some(t) = b.as_any_mut().downcast_mut::<Timer>() {
                if t.name == "TIM3" {
                    cnt3 = t.read(&sys, 0x24);
                    uif3 = t.read(&sys, 0x10) & 1;
                }
            }
        }
        assert_eq!(cnt3, 0, "slave CNT reset by master update, got {cnt3:#X}");
        assert_ne!(uif3, 0, "slave UIF set on reset-mode trigger");
    }

    // Trigger mode (SMS=110): master's update starts a stopped slave.
    #[test]
    fn trgo_trigger_starts_slaved_timer() {
        let sys = crate::system::test_dummy_system();
        for slot in &sys.p.peripherals {
            let mut b = slot.peripheral.borrow_mut();
            if let Some(t) = b.as_any_mut().downcast_mut::<Timer>() {
                if t.name == "TIM2" {
                    t.write(&sys, 0x04, 2 << 4);
                    t.write(&sys, 0x2C, 9);
                    t.write(&sys, 0x00, 1);
                } else if t.name == "TIM4" {
                    // TS=011 -> ITR2 -> TIM3 is NOT TIM2: must NOT start.
                    t.write(&sys, 0x08, (3 << 4) | 6); // SMS=110 trigger
                } else if t.name == "TIM3" {
                    // TS=001 -> ITR1 -> TIM2: must start.
                    t.write(&sys, 0x08, (1 << 4) | 6);
                }
            }
        }
        use std::sync::atomic::Ordering;
        crate::system::INSTRUCTION_COUNT.fetch_add(50, Ordering::Relaxed);
        // Shared system tick: TIM2's update queues TRGO, the end-of-tick
        // drain routes it to TIM3 (TIM4's TS points elsewhere: stays put).
        sys.tick();
        let (mut c3, mut c4) = (0, 0);
        for slot in &sys.p.peripherals {
            let mut b = slot.peripheral.borrow_mut();
            if let Some(t) = b.as_any_mut().downcast_mut::<Timer>() {
                if t.name == "TIM3" {
                    c3 = t.read(&sys, 0x00) & 1;
                } else if t.name == "TIM4" {
                    c4 = t.read(&sys, 0x00) & 1;
                }
            }
        }
        assert_ne!(c3, 0, "routed slave starts on master update");
        assert_eq!(c4, 0, "unrouted timer stays stopped");
    }
}
