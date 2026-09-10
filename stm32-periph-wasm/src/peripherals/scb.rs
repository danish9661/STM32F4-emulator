use crate::system::System;
use super::Peripheral;

pub struct Scb {
    vtor: u32,       // 0x08
    icsr: u32,       // 0x04
    aircr: u32,      // 0x0C
    scr: u32,        // 0x10
    ccr: u32,        // 0x14
    shpr: [u32; 3],  // 0x18-0x20 system handler priorities (SHPR1-SHPR3)
    shcsr: u32,      // 0x24
    cfsr: u32,       // 0x28
    hfsr: u32,       // 0x2C
    dfsr: u32,       // 0x30
    mmfar: u32,      // 0x34
    bfar: u32,       // 0x38
    afsr: u32,       // 0x3C
    cpacr: u32,      // 0x88 (0x0 in the SVD's split FPU_CPACR slot)
    /// The SVD splits CPACR into its own `FPU_CPACR` peripheral at
    /// 0xE000ED88; that slot is a CPACR-only view (offset 0 == CPACR).
    cpacr_slot: bool,
}

impl Default for Scb {
    fn default() -> Self {
        Self {
            vtor: 0x0800_0000,
            aircr: 0xFA05_0000,
            shcsr: 0x0000_0000,
            ..unsafe { std::mem::zeroed() }
        }
    }
}

impl Scb {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SCB" || name == "SCB_Trusted" {
            Some(Box::new(Self::default()))
        } else if name == "FPU_CPACR" {
            Some(Box::new(Self { cpacr_slot: true, ..Self::default() }))
        } else {
            None
        }
    }

    pub fn vtor(&self) -> u32 { self.vtor }

    fn write_aircr(&mut self, value: u32) {
        // VECTKEY lives in the HIGH halfword (bits 31:16); the old gate
        // checked the LOW half, so no AIRCR write ever applied (PRIGROUP
        // stuck at reset 0, SYSRESETREQ dead).
        if (value >> 16) & 0xFFFF == 0x05FA {
            // Keep only the writable low bits (PRIGROUP [10:8]); the high
            // half always reads VECTKEYSTAT (0xFA05). The old mask kept the
            // high halfword instead, which silently dropped PRIGROUP even
            // past a correct key (second half of the dead-AIRCR bug).
            self.aircr = (0xFA05 << 16) | (value & 0x0000_0F00);
            let vectkey = (value >> 16) & 0xFFFF;
            if vectkey == 0x05FA {
                let sysreset = (value >> 2) & 1;
                if sysreset == 1 {
                    // System reset request (AIRCR SYSRESETREQ) — reboot the
                    // guest but don't latch a watchdog-specific cause bit.
                    crate::system::request_watchdog_reset(0);
                }
            }
        }
    }

    fn write_icsr(&mut self, value: u32, sys: &System) {
        use crate::peripherals::nvic::irq;
        // Set-pending
        if value & (1 << 28) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(irq::PENDSV);
        }
        if value & (1 << 26) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(irq::SYSTICK);
        }
        // Clear-pending
        if value & (1 << 25) != 0 {
            sys.p.nvic.borrow_mut().clear_pending(irq::SYSTICK);
        }
        if value & (1 << 27) != 0 {
            sys.p.nvic.borrow_mut().clear_pending(irq::PENDSV);
        }
        self.icsr = (self.icsr & 0xE01F_FFFF) | (value & 0x1FE0_0000) | (value & 0x1FF);
    }
}

impl Peripheral for Scb {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        if self.cpacr_slot {
            return match offset {
                0 => self.cpacr,
                _ => 0,
            };
        }
        match offset {
            0x00 => {
                // CPUID - r0p1 of Cortex-M4
                let implementer = 0x41; // ARM
                let variant = 0;
                let part = 0xC24; // Cortex-M4
                let revision = 1;
                (implementer << 24) | (variant << 20) | (part << 4) | revision
            }
            0x04 => {
                // ICSR: current pending vector, etc.
                let mut v = self.icsr & 0xE01F_FFFF;
                v |= (_sys.p.nvic.borrow().get_pending_vector()) << 16;
                v
            }
            0x08 => self.vtor,
            0x0C => self.aircr,
            0x10 => self.scr,
            0x14 => self.ccr,
            0x18 => self.shpr[0],
            0x1C => self.shpr[1],
            0x20 => self.shpr[2],
            0x24 => self.shcsr,
            0x28 => self.cfsr,
            0x2C => self.hfsr,
            0x30 => self.dfsr,
            0x34 => self.mmfar,
            0x38 => self.bfar,
            0x3C => self.afsr,
            // 0x40-0x84 reserved
            0x88 => self.cpacr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        if self.cpacr_slot {
            if offset == 0 {
                self.cpacr = value & 0x00F0_0000;
            }
            return;
        }
        match offset {
            0x04 => self.write_icsr(value, sys),
            0x08 => self.vtor = value & 0xFFFF_FC00,
            0x0C => self.write_aircr(value),
            0x10 => self.scr = value & 0x1E,
            0x14 => self.ccr = value & 0xFFFF,
            0x18 => self.shpr[0] = value,
            0x1C => self.shpr[1] = value,
            0x20 => self.shpr[2] = value,
            0x24 => self.shcsr = value & 0x0007_FFFF,
            0x28 => self.cfsr = value & 0xFFFF_FFFF,
            0x2C => self.hfsr = value & 0x7FFF,
            0x30 => self.dfsr = value & 0xFFFF,
            0x34 => self.mmfar = value,
            0x38 => self.bfar = value,
            0x3C => self.afsr = value,
            0x88 => self.cpacr = value & 0x00F0_0000,
            _ => {}
        }
    }
}
