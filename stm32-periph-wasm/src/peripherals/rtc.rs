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
    // mask_bits: bit 7 of each byte = don't care
    let sec_match = if alarm_bcd & (1 << 7) != 0 { true } else { (tr_bcd & 0x7F) == (alarm_bcd & 0x7F) };
    let min_match = if alarm_bcd & (1 << 15) != 0 { true } else { ((tr_bcd >> 8) & 0x7F) == ((alarm_bcd >> 8) & 0x7F) };
    let hr_match = if alarm_bcd & (1 << 23) != 0 { true } else { ((tr_bcd >> 16) & 0x3F) == ((alarm_bcd >> 16) & 0x3F) };
    sec_match && min_match && hr_match
}

pub struct Rtc {
    tr: u32, dr: u32, cr: u32, isr: u32, prer: u32, wutr: u32,
    calibr: u32, alrmar: u32, alrmbr: u32, wpr: u32, ssr: u32,
    shiftr: u32, tstr: u32, tsdr: u32, tsssr: u32, calr: u32,
    tafcr: u32, alrmassr: u32, alrmbssr: u32, bkp: [u32; 20],
    last_inst: u64,
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
                        self.check_alarm(sys);
                    }
                    self.last_inst = now;
                }
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
            0x14 => self.wutr = value,
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
