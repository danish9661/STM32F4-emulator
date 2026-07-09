use std::sync::atomic::Ordering;
use crate::system::System;
use crate::system::INSTRUCTION_COUNT;
use super::Peripheral;

#[derive(Clone)]
pub struct Timer {
    pub name: String,
    tim_idx: u8,
    cr1: u32,
    cr2: u32,
    smcr: u32,
    dier: u32,
    pub sr: u32,
    egr: u32,
    ccmr1: u32,
    ccmr2: u32,
    ccer: u32,
    cnt: u32,
    psc: u32,
    arr: u32,
    ccr: [u32; 4],
    pub pwm_duty: [f64; 4],
    last_tick: u64,
}

impl Timer {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let tim_idx = match name {
            "TIM1" => Some(1), "TIM2" => Some(2), "TIM3" => Some(3),
            "TIM4" => Some(4), "TIM5" => Some(5), "TIM6" => Some(6),
            "TIM7" => Some(7), "TIM8" => Some(8), "TIM9" => Some(9),
            "TIM10" => Some(10), "TIM11" => Some(11), "TIM12" => Some(12),
            "TIM13" => Some(13), "TIM14" => Some(14),
            _ => None,
        }?;
        Some(Box::new(Self { name: name.to_string(), tim_idx, ..Self::default() }))
    }

    fn advance(&mut self, _sys: &System) {
        let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
        let elapsed = now - self.last_tick;
        self.last_tick = now;
        if elapsed == 0 { return; }
        let enabled = self.cr1 & 1;
        if enabled == 0 { return; }
        let prescaler = ((self.psc & 0xFFFF) + 1) as u64;
        let reload = if self.arr == 0 { 0xFFFF } else { self.arr } as u64;
        let ticks = elapsed as u64;
        let inc = ticks / prescaler;
        if inc == 0 { return; }
        self.cnt = (self.cnt as u64 + inc) as u32;
        if self.cnt as u64 > reload {
            self.cnt = (self.cnt as u64 % (reload + 1)) as u32;
            self.sr |= 1;
        }
        self.update_pwm(reload);
    }

    fn update_pwm(&mut self, reload: u64) {
        let ccer = self.ccer;
        let cnt = self.cnt as u64;
        let arr = reload;
        for ch in 0..4 {
            let ccr_val = self.ccr[ch] as u64;
            let cc_enabled = ccer & (1 << (ch * 4)) != 0;
            if !cc_enabled || ccr_val == 0 || arr == 0 {
                self.pwm_duty[ch] = 0.0;
                continue;
            }
            let duty = ccr_val as f64 / arr as f64;
            self.pwm_duty[ch] = if cnt < ccr_val { duty } else { 1.0 - duty };
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            name: String::new(), tim_idx: 0,
            cr1: 0, cr2: 0, smcr: 0, dier: 0, sr: 0, egr: 0,
            ccmr1: 0, ccmr2: 0, ccer: 0, cnt: 0, psc: 0, arr: 0,
            ccr: [0; 4], pwm_duty: [0.0; 4], last_tick: 0,
        }
    }
}

impl Peripheral for Timer {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.advance(sys);
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.smcr,
            0x0C => self.dier,
            0x10 => self.sr,
            0x14 => self.egr,
            0x18 => self.ccmr1,
            0x1C => self.ccmr2,
            0x20 => self.ccer,
            0x24 => self.cnt,
            0x28 => self.psc,
            0x2C => self.arr,
            0x34..=0x40 => self.ccr[((offset - 0x34) / 4) as usize],
            0x48 => 0, // DCR
            0x4C => 0, // DMAR
            0x50 => 0, // TIM1_OR
            0x54 => 0, // CCMR3 (for TIM1/TIM8)
            0x58 => 0, // CCR5 (for TIM1/TIM8)
            0x5C => 0, // CCR6 (for TIM1/TIM8)
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.advance(sys);
        match offset {
            0x00 => {
                self.cr1 = value;
                if value & (1 << 4) != 0 {
                    self.cnt = 0;
                    self.sr = 0;
                }
            }
            0x04 => self.cr2 = value,
            0x08 => self.smcr = value,
            0x0C => self.dier = value,
            0x10 => self.sr &= !value,
            0x14 => {
                self.egr = value;
                if value & 1 != 0 {
                    self.cnt = 0;
                    let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
                    self.last_tick = now;
                    self.sr |= 1;
                }
            }
            0x18 => self.ccmr1 = value,
            0x1C => self.ccmr2 = value,
            0x20 => self.ccer = value,
            0x24 => self.cnt = value,
            0x28 => self.psc = value,
            0x2C => self.arr = value,
            0x34..=0x40 => {
                let idx = ((offset - 0x34) / 4) as usize;
                self.ccr[idx] = value;
            }
            0x48 => {}
            0x4C => {}
            0x50 => {}
            0x54 => {}
            0x58 => {}
            0x5C => {}
            _ => {}
        }
    }
}
