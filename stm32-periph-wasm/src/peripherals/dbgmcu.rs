use crate::system::System;
use super::Peripheral;

/// DBGMCU freeze behavior: APB1/APB2 freeze bits halt debug-aware
/// peripherals while the core is halted by a debugger. There is no debugger
/// here, so the freeze monitor is a queryable state bit, not a halt: the
/// driver (or a test) sets `debug_halt` and `frozen(name)` reports whether
/// that peripheral's clock would be frozen. TIM2-14 + WWDG/IWDG consult it
/// in their advance paths (a frozen timer neither counts nor fires).
pub struct Dbgmcu {
    cr: u32, apb1_fz: u32, apb2_fz: u32,
}

/// Process-wide debug-halt flag: true while a debugger holds the core.
/// The emulator has no debugger; tests set/clear it via
/// `dbgmcu_set_halt` and `reset_globals` clears it.
static DEBUG_HALT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Set (or clear) the debug-halt state.
pub fn dbgmcu_set_halt(halt: bool) {
    DEBUG_HALT.store(halt, std::sync::atomic::Ordering::Relaxed);
}

/// True while a debugger holds the core halted.
pub fn dbgmcu_halted() -> bool {
    DEBUG_HALT.load(std::sync::atomic::Ordering::Relaxed)
}

// Freeze-bit positions (RM0090 §38.16). APB1_FZ covers TIM2-7/12-14,
// WWDG, IWDG, I2C1-3 SMBUS timeouts, CAN1/2. APB2_FZ covers TIM1/8-11.
fn apb1_freeze_bit(name: &str) -> Option<u32> {
    match name {
        "TIM2" => Some(0), "TIM3" => Some(1), "TIM4" => Some(2),
        "TIM5" => Some(3), "TIM6" => Some(4), "TIM7" => Some(5),
        "TIM12" => Some(6), "TIM13" => Some(7), "TIM14" => Some(8),
        "WWDG" => Some(11), "IWDG" => Some(12),
        "CAN1" => Some(25), "CAN2" => Some(26),
        _ => None,
    }
}

fn apb2_freeze_bit(name: &str) -> Option<u32> {
    match name {
        "TIM1" => Some(0), "TIM8" => Some(1),
        "TIM9" => Some(16), "TIM10" => Some(17), "TIM11" => Some(18),
        _ => None,
    }
}

/// Would `name`'s clock be frozen right now: halted AND its freeze bit set.
pub fn dbgmcu_frozen(sys: &System, name: &str) -> bool {
    if !dbgmcu_halted() {
        return false;
    }
    for slot in &sys.p.peripherals {
        if slot.start == 0xE004_2000 {
            let mut b = slot.peripheral.borrow_mut();
            if let Some(d) = b.as_any_mut().downcast_mut::<Dbgmcu>() {
                if let Some(bit) = apb1_freeze_bit(name) {
                    if d.apb1_fz & (1 << bit) != 0 {
                        return true;
                    }
                }
                if let Some(bit) = apb2_freeze_bit(name) {
                    if d.apb2_fz & (1 << bit) != 0 {
                        return true;
                    }
                }
                return false;
            }
            break;
        }
    }
    false
}

impl Default for Dbgmcu {
    fn default() -> Self { Self { cr: 0, apb1_fz: 0, apb2_fz: 0 } }
}

impl Dbgmcu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DBGMCU" || name == "DBG" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Dbgmcu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => 0x10006411,
            0x04 => self.cr & 0x1F_0077,
            0x08 => self.apb1_fz,
            0x0C => self.apb2_fz,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x04 => self.cr = value & 0x1F_0077,
            0x08 => self.apb1_fz = value,
            0x0C => self.apb2_fz = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::Peripheral;

    // Freeze is queryable state, not a halt: with no halt, even a set
    // freeze bit changes nothing; with halt + TIM3 freeze, TIM3 neither
    // counts nor fires, and resume doesn't burst.
    #[test]
    fn dbgmcu_freeze_stops_tim3_without_burst() {
        let sys = crate::system::test_dummy_system();
        // Find both slots.
        let (mut dbg, mut tim): (Option<usize>, Option<usize>) = (None, None);
        for (i, s) in sys.p.peripherals.iter().enumerate() {
            if s.start == 0xE004_2000 { dbg = Some(i); }
            if s.start == 0x4000_0400 { tim = Some(i); }
        }
        let (dbg, tim) = (dbg.expect("dbg"), tim.expect("tim"));
        // Freeze bit set but no halt: timer runs normally.
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, 1 << 1);
        assert!(!dbgmcu_frozen(&sys, "TIM3"), "no halt -> not frozen");
        // Halt without a freeze bit for TIM3 (freeze TIM2 instead): runs.
        // (TIM2's bit alone must not freeze TIM3; IWDG shares APB1 so it
        // stays clear here and is asserted separately below.)
        dbgmcu_set_halt(true);
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, 1 << 0);
        assert!(!dbgmcu_frozen(&sys, "TIM3"), "TIM2 bit != TIM3");
        assert!(!dbgmcu_frozen(&sys, "IWDG"), "IWDG bit 12 clear so far");
        // Halt + TIM3 freeze: frozen, counter holds across ticks.
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, 1 << 1);
        assert!(dbgmcu_frozen(&sys, "TIM3"), "halt+bit -> frozen");
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, (1 << 1) | (1 << 12));
        assert!(dbgmcu_frozen(&sys, "IWDG"), "IWDG freezes on bit 12");
        // TIM3 enabled, ARR far away: ticks while frozen must not advance.
        sys.p.peripherals[tim].peripheral.borrow_mut().write(&sys, 0x00, 1);
        sys.p.peripherals[tim].peripheral.borrow_mut().write(&sys, 0x2C, 0xFFFF);
        crate::system::INSTRUCTION_COUNT.fetch_add(50_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals[tim].peripheral.borrow_mut().tick(&sys);
        let c0 = sys.p.peripherals[tim].peripheral.borrow_mut().read(&sys, 0x24);
        assert_eq!(c0, 0, "frozen counter holds at 0");
        // Resume: no catch-up burst (counter advances only fresh ticks).
        dbgmcu_set_halt(false);
        crate::system::INSTRUCTION_COUNT.fetch_add(2_000, std::sync::atomic::Ordering::Relaxed);
        sys.p.peripherals[tim].peripheral.borrow_mut().tick(&sys);
        let c1 = sys.p.peripherals[tim].peripheral.borrow_mut().read(&sys, 0x24);
        assert!(c1 > 0 && c1 < 50_000, "resume advances fresh ticks only, got {c1}");
        dbgmcu_set_halt(false);
    }
}
