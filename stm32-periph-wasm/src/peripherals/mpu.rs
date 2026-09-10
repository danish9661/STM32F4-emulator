use crate::system::System;
use super::Peripheral;

/// MPU (SVD `MPU` peripheral at 0xE000ED90). Region programming is stored
/// faithfully (TYPE/RNR/RBAR/RASR read back what a bring-up sequence
/// writes), but protection is NOT enforced: instead, setting CTRL.ENABLE
/// latches a model sticky (`set_mpu_enabled`) that halts the driver with a
/// clear message. Continuing to run unprotected would be silently wrong
/// (MemManage faults would never fire); a loud halt names the gap.
/// No shipped firmware enables the MPU, so this path is guest-opt-in only.
pub struct Mpu {
    ctrl: u32,          // +0x4 (ENABLE/HFNMIENA/PRIVDEFENA only)
    rnr: u32,           // +0x8
    rbar: [u32; 8],     // +0xC (per selected region)
    rasr: [u32; 8],     // +0x10
}

impl Default for Mpu {
    fn default() -> Self {
        Self { ctrl: 0, rnr: 0, rbar: [0; 8], rasr: [0; 8] }
    }
}

impl Mpu {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "MPU" { Some(Box::new(Self::default())) } else { None }
    }

    fn region(&self) -> usize {
        (self.rnr & 7) as usize
    }
}

impl Peripheral for Mpu {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            // TYPE: unified map, 8 data + 8 instruction regions (M4F).
            0x0 => 0x0008_0800,
            0x4 => self.ctrl,
            0x8 => self.rnr & 7,
            0xC => self.rbar[self.region()],
            0x10 => self.rasr[self.region()],
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x4 => {
                self.ctrl = value & 0x7;
                crate::system::set_mpu_enabled(value & 1 != 0);
            }
            0x8 => self.rnr = value & 7,
            0xC => {
                let r = self.region();
                self.rbar[r] = value;
            }
            0x10 => {
                let r = self.region();
                self.rasr[r] = value;
            }
            _ => {}
        }
    }
}
