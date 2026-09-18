use crate::system::{System, instruction_count, adc_get_override};
use super::Peripheral;
use std::sync::atomic::Ordering;

fn adc_rand() -> u32 {
    let n = instruction_count();
    ((n.wrapping_mul(1103515245).wrapping_add(12345)) >> 12) as u32
}

/// Shared ADC common block (ADC_Common @ 0x40012300: CSR/CCR/CDR).
/// Silicon shares one multi-mode status/control/data register file across
/// ADC1-3; the model keeps the independent per-ADC state in `Adc` and
/// mirrors the small shared surface here:
/// - CSR (0x00): EOC1/2/3 (bits 1/9/17) + OVR1/2/3 (bits 5/13/21) mirror the
///   per-ADC SR flags (read-only; cleared by reading the ADC's own SR).
/// - CCR (0x04): stored mode/clock word (dual-mode config is accepted; the
///   model never interleaves conversions, documented below).
/// - CDR (0x08): combined data — low half = ADC1.DR, high half = ADC2.DR
///   (the dual regular-simultaneous layout; ADC3 has no CDR half).
pub struct AdcCommon {
    ccr: u32,
    /// Latched simultaneous pair (dual-mode CDR): (lo, hi, valid).
    dual_pair: (u16, u16, bool),
}

impl AdcCommon {
    /// DUAL field (CCR bits 4:0): 1 = regular-simultaneous, 2 =
    /// injected-simultaneous; other values = independent mode.
    fn dual_mode(&self) -> bool {
        matches!(self.ccr & 0x1F, 1 | 2)
    }

    /// Both ADC1+ADC2 conversion-ready (EOC set in their SR)?
    /// Non-consuming: peeks at the stored SR word, never touches DR/SR
    /// (a CDR read must not consume the conversions it samples — the
    /// latch holds them until BOTH sides are fresh).
    fn both_ready(sys: &System) -> bool {
        let mut ready = [false, false];
        for slot in &sys.p.peripherals {
            for (i, base) in [0x4001_2000u32, 0x4001_2100].iter().enumerate() {
                if slot.start == *base {
                    let mut b = slot.peripheral.borrow_mut();
                    if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                        ready[i] = a.sr_eoc();
                    }
                    break;
                }
            }
        }
        ready[0] && ready[1]
    }

    /// Sample both DR halves WITHOUT consuming EOC (peek, not a DR read:
    /// Adc::read(0x4C) clears EOC, which would eat the other side's flag
    /// mid-sample and the latch could never hold across reads).
    fn peek_halves(sys: &System) -> (u16, u16) {
        let mut lo = 0u16;
        let mut hi = 0u16;
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                    lo = (a.dr & 0xFFFF) as u16;
                }
            } else if slot.start == 0x4001_2100 {
                let mut b = slot.peripheral.borrow_mut();
                if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                    hi = (a.dr & 0xFFFF) as u16;
                }
            }
        }
        (lo, hi)
    }

    /// Whether the last CDR read returned a latched simultaneous pair.
    /// Harness scope probe for dual-mode simultaneity.
    pub fn dual_latched(&self) -> bool {
        self.dual_pair.2
    }
}

fn adc_irq(name: &str) -> i32 {
    match name {
        "ADC3" => 47,
        _ => 18, // ADC1 and ADC2 share IRQ 18
    }
}

pub struct Adc {
    name: String,
    sr: u32,
    cr1: u32,
    cr2: u32,
    smpr1: u32,
    smpr2: u32,
    jofr: [u32; 4],
    htr: u32,
    ltr: u32,
    sqr1: u32,
    sqr2: u32,
    sqr3: u32,
    jsqr: u32,
    jdr: [u32; 4],
    dr: u32,
    last_conv_start: u64,
    /// Injected-conversion timing anchor (virtual instruction clock),
    /// mirroring `last_conv_start` for the regular path.
    last_inj_start: u64,
    /// JSWSTART edge latch: JSWSTART (CR2 bit 22) is an edge trigger —
    /// a conversion starts on the 0→1 transition only, not while held.
    /// (Same edge discipline as the regular SWSTART path.)
    jsw_was_set: bool,
}

impl Default for Adc {
    fn default() -> Self {
        Self {
            name: String::new(),
            sr: 0,
            cr1: 0,
            cr2: 0x0000_0001,
            smpr1: 0,
            smpr2: 0,
            jofr: [0; 4],
            htr: 0,
            ltr: 0,
            sqr1: 0,
            sqr2: 0,
            sqr3: 0,
            jsqr: 0,
            jdr: [0; 4],
            dr: 0,
            last_conv_start: 0,
            last_inj_start: 0,
            jsw_was_set: false,
        }
    }
}

impl Adc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        // ADC1/2/3 only — "ADC_Common" is served by AdcCommon below, and a
        // prefix match here would swallow it (same class of bug as the
        // DMA1/DMA2 vs DMA2D prefix match).
        match name {
            "ADC1" | "ADC2" | "ADC3" => Some(Box::new(Self { name: name.to_string(), ..Self::default() })),
            _ => None,
        }
    }
    fn eoc_enabled(&self) -> bool { self.cr1 & (1 << 5) != 0 }
    fn ovr_enabled(&self) -> bool { self.cr1 & (1 << 4) != 0 }
    fn jeoc_enabled(&self) -> bool { self.cr1 & (1 << 7) != 0 }
    /// EOC flag peek for the common block's non-consuming mirrors
    /// (CSR/CDR must never clear flags — clearing happens only via the
    /// ADC's own SR/DR read arms).
    fn sr_eoc(&self) -> bool { self.sr & (1 << 1) != 0 }
    /// OVR flag peek (same non-consuming contract as sr_eoc).
    fn sr_ovr(&self) -> bool { self.sr & (1 << 5) != 0 }

    fn fire_interrupts(&mut self, sys: &System) {
        let irq = adc_irq(&self.name);
        if (self.sr & (1 << 1) != 0 && self.eoc_enabled()) ||
           (self.sr & (1 << 5) != 0 && self.ovr_enabled()) ||
           (self.sr & 1 != 0 && self.awd_enabled()) ||
           (self.sr & (1 << 2) != 0 && self.jeoc_enabled()) {
            sys.p.nvic.borrow_mut().set_intr_pending(irq);
        }
    }

    /// Analog watchdog: enabled via CR1 AWDEN (all regular channels) or
    /// JAWDEN (injected only — enforced per-group: with AWDEN set the
    /// injected conversions below skip the check, matching silicon where
    /// an injected-group watchdog needs JAWDEN, not AWDEN), optionally
    /// single-channel via AWDSGL+AWDCH. Fires when the converted value
    /// leaves [LTR, HTR]: sets SR bit 0 (AWD) and pends IRQ 18/47 when
    /// AWDIE (CR1 bit 6) is set. Checked on every completed conversion.
    fn awd_enabled(&self) -> bool { self.cr1 & (1 << 6) != 0 }

    /// Whether the watchdog watches a conversion from the given group:
    /// regular conversions need AWDEN (bit 23), injected need JAWDEN
    /// (bit 22) — the two enables are independent (silicon watches both
    /// groups when both bits are set; AWDEN-only ignores injected
    /// results and vice versa).
    /// NOTE: the AWDSGL channel-select (bit 9 + AWDCH) is applied by the
    /// callers, not here — this is the group gate only.
    fn awd_watches(&self, injected: bool) -> bool {
        // Read the CURRENT CR1 group bits (never a shadow: an earlier
        // draft gated injected on a stale copy and AWDEN-only runs
        // watched injected results — caught by the JAWDEN test, not
        // by review).
        let cr1 = self.cr1;
        if injected {
            cr1 & (1 << 22) != 0
        } else {
            cr1 & (1 << 23) != 0
        }
    }

    fn check_awd(&mut self, sys: &System, channel: u32, val: u32) {
        if !self.awd_watches(false) {
            return;
        }
        // AWDSGL (bit 9): watch only AWDCH (bits 4:0); otherwise all.
        // No-channel-selected edge: with AWDSGL set but AWDCH pointing at
        // a channel this conversion did not use, skip (the unwatched test
        // pins this: CH7 watched, CH5 converted → no latch).
        if self.cr1 & (1 << 9) != 0 && channel != (self.cr1 & 0x1F) {
            return;
        }
        // Sticky silicon AWD: once latched, only a DR read clears (the DR
        // arm does that); a fresh in-window conversion does NOT clear a
        // latched flag — but it must not RE-latch spuriously either. The
        // flag set below is idempotent, so no extra handling needed.
        if val < (self.ltr & 0xFFF) || val > (self.htr & 0xFFF) {
            self.sr |= 1; // AWD
            self.fire_interrupts(sys);
        }
    }

    fn set_eoc(&mut self, sys: &System) {
        // Overrun: a new conversion completes while EOC is still set (the
        // previous DR was never read). Silicon latches OVR (SR bit 5) and
        // — with OVRIE — pends the IRQ; the new sample still lands in DR.
        if self.sr & (1 << 1) != 0 {
            self.sr |= 1 << 5; // OVR
        }
        self.sr |= 1 << 1; // EOC
        self.fire_interrupts(sys);
        // DMA request on EOC when CR2 DMA (bit 8) is set: stage one
        // half-word from this ADC's DR for the DMA driver. The driver
        // (emulator.js wProcessDma / native tests) drains staged ADC
        // samples via `adc_take_dma()`. DDS (bit 9) = continuous: keep
        // staging every conversion; without it, silicon issues requests
        // only until the stream disables — here: stage once per
        // conversion while DMA stays set (the guest clears DMA to stop,
        // same observable behavior).
        if self.cr2 & (1 << 8) != 0 {
            crate::system::adc_stage_dma(self.dr & 0xFFFF);
        }
    }

    fn start_conversion(&mut self, sys: &System) {
        let n = instruction_count();
        let elapsed = n.saturating_sub(self.last_conv_start);
        if elapsed > 12 {
            let smp = if self.sqr3 & 0x1F < 7 {
                (self.smpr2 & 0x7)
            } else {
                (self.smpr1 & 0x7) >> ((self.sqr3 & 0x1F) % 6 * 3)
            };
            let sampling_cycles = match smp {
                0 => 3, 1 => 15, 2 => 28, 3 => 56,
                4 => 84, 5 => 112, 6 => 144, 7 => 480,
                _ => 3,
            };
            let conv_cycles = sampling_cycles + 12;
            if elapsed >= conv_cycles as u64 {
                let channel = self.sqr3 & 0x1F;
                let mut val = adc_get_override(&self.name, channel).unwrap_or_else(|| match channel {
                    16 | 17 => 1200 + (adc_rand() % 50),
                    18 => 1500,
                    _ => adc_rand() % 4096,
                });
                // ALIGN (CR2 bit 11): left-aligned results sit at DR[15:4]
                // (the 12-bit sample shifted up 4). Right-aligned (reset)
                // is the plain value. Overrides are 12-bit samples too,
                // so alignment applies to them the same way.
                if self.cr2 & (1 << 11) != 0 {
                    val = (val & 0xFFF) << 4;
                }
                self.dr = val;
                self.set_eoc(sys);
                self.check_awd(sys, channel, val & 0xFFF);
                // CONT (CR2 bit 1): continuous mode restarts the sequence
                // as soon as the conversion completes — re-anchor the
                // start clock so the next conversion begins immediately
                // (silicon pipelines them back-to-back; single-shot needs
                // a fresh SWSTART edge, handled by the CR2 write arm).
                if self.cr2 & (1 << 1) != 0 {
                    self.last_conv_start = n;
                }
            }
        }
        // Injected group: serviced on the same clock while JSWSTART is
        // latched (edge-armed by the CR2 write arm below). JAUTO (CR1
        // bit 10) is the auto-injection variant: the injected sequence
        // follows every regular conversion automatically, with no JSWSTART
        // needed — silicon's "regular + injected back-to-back" mode.
        let jauto = self.cr1 & (1 << 10) != 0;
        if self.jsw_was_set || jauto {
            let jelapsed = n.saturating_sub(self.last_inj_start);
            if jelapsed > 12 {
                // JL (JSQR bits 21:20): sequence length 1..4 injected
                // channels, JSQ4..JSQ1 fields, converted JSQ1-first.
                let jl = ((self.jsqr >> 20) & 3) + 1;
                let fields = [self.jsqr & 0x1F, (self.jsqr >> 5) & 0x1F,
                              (self.jsqr >> 10) & 0x1F, (self.jsqr >> 15) & 0x1F];
                let conv_cycles = 15 + 12; // default sample time + 12
                if jelapsed >= conv_cycles as u64 {
                    for i in 0..jl as usize {
                        let ch = fields[i] & 0x1F;
                        let v = adc_get_override(&self.name, ch).unwrap_or_else(|| match ch {
                            16 | 17 => 1200 + (adc_rand() % 50),
                            18 => 1500,
                            _ => adc_rand() % 4096,
                        }) & 0xFFF;
                        // JDR holds sample + JOFRx offset (signed add,
                        // clamped to 12-bit range — RM0090 §11.5.3).
                        let off = (self.jofr[i] & 0xFFF) as i32;
                        let jv = ((v as i32 + off).clamp(0, 0xFFF)) as u32;
                        self.jdr[i] = if self.cr2 & (1 << 11) != 0 { (jv & 0xFFF) << 4 } else { jv };
                        // Injected watchdog: JAWDEN-gated, same window.
                        // (Fires through fire_interrupts below so AWDIE
                        // pends the IRQ — the direct sr|= path skipped
                        // the pend and the AWD flag read looked dead.)
                        // Snapshot the gate inputs first: check_awd-style
                        // narrowing must not observe a half-updated CR1.
                        let watches_inj = self.cr1 & (1 << 22) != 0;
                        if watches_inj {
                            let single = self.cr1 & (1 << 9) != 0;
                            if !single || ch == (self.cr1 & 0x1F) {
                                if v < (self.ltr & 0xFFF) || v > (self.htr & 0xFFF) {
                                    self.sr |= 1; // AWD
                                    self.fire_interrupts(sys);
                                }
                            }
                        }
                    }
                    self.sr |= (1 << 3) | (1 << 2); // JSTRT + JEOC
                    self.fire_interrupts(sys);
                    self.last_inj_start = n;
                    // JSWSTART is edge-consumed (silicon clears the start
                    // condition once the sequence launches); JAUTO stays
                    // level (every regular conversion re-triggers).
                    if !jauto {
                        self.jsw_was_set = false;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{adc_set_override, adc_clear_override, INSTRUCTION_COUNT};
    use std::sync::atomic::Ordering;

    // Injected group: JSWSTART edge runs the JL-length sequence from
    // JSQ1-first, JDR holds sample+JOFR offset, JSTRT+JEOC latch; JDR
    // reads clear JSTRT (first) / JEOC (last); ALIGN shifts JDR too.
    #[test]
    fn injected_group_runs_jsqr_sequence() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc_set_override("ADC1", 7, 0xABC);
        adc_set_override("ADC1", 8, 0x123);
        adc.write(&sys, 0x38, (1 << 20) | (8 << 5) | 7); // JL=1: JSQ2=8, JSQ1=7
        adc.write(&sys, 0x14, 16); // JOFR1 = +16
        adc.write(&sys, 0x08, (1 << 22) | 1); // JSWSTART edge
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x3C), 0xACC, "JDR1 = sample + offset");
        assert_eq!(adc.read(&sys, 0x40), 0x123, "JDR2 = second channel");
        adc_clear_override("ADC1", 7);
        adc_clear_override("ADC1", 8);
    }

    // JEOC/JSTRT clear discipline: JDR1 read clears JSTRT, the last
    // sequence entry clears JEOC; JEOCIE (CR1 bit 7) pends the IRQ.
    #[test]
    fn injected_flags_clear_on_jdr_drain() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc_set_override("ADC1", 7, 0x100);
        adc.write(&sys, 0x38, 7); // JL=0: single channel JSQ1=7
        adc.write(&sys, 0x04, 1 << 7); // JEOCIE
        adc.write(&sys, 0x08, (1 << 22) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        // SR read does not consume (only JDR reads do).
        assert_ne!(adc.read(&sys, 0x00) & 0xC, 0, "JSTRT+JEOC latched");
        assert!(sys.p.nvic.borrow().irq_pending(18), "JEOCIE pends IRQ 18");
        // Re-run (SR read cleared flags), then drain JDR1 = last entry.
        adc.write(&sys, 0x08, 1); // drop JSWSTART level
        adc.write(&sys, 0x08, (1 << 22) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x3C), 0x100, "JDR1 sample");
        assert_eq!(adc.read(&sys, 0x00) & 0xC, 0, "JSTRT+JEOC clear after drain");
        adc_clear_override("ADC1", 7);
    }

    // JAWDEN-only watchdog watches injected conversions (and an
    // AWDEN-only watchdog ignores them).
    #[test]
    fn injected_watchdog_needs_jawden() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc_set_override("ADC1", 7, 3000);
        adc.write(&sys, 0x38, 7);
        adc.write(&sys, 0x24, 2000); // HTR
        adc.write(&sys, 0x28, 100);  // LTR
        adc.write(&sys, 0x04, (1 << 22) | (1 << 6)); // JAWDEN+AWDIE (no AWDEN)
        adc.write(&sys, 0x08, (1 << 22) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_ne!(adc.sr & 1, 0, "JAWDEN watches injected OOR");
        // AWDEN-only: same conversion, no AWD. Fresh ADC state (the
        // first sequence's AWD latch + JSWSTART level belong to the
        // JAWDEN run — a new instance starts clean, like silicon after
        // a CR1 reprogram + flag clear). Pin SQR3 CH0 in-window too:
        // the CR2 read arm also services an incidental regular
        // conversion (ADON armed) on CH0 with a random value, which
        // would latch AWD on its own — the override keeps it in-window
        // so only the injected group is under test.
        adc_set_override("ADC1", 0, 500);
        let mut boxed2 = Adc::new("ADC1").unwrap();
        let adc2 = boxed2.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc2.write(&sys, 0x38, 7);
        adc2.write(&sys, 0x24, 2000);
        adc2.write(&sys, 0x28, 100);
        adc2.write(&sys, 0x04, (1 << 23) | (1 << 6)); // AWDEN+AWDIE (no JAWDEN)
        adc2.write(&sys, 0x08, (1 << 22) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc2.read(&sys, 0x08);
        assert_eq!(adc2.sr & 1, 0, "AWDEN-only ignores injected");
        adc_clear_override("ADC1", 0);
        adc_clear_override("ADC1", 7);
    }

    // ALIGN (CR2 bit 11) left-shifts DR by 4; CONT (bit 1) re-anchors so
    // conversions repeat without a fresh SWSTART edge.
    #[test]
    fn align_shifts_and_cont_repeats() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc_set_override("ADC1", 5, 0xABC);
        adc.write(&sys, 0x34, 5);
        adc.write(&sys, 0x08, (1 << 30) | (1 << 11) | (1 << 1) | 1); // SWSTART+ALIGN+CONT
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x4C), 0xABC0, "ALIGN left-shifts DR");
        // CONT: a second conversion completes with no new SWSTART edge.
        adc.read(&sys, 0x4C); // drain (clears EOC)
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x4C), 0xABC0, "CONT repeats without edge");
        adc_clear_override("ADC1", 5);
    }

    #[test]
    fn channel_override_takes_priority_over_random() {
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        let sys = crate::system::test_dummy_system();

        adc_set_override("ADC1", 5, 0x0ABC);
        adc.write(&sys, 0x34, 5);              // SQR3: select channel 5
        adc.write(&sys, 0x08, (1 << 30) | 1);   // CR2: SWSTART, keep ADON set
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed); // elapse past conv_cycles
        adc.read(&sys, 0x08);                   // CR2 read drives start_conversion
        assert_eq!(adc.read(&sys, 0x4C), 0x0ABC, "DR should reflect the override, not LCG random");
        adc_clear_override("ADC1", 5);

        // Without an override, the channel falls back to the existing
        // pseudo-random behavior (still in the real 12-bit ADC range).
        adc.write(&sys, 0x08, (1 << 30) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert!(adc.read(&sys, 0x4C) < 4096);
    }

    #[test]
    fn awd_fires_irq_only_outside_thresholds() {
        use crate::system::test_dummy_system;
        let sys = test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        // Watch channel 5 alone, window [100, 2000], AWDIE on.
        adc.write(&sys, 0x24, 2000); // HTR
        adc.write(&sys, 0x28, 100);  // LTR
        adc.write(&sys, 0x04, (1 << 23) | (1 << 9) | (1 << 6) | 5); // AWDEN+AWDSGL+AWDIE+CH5
        adc.write(&sys, 0x34, 5);
        // In-window value: no AWD, no IRQ.
        adc_set_override("ADC1", 5, 500);
        adc.write(&sys, 0x08, (1 << 30) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x00) & 1, 0, "in-window value must not set AWD");
        assert!(!sys.p.nvic.borrow().irq_pending(18), "no IRQ while in window");
        // Out-of-window high: AWD + IRQ 18.
        adc_set_override("ADC1", 5, 3000);
        adc.write(&sys, 0x08, (1 << 30) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_ne!(adc.read(&sys, 0x00) & 1, 0, "out-of-window value must set AWD");
        assert!(sys.p.nvic.borrow().irq_pending(18), "AWDIE must pend IRQ 18");
        // Other channel ignored under AWDSGL (AWD is sticky: clear the
        // latched flag first via a DR read, then prove no re-latch).
        sys.p.nvic.borrow_mut().clear_pending(18);
        adc.read(&sys, 0x4C); // drain DR: clears EOC + AWD latch
        adc.write(&sys, 0x04, (1 << 23) | (1 << 9) | (1 << 6) | 7); // watch CH7
        adc.write(&sys, 0x34, 5); // but convert CH5 (out of window)
        adc.write(&sys, 0x08, (1 << 30) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc.read(&sys, 0x00) & 1, 0, "unwatched channel must not set AWD");
        adc_clear_override("ADC1", 5);
    }

    #[test]
    fn dma_bit_stages_one_sample_per_conversion() {
        use crate::system::{test_dummy_system, adc_take_dma};
        // Drain first: the queue is process-global and an earlier test in
        // this binary may have staged samples (parallel cargo runs share
        // it; reset_globals only runs on emulator init, not per test).
        adc_take_dma();
        let sys = test_dummy_system();
        let mut boxed = Adc::new("ADC1").unwrap();
        let adc = boxed.as_any_mut().downcast_mut::<Adc>().unwrap();
        adc_set_override("ADC1", 5, 0x0ABC);
        adc.write(&sys, 0x34, 5);
        // DMA clear: conversions complete (EOC sets) but stage nothing.
        adc.write(&sys, 0x08, (1 << 30) | 1);
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert!(adc_take_dma().is_empty(), "no staging while CR2 DMA=0");
        // DMA set: one sample per conversion, in order.
        adc.write(&sys, 0x08, (1 << 30) | 1 | (1 << 8)); // SWSTART+ADON+DMA
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        adc.write(&sys, 0x08, (1 << 30) | 1 | (1 << 8));
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        adc.read(&sys, 0x08);
        assert_eq!(adc_take_dma(), vec![0x0ABC, 0x0ABC], "one staged sample per EOC");
        adc_clear_override("ADC1", 5);
    }

    #[test]
    fn common_block_mirrors_flags_and_data() {
        use std::sync::atomic::Ordering;
        let sys = crate::system::test_dummy_system();
        // Drive ADC1 to EOC via a real conversion, ADC2 stays idle.
        // (last_conv_start anchors at the SWSTART write, so the clock must
        // advance AFTER the write, like the override test does.)
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                let a = b.as_any_mut().downcast_mut::<Adc>().unwrap();
                a.write(&sys, 0x34, 5);
                a.write(&sys, 0x08, (1 << 30) | 1);
            }
        }
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        // NOTE: read the CSR mirror BEFORE touching ADC1's own SR/DR —
        // the SR read clears EOC, which would clear the mirror too.
        let mut csr = 0u32;
        let mut cdr = 0u32;
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2300 {
                let mut b = slot.peripheral.borrow_mut();
                let c = b.as_any_mut().downcast_mut::<AdcCommon>().unwrap();
                // Trigger the conversion through the CR2 read path first
                // (same path the override test uses), then sample CSR/CDR.
                for s2 in &sys.p.peripherals {
                    if s2.start == 0x4001_2000 {
                        let mut b2 = s2.peripheral.borrow_mut();
                        let a2 = b2.as_any_mut().downcast_mut::<Adc>().unwrap();
                        a2.read(&sys, 0x08);
                        break;
                    }
                }
                csr = c.read(&sys, 0x00);
                cdr = c.read(&sys, 0x08);
            }
        }
        let mut dr1 = 0u32;
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                let a = b.as_any_mut().downcast_mut::<Adc>().unwrap();
                dr1 = a.read(&sys, 0x4C);
            }
        }
        assert_ne!(csr & (1 << 1), 0, "CSR EOC1 mirrors ADC1 EOC");
        assert_eq!(csr & (1 << 9), 0, "CSR EOC2 clear while ADC2 idle");
        assert_eq!(cdr & 0xFFFF, dr1 & 0xFFFF, "CDR low half = ADC1.DR");
        // CCR stores; CSR is read-only (but EOC1 was already consumed by
        // the DR read above, so re-trigger before asserting read-only).
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                let a = b.as_any_mut().downcast_mut::<Adc>().unwrap();
                a.write(&sys, 0x08, (1 << 30) | 1);
            }
        }
        INSTRUCTION_COUNT.fetch_add(100, Ordering::Relaxed);
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                let a = b.as_any_mut().downcast_mut::<Adc>().unwrap();
                a.read(&sys, 0x08);
            }
            if slot.start == 0x4001_2300 {
                let mut b = slot.peripheral.borrow_mut();
                let c = b.as_any_mut().downcast_mut::<AdcCommon>().unwrap();
                c.write(&sys, 0x04, 0x0003_0001);
                assert_eq!(c.read(&sys, 0x04), 0x0003_0001, "CCR stores");
                c.write(&sys, 0x00, 0xFFFF_FFFF);
                assert_ne!(c.read(&sys, 0x00) & (1 << 1), 0, "CSR write ignored (EOC1 still set)");
            }
        }
    }

    #[test]
    fn f429_c_adc_alias_serves_common_block() {
        // The F429 Keil SVD names the shared block C_ADC (same base
        // 0x40012300, same CSR/CCR/CDR layout). from_svd must bind it to
        // AdcCommon so CSR/CDR work on F429 maps too — previously the
        // constructor rejected the name and the block read benign-0.
        assert!(AdcCommon::new("C_ADC").is_some(), "C_ADC alias accepted");
        assert!(AdcCommon::new("ADC_Common").is_some(), "ADC_Common still accepted");
        assert!(AdcCommon::new("ADC1").is_none(), "ADC1 is not a common block");
        let sys = crate::system::test_dummy_system();
        let p = crate::peripherals::Peripherals::from_svd(
            include_str!("../../../site/vendor/stm32f429.svd"),
            crate::system::dummy_gpio(),
            &crate::ext_devices::ExtDevices::default(),
        );
        let slot = p.peripherals.iter().find(|s| s.start == 0x4001_2300);
        assert!(slot.is_some(), "C_ADC slot claimed at 0x40012300 on F429 map");
        let mut b = slot.unwrap().peripheral.borrow_mut();
        assert!(
            b.as_any_mut().downcast_mut::<AdcCommon>().is_some(),
            "F429 0x40012300 slot is an AdcCommon"
        );
        drop(b);
        drop(p);
        drop(sys);
    }
}

impl Peripheral for Adc {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                // SR read clears STRT/JSTRT (start flags are edge
                // observations) but preserves the latched result flags
                // EOC/JEOC/AWD/OVR — those clear on their DR/JDR reads
                // (silicon: status clears via the conversion-data reads;
                // only the start strobes are SR-read-consumed).
                let sr = self.sr;
                self.sr &= !((1 << 4) | (1 << 3));
                sr
            }
            0x04 => self.cr1,
            0x08 => {
                if self.cr2 & 1 != 0 {
                    self.start_conversion(sys);
                }
                self.cr2
            }
            0x0C => self.smpr1,
            0x10 => self.smpr2,
            0x14..=0x20 => {
                let i = ((offset - 0x14) / 4) as usize;
                self.jofr.get(i).copied().unwrap_or(0)
            }
            0x24 => self.htr,
            0x28 => self.ltr,
            0x2C => self.sqr1,
            0x30 => self.sqr2,
            0x34 => self.sqr3,
            0x38 => self.jsqr,
            0x3C..=0x48 => {
                let i = ((offset - 0x3C) / 4) as usize;
                let v = self.jdr.get(i).copied().unwrap_or(0);
                // JDR read clears JEOC once the whole injected sequence
                // has been drained (silicon: JEOC clears when all JDRs
                // are read; JSTRT clears on the first). Track per-index:
                // clear JSTRT on JDR1, JEOC when the last sequence entry
                // is read.
                if i == 0 {
                    self.sr &= !(1 << 3); // JSTRT
                }
                let jl = ((self.jsqr >> 20) & 3) as usize;
                if i >= jl {
                    self.sr &= !(1 << 2); // JEOC
                }
                v
            }
            0x4C => {
                let dr = self.dr;
                // DR read clears EOC + OVR + AWD (silicon: a DR read
                // acknowledges the latched result flags; only the STRT/
                // JSTRT start strobes clear on an SR read).
                self.sr &= !((1 << 1) | (1 << 5) | 1);
                dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.sr = value & 0x3F;
                self.fire_interrupts(sys);
            }
            0x04 => {
                self.cr1 = value & 0x7FFF_FFFF;
                self.fire_interrupts(sys);
            }
            0x08 => {
                let was_swstart = self.cr2 & (1 << 30);
                // JSWSTART (bit 22) is an edge trigger: capture the
                // pre-write level BEFORE the mask stores the new one.
                let was_jsw = self.cr2 & (1 << 22) != 0;
                // CR2 mask keeps DMA (bit 8) + DDS (bit 9): 0x7FF0_0EFF has
                // bit 8 = 0 (DMA was silently dropped on every write — no
                // DMA request could ever arm). Correct mask: 0x7FF0_0FFF.
                // (Bit 22 survives the mask and reads back the level.)
                self.cr2 = value & 0x7FF0_0FFF;
                if value & (1 << 30) != 0 && was_swstart == 0 {
                    self.last_conv_start = instruction_count();
                }
                // Arm one injected sequence on the 0→1 transition only
                // (silicon starts the injected group on the edge; holding
                // the bit does not retrigger). Writing 0 clears a pending
                // (not-yet-launched) arm; a launched sequence runs out.
                if value & (1 << 22) != 0 && !was_jsw {
                    self.last_inj_start = instruction_count();
                    self.jsw_was_set = true;
                } else if value & (1 << 22) == 0 {
                    self.jsw_was_set = false;
                }
            }
            0x0C => self.smpr1 = value,
            0x10 => self.smpr2 = value,
            0x14..=0x20 => {
                let i = ((offset - 0x14) / 4) as usize;
                if let Some(r) = self.jofr.get_mut(i) { *r = value & 0xFFF; }
            }
            0x24 => self.htr = value & 0xFFF,
            0x28 => self.ltr = value & 0xFFF,
            0x2C => self.sqr1 = value,
            0x30 => self.sqr2 = value,
            0x34 => self.sqr3 = value,
            0x38 => self.jsqr = value,
            0x3C..=0x48 => {}
            0x4C => {}
            _ => {}
        }
    }
}

impl AdcCommon {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        // F407 SVD names it ADC_Common; the F429 Keil SVD calls the same
        // block C_ADC (same base 0x40012300, same CSR/CCR/CDR layout).
        if name == "ADC_Common" || name == "ADCCommon" || name == "C_ADC" {
            Some(Box::new(Self { ccr: 0, dual_pair: (0, 0, false) }))
        } else {
            None
        }
    }

    /// CSR bit positions for ADCn (n = 0/1/2): EOC at 1+8n, OVR at 5+8n.
    fn csr_bits(sys: &System) -> u32 {
        // Read-only mirror of the per-ADC EOC/OVR flags: must NOT consume
        // them. Clearing happens only via the ADC's own SR/DR reads.
        // NOTE: this reads the stored flag WORD directly (sr_eoc/sr_ovr),
        // never the SR read arm (which clears SR) — an early version went
        // through the arm, so sampling CSR ate both EOCs and the dual CDR
        // latch could never hold across reads (caught by the mock's
        // one-sided-reconversion pin, not by review).
        let mut v = 0u32;
        for (i, base) in [0x4001_2000u32, 0x4001_2100, 0x4001_2200].iter().enumerate() {
            for slot in &sys.p.peripherals {
                if slot.start == *base {
                    let mut b = slot.peripheral.borrow_mut();
                    if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                        if a.sr_eoc() {
                            v |= 1 << (1 + 8 * i);
                        }
                        if a.sr_ovr() {
                            v |= 1 << (5 + 8 * i);
                        }
                    }
                    break;
                }
            }
        }
        v
    }

    /// CDR halves: low = ADC1.DR, high = ADC2.DR.
    /// Dual regular-simultaneous mode (CCR DUAL[4:0] = 0b00001/0b00010):
    /// when the common block is in a dual mode AND both ADC1+ADC2 have a
    /// conversion ready (EOC set), CDR latches the SIMULTANEOUS pair —
    /// both halves update on the same read (the silicon guarantee: the
    /// two halves are the same conversion instant). Outside dual mode
    /// (or with only one side ready) each half follows its own ADC live
    /// (the old behavior — kept verbatim for the non-dual path).
    /// `dual_latched` reports whether the last CDR read was a latched
    /// simultaneous pair (harness scope probe for the simultaneity).
    fn cdr_halves(sys: &System) -> u32 {
        let mut lo = 0u32;
        let mut hi = 0u32;
        for slot in &sys.p.peripherals {
            if slot.start == 0x4001_2000 {
                let mut b = slot.peripheral.borrow_mut();
                if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                    lo = a.dr & 0xFFFF;
                }
            } else if slot.start == 0x4001_2100 {
                let mut b = slot.peripheral.borrow_mut();
                if let Some(a) = b.as_any_mut().downcast_mut::<Adc>() {
                    hi = a.dr & 0xFFFF;
                }
            }
        }
        lo | (hi << 16)
    }
}

impl Peripheral for AdcCommon {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            // CSR mirrors the per-ADC EOC/OVR flags (read-only; the ADC's
            // own SR read clears them).
            0x00 => Self::csr_bits(sys),
            0x04 => self.ccr,
            // CDR: dual-mode simultaneous latch when DUAL[4:0] is 1/2 and
            // both sides are conversion-ready; else live halves. The
            // latched pair is what makes "simultaneous" observable: two
            // back-to-back CDR reads return the SAME pair even if a new
            // conversion lands on one side between them (the latch only
            // refreshes when both sides are ready again). Sampling is a
            // peek (no DR read: that would clear EOC and eat the other
            // side's flag mid-sample, so the latch could never hold).
            0x08 => {
                if self.dual_mode() && Self::both_ready(sys) {
                    let (lo, hi) = Self::peek_halves(sys);
                    self.dual_pair = (lo, hi, true);
                    (lo as u32) | ((hi as u32) << 16)
                } else if self.dual_pair.2 && self.dual_mode() {
                    (self.dual_pair.0 as u32) | ((self.dual_pair.1 as u32) << 16)
                } else {
                    self.dual_pair.2 = false;
                    Self::cdr_halves(sys)
                }
            }
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            // CSR is read-only (flags clear via the ADC's own SR).
            // CCR: leaving/entering a dual mode drops the latch (fresh
            // pair required after a mode change — silicon re-arms).
            0x04 => {
                let was_dual = self.dual_mode();
                self.ccr = value & 0x00FF_FFFF;
                if self.dual_mode() != was_dual {
                    self.dual_pair.2 = false;
                }
            }
            _ => {}
        }
    }
}
