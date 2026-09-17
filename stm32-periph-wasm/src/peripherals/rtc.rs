use crate::system::{System, INSTRUCTION_COUNT};
use std::sync::atomic::Ordering;
use super::Peripheral;

fn bcd_add(a: u32, b: u32, max: u32) -> (u32, bool) {
    let raw = a + b;
    let al = (a & 0x0F) + (b & 0x0F);
    let adj_low = if al > 9 { al - 10 } else { al };
    let carry = (al > 9) as u32;
    let ah = (a >> 4) + (b >> 4) + carry;
    let val = (ah << 4) | adj_low;
    let overflow = if max >= 0x59 { ah > 5 || (ah == 5 && adj_low > 9) } else { ah > 2 || (ah == 2 && adj_low > 3) };
    (val, overflow || raw > max)
}

fn bcd_inc_tr(tr: u32) -> u32 {
    let sec = (tr >> 0) & 0x7F;
    let min = (tr >> 8) & 0x7F;
    let hr  = (tr >> 16) & 0x3F;
    let (new_sec, carry_sec) = bcd_add(sec, 1, 0x59);
    let (new_min, carry_min) = if carry_sec { bcd_add(min, 1, 0x59) } else { (min, false) };
    let (new_hr, _) = if carry_min { bcd_add(hr, 1, 0x23) } else { (hr, false) };
    ((tr & 0xFF00_0000) | (new_hr << 16) | (new_min << 8) | new_sec)
}

fn bcd_match(tr_bcd: u32, alarm_bcd: u32, mask_bits: u32) -> bool {
    // alarm_bcd has MSB bits per field indicating "don't care"
    let _ = mask_bits;
    // mask_bits: bit 7 of each byte = don't care
    let sec_match = if alarm_bcd & (1 << 7) != 0 { true } else { (tr_bcd & 0x7F) == (alarm_bcd & 0x7F) };
    let min_match = if alarm_bcd & (1 << 15) != 0 { true } else { ((tr_bcd >> 8) & 0x7F) == ((alarm_bcd >> 8) & 0x7F) };
    let hr_match = if alarm_bcd & (1 << 23) != 0 { true } else { ((tr_bcd >> 16) & 0x3F) == ((alarm_bcd >> 16) & 0x3F) };
    sec_match && min_match && hr_match
}

/// Smooth-calibration accumulator (net pulses over each 512-second
/// window). Process-global like INSTRUCTION_COUNT: the RTC advances on
/// the shared virtual clock, so one accumulator is correct.
fn cal_accum_add(net: i64) -> (bool, bool) {
    use std::sync::OnceLock;
    static ACC: OnceLock<std::sync::Mutex<i64>> = OnceLock::new();
    let m = ACC.get_or_init(|| std::sync::Mutex::new(0));
    let mut acc = m.lock().unwrap();
    *acc += net;
    // Returns (add_second, skip_second) for this window edge.
    if *acc >= 512 {
        *acc -= 512;
        (true, false)
    } else if *acc <= -512 {
        *acc += 512;
        (false, true)
    } else {
        (false, false)
    }
}

pub struct Rtc {
    tr: u32, dr: u32, cr: u32, isr: u32, prer: u32, wutr: u32,
    calibr: u32, alrmar: u32, alrmbr: u32, wpr: u32, ssr: u32,
    shiftr: u32, tstr: u32, tsdr: u32, tsssr: u32, calr: u32,
    tafcr: u32, alrmassr: u32, alrmbssr: u32, bkp: [u32; 20],
    last_inst: u64,
    /// Wakeup-timer countdown state: reload value armed by a WUTR write
    /// while WUTE is set; counts down in whole seconds alongside TR.
    /// Silicon clocks it from RTCCLK/2/4/8/16 or 1Hz+WUCKSEL; here one
    /// second of virtual time decrements it by one (same clock as TR —
    /// the observable contract is the flag + IRQ, not the prescaler).
    wut_reload: u16,
    wut_count: u16,
    wut_armed: bool,
}

impl Default for Rtc {
    fn default() -> Self {
        unsafe { std::mem::zeroed() }
    }
}

impl Rtc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RTC" {
            Some(Box::new(Rtc {
                isr: 0x0000_0007, prer: 0x007F_00FF, ssr: 0x0000_7FFF,
                tr: 0x0000_2100, dr: 0x0000_2101,
                last_inst: INSTRUCTION_COUNT.load(Ordering::Relaxed),
                ..Default::default()
            }))
        } else { None }
    }

    fn check_alarm(&mut self, sys: &System) {
        self.check_alarm_impl(sys);
    }

    /// Harness = the tamper pin: latch TAMP1F (ISR bit 13). Fires IRQ 2
    /// when TAMPIE (TAFCR bit 2) is set. Cleared by writing ISR bit 13
    /// (firmware's usual clear path).
    pub fn tamper(&mut self, sys: &System) {
        self.isr |= 1 << 13; // TAMP1F
        if self.tafcr & (1 << 2) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(2); // TAMP_STAMP IRQ
        }
    }

    /// Harness = the timestamp pin event: capture current TR/DR/SSR into
    /// TSTR/TSDR/TSSSR and latch TSF (ISR bit 11). On a second event while
    /// TSF is still set, latch TSOVF (bit 12) instead of overwriting
    /// (silicon overrun). Fires IRQ 2 when TSIE (CR bit 15) is set.
    pub fn timestamp(&mut self, sys: &System) {
        if self.isr & (1 << 11) != 0 {
            self.isr |= 1 << 12; // TSOVF: previous stamp unread
            return;
        }
        self.tstr = self.tr;
        self.tsdr = self.dr;
        self.tsssr = self.ssr;
        self.isr |= 1 << 11; // TSF
        if self.cr & (1 << 15) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(2);
        }
    }

    fn check_alarm_impl(&mut self, sys: &System) {        let alra_enabled = self.cr & (1 << 8) != 0; // ALRAE
        let alrb_enabled = self.cr & (1 << 9) != 0; // ALRBE
        if !alra_enabled && !alrb_enabled { return; }

        let irq = 41; // RTC_Alarm (ALRAF, ALRBF) — F407 NVIC IRQ 41

        if alra_enabled && self.isr & (1 << 8) != 0 { // ALRAF already set
            if self.cr & (1 << 12) != 0 { // ALRAIE
                sys.p.nvic.borrow_mut().set_intr_pending(irq);
            }
            return;
        }

        if alrb_enabled && self.isr & (1 << 9) != 0 { // ALRBF already set
            if self.cr & (1 << 13) != 0 { // ALRBIE
                sys.p.nvic.borrow_mut().set_intr_pending(irq);
            }
            return;
        }

        // Compare alarm with current time
        if alra_enabled && !alrb_enabled {
            if bcd_match(self.tr, self.alrmar, self.alrmar) {
                self.isr |= 1 << 8; // ALRAF
                if self.cr & (1 << 12) != 0 { // ALRAIE
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }

        if alrb_enabled && !alra_enabled {
            if bcd_match(self.tr, self.alrmbr, self.alrmbr) {
                self.isr |= 1 << 9; // ALRBF
                if self.cr & (1 << 13) != 0 { // ALRBIE
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }

        if alra_enabled && alrb_enabled {
            let alra_match = bcd_match(self.tr, self.alrmar, self.alrmar);
            let alrb_match = bcd_match(self.tr, self.alrmbr, self.alrmbr);
            if alra_match {
                self.isr |= 1 << 8;
                if self.cr & (1 << 12) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
            if alrb_match {
                self.isr |= 1 << 9;
                if self.cr & (1 << 13) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }

    fn advance_time(&mut self, sys: &System) {
        if (self.isr & 1) == 0 {
            let now = INSTRUCTION_COUNT.load(Ordering::Relaxed);
            let elapsed = now.wrapping_sub(self.last_inst);
            if elapsed > 100 {
                let async_prer = (self.prer >> 16) & 0x7F;
                let sync_prer = self.prer & 0x7FFF;
                let ticks_per_sec = (async_prer + 1) * (sync_prer + 1);
                let secs = elapsed / ticks_per_sec.max(1) as u64;
                if secs > 0 {
                    for _ in 0..secs.min(100) {
                        self.tr = bcd_inc_tr(self.tr);
                        self.tick_second(sys);
                        self.check_alarm(sys);
                    }
                    self.last_inst = now;
                }
            }
        }
    }

    /// One virtual second elapsed: sub-second reload, wakeup countdown,
    /// and smooth calibration accumulate.
    fn tick_second(&mut self, sys: &System) {
        // Wakeup timer: counts down while WUTE (CR bit 10) is set; at zero
        // latch WUTF (ISR bit 10), reload, and pend IRQ 2 when WUTIE
        // (CR bit 14) is set. Reload value comes from the last WUTR write
        // (WUTWF gating is a firmware sequencing detail — the model arms
        // on the write itself, like the alarm path arms on ALRMAR).
        if self.cr & (1 << 10) != 0 {
            if !self.wut_armed {
                self.wut_count = self.wut_reload;
                self.wut_armed = true;
            } else if self.wut_count > 0 {
                self.wut_count -= 1;
            }
            if self.wut_armed && self.wut_count == 0 {
                self.isr |= 1 << 10; // WUTF
                self.wut_count = self.wut_reload;
                if self.cr & (1 << 14) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(2);
                }
            }
        } else {
            self.wut_armed = false;
        }
        // Smooth calibration (CALR): CALM[8:0] pulses added (+CALP) or
        // masked (−) over each 512-second window. The model applies the
        // net rate directly: effective seconds per window = 512 + CALP −
        // CALM — by stretching this second 1/(512+net) of the time is
        // overkill; instead accumulate the deficit and skip one TR second
        // per window when net is negative (CALM-only, the common case).
        // Positive net (CALP set, CALM=0) adds a second per window.
        let calm = (self.calr & 0x1FF) as i32;
        let calp = if self.calr & (1 << 15) != 0 { 512 } else { 0 };
        let net = calp - calm;
        if net != 0 {
            let (add, skip) = cal_accum_add(net as i64);
            if add {
                self.tr = bcd_inc_tr(self.tr); // extra second this window
            } else if skip {
                // Skip this second: hold last_inst back one second worth
                // so the next second takes twice as long (the observable
                // long-run rate is what firmware checks).
                self.last_inst = self.last_inst.wrapping_add(
                    (((self.prer >> 16) & 0x7F) + 1) as u64
                        * (((self.prer & 0x7FFF) + 1) as u64),
                );
            }
        }
    }
}

impl Peripheral for Rtc {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        self.advance_time(sys);
        match offset {
            0x00 => self.tr, 0x04 => self.dr, 0x08 => self.cr, 0x0C => self.isr,
            0x10 => self.prer, 0x14 => self.wutr, 0x18 => self.calibr,
            0x1C => self.alrmar, 0x20 => self.alrmbr, 0x24 => self.wpr,
            0x28 => self.ssr, 0x2C => self.shiftr, 0x30 => self.tstr,
            0x34 => self.tsdr, 0x38 => self.tsssr, 0x3C => self.calr,
            0x40 => self.tafcr, 0x44 => self.alrmassr, 0x48 => self.alrmbssr,
            0x50..=0x9C => {
                let idx = ((offset - 0x50) / 4) as usize;
                if idx < 20 { self.bkp[idx] } else { 0 }
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.tr = value,
            0x04 => self.dr = value,
            0x08 => {
                self.cr = value;
                self.check_alarm(sys);
            }
            0x0C => {
                self.isr = value;
                self.check_alarm(sys);
            }
            0x10 => self.prer = value,
            0x14 => {
                self.wutr = value & 0xFFFF;
                // Arming: reload the countdown from the written value
                // (firmware writes WUTR then sets WUTE; the count starts
                // at the programmed value on the next second).
                self.wut_reload = (value & 0xFFFF) as u16;
                self.wut_armed = false; // re-arm on next tick
            }
            0x18 => self.calibr = value,
            0x1C => {
                self.alrmar = value;
                self.check_alarm(sys);
            }
            0x20 => {
                self.alrmbr = value;
                self.check_alarm(sys);
            }
            0x24 => self.wpr = value,
            0x28 => self.ssr = value,
            0x2C => self.shiftr = value,
            0x30 => self.tstr = value,
            0x34 => self.tsdr = value,
            0x38 => self.tsssr = value,
            0x3C => self.calr = value,
            0x40 => self.tafcr = value,
            0x44 => self.alrmassr = value,
            0x48 => self.alrmbssr = value,
            0x50..=0x9C => {
                let idx = ((offset - 0x50) / 4) as usize;
                if idx < 20 { self.bkp[idx] = value; }
            }
            _ => {}
        }
    }
}
