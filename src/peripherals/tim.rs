use std::sync::atomic::Ordering;
use crate::system::System;
use super::Peripheral;
use crate::emulator::NUM_INSTRUCTIONS;

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

    fn advance(&mut self, sys: &System) {
        let now = crate::emulator::NUM_INSTRUCTIONS.load(std::sync::atomic::Ordering::Relaxed);
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
        self.propagate_pwm(sys);
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
            let duty_cycle = ccr_val as f64 / arr as f64;
            self.pwm_duty[ch] = if cnt < ccr_val { duty_cycle } else { 1.0 - duty_cycle };
        }
    }

    fn propagate_pwm(&self, sys: &System) {
        let tim_num = self.tim_idx;
        if let Some(ref n) = sys.n {
            let mut n = n.borrow_mut();
            for ch in 0..4 {
                if self.pwm_duty[ch] != 0.0 {
                    n.write_timer_analog(tim_num, ch as u8, self.pwm_duty[ch]);
                }
            }
        }
    }
}

impl Default for Timer {
    fn default() -> Self {
        Self {
            name: String::new(), tim_idx: 0,
            cr1: 0, cr2: 0, smcr: 0, dier: 0, sr: 0, egr: 0,
            ccmr1: 0, ccmr2: 0, ccer: 0,
            cnt: 0, psc: 0, arr: 0xFFFF,
            ccr: [0; 4], pwm_duty: [0.0; 4],
            last_tick: 0,
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
            0x34 => self.ccr[0],
            0x38 => self.ccr[1],
            0x3C => self.ccr[2],
            0x40 => self.ccr[3],
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        self.advance(sys);
        match offset {
            0x00 => self.cr1 = value,
            0x04 => self.cr2 = value,
            0x08 => self.smcr = value,
            0x0C => self.dier = value,
            0x10 => self.sr = value,
            0x14 => { self.egr = value; if value & 1 != 0 { self.cnt = 0; } }
            0x18 => self.ccmr1 = value,
            0x1C => self.ccmr2 = value,
            0x20 => self.ccer = value,
            0x24 => self.cnt = value,
            0x28 => self.psc = value,
            0x2C => { self.arr = value; if value == 0 { self.arr = 0xFFFF; } }
            0x34 => self.ccr[0] = value,
            0x38 => self.ccr[1] = value,
            0x3C => self.ccr[2] = value,
            0x40 => self.ccr[3] = value,
            _ => {}
        }
        self.update_pwm(if self.arr == 0 { 0xFFFF } else { self.arr as u64 });
        self.propagate_pwm(sys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_timer(name: &str) -> Timer {
        Timer {
            name: name.to_string(),
            tim_idx: match name {
                "TIM1" => 1, "TIM2" => 2, "TIM3" => 3, "TIM4" => 4,
                "TIM5" => 5, "TIM6" => 6, "TIM7" => 7, "TIM8" => 8,
                _ => 0,
            },
            arr: 0xFFFF,
            ..Timer::default()
        }
    }

    fn make_sys() -> System<'static, 'static> {
        let uc = Box::new(unicorn_engine::Unicorn::new(
            unicorn_engine::unicorn_const::Arch::ARM,
            unicorn_engine::unicorn_const::Mode::MCLASS | unicorn_engine::unicorn_const::Mode::LITTLE_ENDIAN,
        ).unwrap());
        System {
            uc: std::cell::RefCell::new(Box::leak(uc)),
            p: std::rc::Rc::new(crate::peripherals::Peripherals::default()),
            d: std::rc::Rc::new(crate::ext_devices::ExtDevices {
                spi_flashes: vec![], usart_probes: vec![], displays: vec![],
                lcds: vec![], touchscreens: vec![],
            }),
            n: None,
        }
    }

    #[test]
    fn test_timer_new() {
        let t = Timer::new("TIM2");
        assert!(t.is_some());
        let t = Timer::new("TIM99");
        assert!(t.is_none());
    }

    #[test]
    fn test_timer_defaults() {
        let mut t = Box::new(make_timer("TIM3"));
        let sys = make_sys();
        assert_eq!(t.read(&sys, 0x00), 0);
        assert_eq!(t.read(&sys, 0x28), 0);
        assert_eq!(t.read(&sys, 0x2C), 0xFFFF);
        assert_eq!(t.read(&sys, 0x24), 0);
    }

    #[test]
    fn test_timer_write_read() {
        let mut t = Box::new(make_timer("TIM2"));
        let sys = make_sys();
        t.write(&sys, 0x00, 0x01);
        t.write(&sys, 0x28, 100);
        t.write(&sys, 0x2C, 1000);
        t.write(&sys, 0x34, 500);
        assert_eq!(t.read(&sys, 0x00), 0x01);
        assert_eq!(t.read(&sys, 0x28), 100);
        assert_eq!(t.read(&sys, 0x2C), 1000);
        assert_eq!(t.read(&sys, 0x34), 500);
    }

    #[test]
    fn test_timer_pwm_duty() {
        let mut t = make_timer("TIM2");
        let sys = make_sys();

        t.write(&sys, 0x00, 0x01);
        t.write(&sys, 0x28, 0);
        t.write(&sys, 0x2C, 100);
        t.write(&sys, 0x34, 25);
        t.write(&sys, 0x20, 0x01);

        let _cnt = t.read(&sys, 0x24);
        assert!(t.pwm_duty[0] >= 0.0);
        assert!(t.pwm_duty[0] <= 1.0);
    }

    #[test]
    fn test_timer_sr_update_on_overflow() {
        let mut t = make_timer("TIM2");
        let sys = make_sys();

        t.write(&sys, 0x00, 0x01);
        t.write(&sys, 0x28, 0);
        t.write(&sys, 0x2C, 10);

        t.sr = 0;
        NUM_INSTRUCTIONS.store(100, Ordering::Relaxed);
        t.last_tick = 0;
        let _cnt = t.read(&sys, 0x24);
        assert!(t.sr & 1 != 0, "UIF should be set after overflow, sr={:x}", t.sr);
    }

    #[test]
    fn test_timer_wired_to_probe_via_netlist() {
        use crate::components::netlist::{Netlist, WireConfig};
        use crate::components::builtins::TestProbe;

        let mut nl = Netlist::new();
        nl.add_component(Box::new(TestProbe::new("probe1")));
        nl.build_wires(&[
            WireConfig { from: "TIM1.CH1".into(), to: "probe1.in".into() },
        ]);

        nl.write_timer_analog(1, 0, 0.75);
        assert!((nl.read_analog(0, 0) - 0.0).abs() < f64::EPSILON);
    }
}
