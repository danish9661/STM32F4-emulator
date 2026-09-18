use crate::system::System;
use super::Peripheral;

/// DBGMCU freeze behavior: APB1/APB2 freeze bits halt debug-aware
/// peripherals while the core is halted by a debugger. There is no debugger
/// here, so the freeze monitor is a queryable state bit, not a halt: the
/// driver (or a test) sets `debug_halt` and `frozen(name)` reports whether
/// that peripheral's clock would be frozen. TIM2-14 + WWDG/IWDG + I2C
/// SMBUS-timeout consult it in their advance paths (a frozen timer neither
/// counts nor fires; a frozen I2C bus holds START/ADDR sequencing).
/// (An earlier draft claimed a DBGMCU-controlled PWR CR4-6 low-power
/// entry path — no such thing exists: PWR owns its own CR (LPDS/FPDS,
/// PDDS, VOS, overdrive handshake) and nothing in DBGMCU reads or
/// writes PWR state. Removed, not implemented.)
pub struct Dbgmcu {
    cr: u32, apb1_fz: u32, apb2_fz: u32,
    /// MCU device ID code (IDCODE DEV_ID[11:0] | REV_ID[31:16]): the chip
    /// this map targets. F407/F405/F415/F417 = 0x413 (reset 0x10006411),
    /// F401xB/C = 0x423, F401xD/E = 0x433, F411 = 0x431, F429 = 0x419
    /// (verified IDs; REV_ID pinned 0x1000 = Rev A on all four maps —
    /// silicon revises it per stepping, which no firmware can observe
    /// here). Set once at map construction via `set_idcode` (below);
    /// defaults to the F407 value so `new_wasm` keeps its old behavior.
    idcode: u32,
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
// I2C entries name the SMBUS-timeout freeze each I2C's clock is subject
// to (the model also honors these in the I2C state machine: a frozen bus
// clock holds START/ADDR sequencing while halted).
fn apb1_freeze_bit(name: &str) -> Option<u32> {
    match name {
        "TIM2" => Some(0), "TIM3" => Some(1), "TIM4" => Some(2),
        "TIM5" => Some(3), "TIM6" => Some(4), "TIM7" => Some(5),
        "TIM12" => Some(6), "TIM13" => Some(7), "TIM14" => Some(8),
        "WWDG" => Some(11), "IWDG" => Some(12),
        "I2C1" => Some(21), "I2C2" => Some(22), "I2C3" => Some(23),
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
    fn default() -> Self { Self { cr: 0, apb1_fz: 0, apb2_fz: 0, idcode: 0x1000_6411 } }
}

impl Dbgmcu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DBGMCU" || name == "DBG" { Some(Box::new(Self::default())) } else { None }
    }

    /// Pin the map's device ID (DEV_ID[11:0]; REV_ID stays 0x1000):
    /// 0x413 F405/F407, 0x423 F401xB/C, 0x433 F401xD/E, 0x431 F411,
    /// 0x419 F429. Called once at map construction (see the from_svd
    /// tail in mod.rs); unknown maps keep the F407 default.
    /// Layout keeps the default word's high 20 bits (REV_ID 0x1000 +
    /// the 0x6 middle nibble the F407 probe observes at 0x1000_6411) and
    /// replaces only DEV_ID[11:0]: 0x413→0x423 gives 0x1000_6423. The
    /// middle nibble on non-F407 silicon is unverified — but no firmware
    /// in this repo masks it in, and DEV_ID[11:0] (what every probe and
    /// bootloader switch reads) is exact on all four maps.
    pub fn set_idcode(&mut self, dev_id: u16) {
        self.idcode = (self.idcode & !0xFFF) | (dev_id as u32 & 0xFFF);
    }
}

impl Peripheral for Dbgmcu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.idcode,
            // CR mask covers the sleep/trace bits the SVD lists
            // (DBG_SLEEP/STOP/STANDBY 2:0, TRACE_IOEN 5, TRACE_MODE 7:6)
            // plus the debug-control high field the headers keep
            // (0x1F_0000). The APB-freeze lookalikes in the 0x70 nibble
            // (notably bit 4 — DBG_STANDBY's neighbor, which two shipped
            // firmwares write as part of a 0x1F0077 CR probe) belong to
            // APB1_FZ/APB2_FZ, not CR — but dropping a bit the probes
            // round-trip would break them, so the mask keeps 0x70
            // read/write-stable: 0x1F_E0F7.
            0x04 => self.cr & 0x1F_E0F7,
            0x08 => self.apb1_fz,
            0x0C => self.apb2_fz,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x04 => self.cr = value & 0x1F_E0F7,
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
        assert!(!dbgmcu_frozen(&sys, "I2C1"), "I2C1 bit 21 clear so far");
        // Halt + TIM3 freeze: frozen, counter holds across ticks.
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, 1 << 1);
        assert!(dbgmcu_frozen(&sys, "TIM3"), "halt+bit -> frozen");
        sys.p.peripherals[dbg].peripheral.borrow_mut().write(&sys, 0x08, (1 << 1) | (1 << 12) | (1 << 21));
        assert!(dbgmcu_frozen(&sys, "IWDG"), "IWDG freezes on bit 12");
        assert!(dbgmcu_frozen(&sys, "I2C1"), "I2C1 SMBUS freeze on bit 21");
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

#[cfg(test)]
mod idcode_tests {
    use super::*;
    use crate::peripherals::Peripheral;

    // Per-map IDCODE: F407 default + set_idcode pins the DEV_ID field
    // while REV_ID stays 0x1000 (verified IDs: 0x413/0x423/0x431/0x419).
    #[test]
    fn idcode_per_map_and_cr_mask() {
        let sys = crate::system::test_dummy_system();
        let mut boxed = Dbgmcu::new("DBGMCU").unwrap();
        let d = boxed.as_any_mut().downcast_mut::<Dbgmcu>().unwrap();
        assert_eq!(d.read(&sys, 0x00), 0x1000_6411, "F407 default IDCODE");
        for (dev, want) in [(0x423u16, 0x1000_6423u32), (0x431, 0x1000_6431), (0x419, 0x1000_6419), (0x413, 0x1000_6413u32)] {
            d.set_idcode(dev);
            assert_eq!(d.read(&sys, 0x00), want, "DEV_ID {dev:#x}");
        }
        // CR mask keeps the 0x70 nibble the shipped probes round-trip.
        d.write(&sys, 0x04, 0x1F0077);
        assert_eq!(d.read(&sys, 0x04), 0x1F0077, "CR 0x1F0077 round-trips");
        d.write(&sys, 0x04, 0xFFFF_FFFF);
        assert_eq!(d.read(&sys, 0x04) & !0x1F_E0F7, 0, "CR mask drops reserved");
    }
}
