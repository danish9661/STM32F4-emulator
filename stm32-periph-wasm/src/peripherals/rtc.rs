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

/// BCD TR minus one second (SHIFTR borrow path): 00 seconds borrows a
/// minute, 00:00 borrows an hour (24h wrap). Date fields untouched —
/// the model's time is second-granularity (see apply_shiftr).
fn bcd_dec_tr(tr: u32) -> u32 {
    let sec = (tr >> 0) & 0x7F;
    let min = (tr >> 8) & 0x7F;
    let hr = (tr >> 16) & 0x3F;
    let to_bin = |b: u32| ((b >> 4) * 10) + (b & 0xF);
    let to_bcd = |v: u32| (((v / 10) << 4) | (v % 10)) & 0x7F;
    let (s, m, h) = (to_bin(sec), to_bin(min), to_bin(hr & 0x3F));
    let (ns, nm, nh) = if s > 0 {
        (s - 1, m, h)
    } else if m > 0 {
        (59, m - 1, h)
    } else {
        (59, 59, if h > 0 { h - 1 } else { 23 })
    };
    ((tr & 0xFF00_0000) | ((to_bcd(nh) & 0x3F) << 16) | (to_bcd(nm) << 8) | to_bcd(ns))
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
/// the shared virtual clock, so one accumulator is correct. Module scope
/// (not function-local) so reset_globals can drain it between instances.
use std::sync::OnceLock;
static CAL_ACC: OnceLock<std::sync::Mutex<i64>> = OnceLock::new();
fn cal_accum() -> &'static std::sync::Mutex<i64> {
    CAL_ACC.get_or_init(|| std::sync::Mutex::new(0))
}
fn cal_accum_add(net: i64) -> (bool, bool) {
    let mut acc = cal_accum().lock().unwrap();
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

/// Drain the calibration accumulator (fresh-instance hygiene; called by
/// reset_globals so a partially-filled 512 s window never leaks across
/// emulator instances).
pub fn cal_accum_clear() {
    *cal_accum().lock().unwrap() = 0;
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
    /// ALRMASSR/ALRMBSSR programmed flags: the sub-second alarm gate
    /// only applies once firmware writes the register (see ss_match).
    ssa_set: bool,
    ssb_set: bool,
    /// Tamper-pin physics state: last pin level + consecutive-match filter
    /// count (see `tamper_pin`). Reset values: pull-up idle high.
    tamp_level: bool,
    tamp_filter: u32,
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
                tamp_level: true,
                ..Default::default()
            }))
        } else { None }
    }

    fn check_alarm(&mut self, sys: &System) {
        self.check_alarm_impl(sys);
    }

    /// SHIFTR (offset 0x2C) write semantics (RM0090 §26.3.7): the write
    /// applies atomically once per APB cycle — SUBFS (bits 14:0) seconds-
    /// fractions are SUBTRACTED from SSR, with borrow into TR when SSR
    /// underflows (one second less); ADD1S (bit 31) then ADDS one second
    /// to TR/calendar (applied after the subtract, so ADD1S+SUBFS = net
    /// shift of 1s − SUBFS fractions). SHPF (ISR bit 3) latches while the
    /// shift is pending and clears when it completes — the model applies
    /// the shift synchronously and leaves SHPF clear (no pending window
    /// is observable on the instruction clock). RSF (ISR bit 5) latches:
    /// the calendar shadow just changed outside the normal tick.
    /// No-op when the calendar is initializing (ISR bit 7 INIT set):
    /// silicon rejects shifts during init mode.
    fn apply_shiftr(&mut self, sys: &System, value: u32) {
        self.shiftr = value;
        if self.isr & (1 << 7) != 0 {
            return; // init mode: shift rejected
        }
        let subfs = value & 0x7FFF;
        let ssr = self.ssr & 0x7FFF;
        if subfs != 0 {
            if subfs <= ssr {
                self.ssr = (self.ssr & !0x7FFF) | (ssr - subfs);
            } else {
                // Borrow: SSR wraps within its PREDIV_S range and the
                // calendar loses one second (TR decrements by one BCD
                // second; date borrow is out of scope — seconds field
                // only, matching the model's second-granularity time).
                let pred = (self.prer & 0x7FFF) + 1;
                self.ssr = (self.ssr & !0x7FFF) | ((pred + ssr - subfs) & 0x7FFF);
                self.tr = bcd_dec_tr(self.tr);
            }
        }
        if value & (1 << 31) != 0 {
            self.tr = bcd_inc_tr(self.tr);
        }
        self.isr &= !(1 << 3); // SHPF: no pending window
        self.isr |= 1 << 5; // RSF: shadow changed
        let _ = sys;
    }

    /// ALRMASSR/ALRMBSSR (offsets 0x44/0x48) sub-second alarm match:
    /// MASKSS (bits 27:24) selects how many SS field bits (14:0) must
    /// match SSR for the alarm to fire — 0 = all 15 bits, N = top 15−N
    /// bits (silicon masks the low N bits). The second-level alarm gate
    /// (`ss_match`) is ANDed with the TR date match in check_alarm_impl:
    /// with MASKSS=15 (all masked) the sub-second gate passes always.
    /// RESET VALUE: ALRMASSR resets to 0 (MASKSS=0, SS=0 — an exact-SS=0
    /// gate). Firmware that never programs ALRMASSR must NOT be gated by
    /// a stale SSR: the gate is only enforced once firmware WRITES the
    /// register (silicon's reset SS field only matters against a running
    /// sub-second counter; the model's SSR sits at reset 0x7FFF, which
    /// would spuriously block every never-programmed alarm). Tracked by
    /// `ssa_set`/`ssb_set`, armed by the write arms below.
    fn ss_match(alrmassr: u32, ssr: u32) -> bool {
        let maskss = ((alrmassr >> 24) & 0xF) as u32;
        if maskss >= 15 {
            return true;
        }
        let mask = !((1u32 << maskss) - 1) & 0x7FFF;
        ((alrmassr & 0x7FFF) & mask) == ((ssr & 0x7FFF) & mask)
    }

    /// Harness = the tamper pin: latch TAMP1F (ISR bit 13). Fires IRQ 2
    /// when TAMPIE (TAFCR bit 2) is set. Cleared by writing ISR bit 13
    /// (firmware's usual clear path).
    pub fn tamper(&mut self, sys: &System) {
        self.tamper_edge(sys);
    }

    /// Tamper-pin physics (TAFCR-gated): the pin event only fires when
    /// TAMP1E (bit 0) is set; TAMP1TRG (bit 1) selects rising (1) vs
    /// falling (0) edge — the harness `level` is the new pin level and an
    /// event fires only on the matching transition from the stored level.
    /// TAMPFLT (bits 12:11) demands N consecutive matching samples
    /// (0/2/4/8); TAMPPRCH (bits 14:13) + TAMPFREQ (bits 10:8) model the
    /// precharge/filter clock without timing it (the count of matching
    /// samples is the contract, not the RTCCLK cycles). A firing event
    /// clears all 20 backup registers (silicon erases secrets on tamper)
    /// and, with TAMPTS (bit 7), captures a timestamp first (TSF path —
    /// TSOVF if a stamp was already pending).
    pub fn tamper_pin(&mut self, sys: &System, level: bool) {
        // Disabled pin: store the level, never fire.
        if self.tafcr & 1 == 0 {
            self.tamp_level = level;
            return;
        }
        let rising_edge = level && !self.tamp_level;
        let falling_edge = !level && self.tamp_level;
        self.tamp_level = level;
        let want_rising = self.tafcr & (1 << 1) != 0;
        let edge_ok = if want_rising { rising_edge } else { falling_edge };
        // Filter (TAMPFLT): N consecutive SAMPLES at the assertive level
        // (not N edges — alternating high/low calls would reset an edge
        // counter every other sample and x4/x8 could never fire). The
        // first matching edge starts the run; subsequent samples at the
        // assertive level extend it; any sample away resets.
        let need = match (self.tafcr >> 11) & 3 {
            0 => 1,
            1 => 2,
            2 => 4,
            _ => 8,
        };
        let assertive = if want_rising { level } else { !level };
        if edge_ok {
            self.tamp_filter = 1;
        } else if assertive {
            self.tamp_filter += 1;
        } else {
            self.tamp_filter = 0;
        }
        if self.tamp_filter >= need {
            self.tamp_filter = 0;
            // Timestamp first when TAMPTS is set (silicon captures the
            // tamper instant), then erase secrets, then flag.
            if self.tafcr & (1 << 7) != 0 {
                self.timestamp(sys);
            }
            self.bkp = [0; 20];
            self.tamper_edge(sys);
        }
    }

    /// Raw tamper event (unconditional — the legacy `tamper()` path and
    /// the filtered `tamper_pin()` path converge here): latch TAMP1F +
    /// IRQ2 when TAMPIE.
    fn tamper_edge(&mut self, sys: &System) {
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

    fn check_alarm_impl(&mut self, sys: &System) {
        let alra_enabled = self.cr & (1 << 8) != 0; // ALRAE
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

        // Sub-second gate (ALRMASSR/ALRMBSSR MASKSS): ANDed with the TR
        // date match below — with MASKSS=15 the gate passes always.
        // Unprogrammed registers (reset 0) do NOT gate: firmware that
        // never writes ALRMASSR (deep_sleep_demo, standby_demo) must
        // fire on the TR match alone.
        let ssa_ok = if self.ssa_set { Self::ss_match(self.alrmassr, self.ssr) } else { true };
        let ssb_ok = if self.ssb_set { Self::ss_match(self.alrmbssr, self.ssr) } else { true };

        // Compare alarm with current time
        if alra_enabled && !alrb_enabled {
            if ssa_ok && bcd_match(self.tr, self.alrmar, self.alrmar) {
                self.isr |= 1 << 8; // ALRAF
                if self.cr & (1 << 12) != 0 { // ALRAIE
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }

        if alrb_enabled && !alra_enabled {
            if ssb_ok && bcd_match(self.tr, self.alrmbr, self.alrmbr) {
                self.isr |= 1 << 9; // ALRBF
                if self.cr & (1 << 13) != 0 { // ALRBIE
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }

        if alra_enabled && alrb_enabled {
            let alra_match = ssa_ok && bcd_match(self.tr, self.alrmar, self.alrmar);
            let alrb_match = ssb_ok && bcd_match(self.tr, self.alrmbr, self.alrmbr);
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
            0x2C => self.apply_shiftr(sys, value),
            0x30 => self.tstr = value,
            0x34 => self.tsdr = value,
            0x38 => self.tsssr = value,
            0x3C => self.calr = value,
            0x40 => self.tafcr = value,
            0x44 => {
                self.alrmassr = value;
                self.ssa_set = true; // gate now enforced (see ss_match)
                self.check_alarm(sys);
            }
            0x48 => {
                self.alrmbssr = value;
                self.ssb_set = true;
                self.check_alarm(sys);
            }
            0x50..=0x9C => {
                let idx = ((offset - 0x50) / 4) as usize;
                if idx < 20 { self.bkp[idx] = value; }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::INSTRUCTION_COUNT;
    use std::sync::atomic::Ordering;

    // SHIFTR SUBFS borrows one second when SSR underflows; ADD1S adds one.
    #[test]
    fn shiftr_subfs_borrows_and_add1s_adds() {
        let sys = crate::system::test_dummy_system();
        // Drive through a slot-bound instance instead (the leaked pointer
        // above is not slot-bound; rebuild properly here).
        let mut boxed = Rtc::new("RTC").unwrap();
        let r = boxed.as_any_mut().downcast_mut::<Rtc>().unwrap();
        r.write(&sys, 0x0C, 0); // exit init (clear INIT bit 7)
        r.write(&sys, 0x10, (0 << 16) | 999); // PRER: 1000 ticks/sec
        r.write(&sys, 0x00, 0x0012_3050); // TR 12:30:50
        r.write(&sys, 0x28, 100); // SSR=100 sub-seconds
        r.write(&sys, 0x2C, 200); // SUBFS=200 > SSR → borrow
        assert_eq!(r.tr & 0x7F, 0x49, "borrow: seconds 50→49, got {:#x}", r.tr & 0x7F);
        assert_eq!(r.isr & (1 << 5), 1 << 5, "RSF latches on shift");
        r.write(&sys, 0x2C, 1 << 31); // ADD1S
        assert_eq!(r.tr & 0x7F, 0x50, "ADD1S: seconds back to 50");
    }

    // ALRMASSR MASKSS gates the alarm on SSR: exact match fires, masked
    // mismatch blocks, MASKSS=15 passes regardless of SSR.
    #[test]
    fn alrmassr_maskss_gates_alarm() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Rtc::new("RTC").unwrap();
        let r = boxed.as_any_mut().downcast_mut::<Rtc>().unwrap();
        r.write(&sys, 0x0C, 0);
        r.write(&sys, 0x00, 0x0012_3050);
        r.write(&sys, 0x28, 0x100); // SSR
        r.write(&sys, 0x1C, 0x0012_3050 | (1 << 31) | (1 << 23) | (1 << 15) | (1 << 7)); // ALRMAR = TR, date masked
        r.write(&sys, 0x44, 0x100); // ALRMASSR SS=SSR, MASKSS=0 (all bits)
        r.write(&sys, 0x08, 1 << 8); // ALRAE
        assert_ne!(r.isr & (1 << 8), 0, "ALRAF with matching SSR");
        // Mismatch blocks.
        r.write(&sys, 0x0C, 0); // (clear ALRAF: ISR write path stores value)
        r.isr &= !(1 << 8);
        r.write(&sys, 0x28, 0x101); // SSR drifts by one
        r.write(&sys, 0x08, 1 << 8); // re-arm check
        assert_eq!(r.isr & (1 << 8), 0, "ALRAF blocked on SSR mismatch");
        // MASKSS=15: gate passes regardless.
        r.write(&sys, 0x44, (15 << 24) | 0);
        r.write(&sys, 0x08, 1 << 8);
        assert_ne!(r.isr & (1 << 8), 0, "MASKSS=15 passes any SSR");
    }
}
