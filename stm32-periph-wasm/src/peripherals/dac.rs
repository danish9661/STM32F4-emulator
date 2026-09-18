use crate::system::System;
use super::Peripheral;

pub struct Dac {
    cr: u32,
    swtrigr: u32,
    dhr12r1: u32,
    dhr12l1: u32,
    dhr8r1: u32,
    dhr12r2: u32,
    dhr12l2: u32,
    dhr8r2: u32,
    dhr12rd: u32,
    dhr12ld: u32,
    dhr8rd: u32,
    dor1: u32,
    dor2: u32,
    sr: u32,
    // Noise/triangle state
    lfsr1: u16,
    lfsr2: u16,
    tri_cnt1: u16,
    tri_cnt2: u16,
    tri_dir1: bool,
    tri_dir2: bool,
}

impl Default for Dac {
    fn default() -> Self {
        Self {
            sr: 0x0000_0000,
            lfsr1: 0xAAAA,
            lfsr2: 0xAAAA,
            tri_dir1: true,
            tri_dir2: true,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Dac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DAC" { Some(Box::new(Self::default())) } else { None }
    }

    fn mamp1(&self) -> u32 { (self.cr >> 12) & 0xF }
    fn mamp2(&self) -> u32 { (self.cr >> 28) & 0xF }
    fn wave1(&self) -> u32 { (self.cr >> 8) & 0x3 }
    fn wave2(&self) -> u32 { (self.cr >> 24) & 0x3 }

    /// Trigger-source select (TSEL1 bits 5:3 / TSEL2 bits 21:19):
    /// 0 = TIM6 TRGO, 1 = TIM8 TRGO, 2 = TIM7 TRGO, 3 = TIM5 TRGO,
    /// 4 = TIM2 TRGO, 5 = TIM4 TRGO, 6 = EXTI9, 7 = SWTRIG (software).
    fn tsel1(&self) -> u32 { (self.cr >> 3) & 7 }
    fn tsel2(&self) -> u32 { (self.cr >> 19) & 7 }

    /// Whether a trigger source fires the channel now: TENx (bit 2/18)
    /// gates hardware triggers; SWTRIG (SWTRIGR bit 0/1) always fires
    /// (software trigger ignores TEN — RM0090 §13.5.5). `src` is 0..7
    /// for TIM6/TIM8/TIM7/TIM5/TIM2/TIM4/EXTI9/SW; only the matching
    /// TSEL fires when TEN is set.
    fn trigger_fires(&self, ch: u8, src: u8) -> bool {
        let (tsel, ten) = if ch == 1 { (self.tsel1(), self.cr & (1 << 2) != 0) }
                          else { (self.tsel2(), self.cr & (1 << 18) != 0) };
        if src == 7 {
            return true; // SWTRIG always fires
        }
        ten && tsel == src as u32
    }

    /// DMA underrun flag (DMAUDR1 bit 13 / DMAUDR2 bit 29): set when a
    /// DMA-fed trigger arrives with no fresh DHR staged (the DMA engine
    /// did not keep up — silicon holds DOR and flags it). Cleared by
    /// writing the bit (w1c-ish: the SR write arm clears on 1).
    fn flag_underrun(&mut self, ch: u8) {
        if ch == 1 { self.sr |= 1 << 13; } else { self.sr |= 1 << 29; }
    }

    fn update_dor1(&mut self) {
        if self.cr & 1 != 0 { // EN1
            let raw = self.dhr12r1 & 0xFFF;
            self.dor1 = raw;
        }
    }

    fn update_dor2(&mut self) {
        if self.cr & (1 << 16) != 0 { // EN2
            let raw = self.dhr12r2 & 0xFFF;
            self.dor2 = raw;
        }
    }

    /// Hardware trigger arrival on channel `ch` from source `src`
    /// (0..7 = TIM6/TIM8/TIM7/TIM5/TIM2/TIM4/EXTI9/SW). With DMAENx set
    /// and no fresh sample staged since the last trigger, latches
    /// DMAUDR (the underrun contract); otherwise advances the waveform
    /// (noise/triangle) and loads DOR from DHR ( copied sample).
    /// `dma_staged` = the DMA engine delivered a fresh DHR since the
    /// last trigger (harness tracks it via `dac_dma_stage`).
    pub fn hw_trigger(&mut self, ch: u8, src: u8, dma_staged: bool) {
        if !self.trigger_fires(ch, src) {
            return;
        }
        let dmaen = if ch == 1 { self.cr & (1 << 12) != 0 } else { self.cr & (1 << 28) != 0 };
        if dmaen && !dma_staged {
            self.flag_underrun(ch);
            return; // DOR holds (no fresh sample — silicon stalls it)
        }
        self.advance_waveform(ch);
        if ch == 1 { self.update_dor1(); } else { self.update_dor2(); }
    }

    fn advance_waveform(&mut self, ch: u8) {
        let wave = if ch == 1 { self.wave1() } else { self.wave2() };
        let mamp = if ch == 1 { self.mamp1() } else { self.mamp2() };
        match wave {
            0b01 => {
                let lfsr = if ch == 1 { &mut self.lfsr1 } else { &mut self.lfsr2 };
                let bit = ((*lfsr >> 0) ^ (*lfsr >> 2) ^ (*lfsr >> 6) ^ (*lfsr >> 7)) & 1;
                *lfsr = ((*lfsr >> 1) | (bit << 10)) & 0x7FF;
            }
            0b10 | 0b11 => {
                let cnt = if ch == 1 { &mut self.tri_cnt1 } else { &mut self.tri_cnt2 };
                let dir = if ch == 1 { &mut self.tri_dir1 } else { &mut self.tri_dir2 };
                let lsb_mask = if mamp == 0 { 0 } else { (1 << (mamp - 1)) - 1 };
                let max_cnt = lsb_mask;
                if *dir {
                    *cnt = cnt.wrapping_add(1);
                    if *cnt >= max_cnt as u16 { *dir = false; }
                } else {
                    *cnt = cnt.wrapping_sub(1);
                    if *cnt == 0 { *dir = true; }
                }
            }
            _ => {}
        }
    }

    /// Physical-sink sample probe (harness scope): the 12-bit value the
    /// analog pin would carry right now = DOR when the channel is enabled,
    /// else undriven (None → harness reads "floating"). There is no analog
    /// pin layer — this is the documented substitute: DOR readback IS the
    /// sink value (register model real), and firmware polling DOR observes
    /// exactly what the probe reports.
    pub fn sink_sample(&self, ch: u8) -> Option<u16> {
        match ch {
            1 => if self.cr & 1 != 0 { Some((self.dor1 & 0xFFF) as u16) } else { None },
            2 => if self.cr & (1 << 16) != 0 { Some((self.dor2 & 0xFFF) as u16) } else { None },
            _ => None,
        }
    }

    /// DMAUDR underrun latched for channel `ch` (scope probe).
    pub fn underrun(&self, ch: u8) -> bool {
        if ch == 1 { self.sr & (1 << 13) != 0 } else { self.sr & (1 << 29) != 0 }
    }
}

impl Peripheral for Dac {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.swtrigr,
            0x08 => self.dhr12r1,
            0x0C => self.dhr12l1,
            0x10 => self.dhr8r1,
            0x14 => self.dhr12r2,
            0x18 => self.dhr12l2,
            0x1C => self.dhr8r2,
            0x20 => self.dhr12rd,
            0x24 => self.dhr12ld,
            0x28 => self.dhr8rd,
            0x2C => self.dor1,
            0x30 => self.dor2,
            0x34 => self.sr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0x3F3F_FFFF,
            0x04 => {
                self.swtrigr = value;
                if value & 1 != 0 { self.advance_waveform(1); self.update_dor1(); }
                if value & (1 << 1) != 0 { self.advance_waveform(2); self.update_dor2(); }
            }
            0x08 => { self.dhr12r1 = value & 0xFFF; self.update_dor1(); }
            0x0C => { self.dhr12l1 = value & 0xFFF0; self.update_dor1(); }
            0x10 => { self.dhr8r1 = value & 0xFF; self.update_dor1(); }
            0x14 => { self.dhr12r2 = value & 0xFFF; self.update_dor2(); }
            0x18 => { self.dhr12l2 = value & 0xFFF0; self.update_dor2(); }
            0x1C => { self.dhr8r2 = value & 0xFF; self.update_dor2(); }
            0x20 => { self.dhr12rd = value; self.update_dor1(); self.update_dor2(); }
            0x24 => { self.dhr12ld = value; self.update_dor1(); self.update_dor2(); }
            0x28 => { self.dhr8rd = value; self.update_dor1(); self.update_dor2(); }
            0x2C | 0x30 => {} // DOR is read-only
            // SR: DMAUDR1 (bit 13) / DMAUDR2 (bit 29) clear on 1-write
            // (silicon w1c for the underrun flags; other bits reserved).
            0x34 => {
                if value & (1 << 13) != 0 { self.sr &= !(1 << 13); }
                if value & (1 << 29) != 0 { self.sr &= !(1 << 29); }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod gap10_tests {
    use super::*;
    use crate::system::test_dummy_system;

    // TSEL trigger mux: TEN-gated source match loads DOR; SW always
    // fires; wrong source with TEN set does not; TEN clear ignores
    // hardware sources (SW still fires).
    #[test]
    fn tsel_mux_gates_triggers() {
        let _sys = test_dummy_system();
        let mut d = Dac::default();
        d.cr = 1 | (4 << 3) | (1 << 2); // EN1 + TSEL1=TIM2(4) + TEN1
        d.dhr12r1 = 0xABC;
        d.hw_trigger(1, 4, true); // TIM2: match → DOR loads
        assert_eq!(d.dor1, 0xABC, "matching TSEL fires");
        d.dor1 = 0;
        d.hw_trigger(1, 0, true); // TIM6: mismatch → held
        assert_eq!(d.dor1, 0, "wrong source held under TEN");
        d.hw_trigger(1, 7, true); // SW: always fires
        assert_eq!(d.dor1, 0xABC, "SW fires regardless of TSEL");
        d.cr &= !(1 << 2); // TEN clear
        d.dor1 = 0;
        d.hw_trigger(1, 4, true); // hardware source ignored now
        assert_eq!(d.dor1, 0, "TEN clear blocks hardware trigger");
        d.hw_trigger(1, 7, true);
        assert_eq!(d.dor1, 0xABC, "SW still fires with TEN clear");
    }

    // DMAUDR: DMAEN + trigger with nothing staged latches the underrun
    // and holds DOR; a staged sample loads DOR with no flag; w1c clears.
    #[test]
    fn dmaudr_latches_without_staged_sample() {
        let _sys = test_dummy_system();
        let mut d = Dac::default();
        d.cr = 1 | (4 << 3) | (1 << 2) | (1 << 12); // EN1+TSEL+TEN1+DMAEN1
        d.dhr12r1 = 0x123;
        d.hw_trigger(1, 4, false); // nothing staged → underrun
        assert!(d.underrun(1), "DMAUDR1 latches");
        assert_eq!(d.dor1, 0, "DOR holds on underrun");
        d.hw_trigger(1, 4, true); // staged → loads, flag stays (sticky)
        assert_eq!(d.dor1, 0x123, "staged sample loads DOR");
        assert!(d.underrun(1), "DMAUDR sticky until cleared");
        d.write(&_sys, 0x34, 1 << 13); // w1c clear
        assert!(!d.underrun(1), "DMAUDR clears on 1-write");
    }
}
