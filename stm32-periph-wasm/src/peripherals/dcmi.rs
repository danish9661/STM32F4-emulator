use crate::system::System;
use super::Peripheral;

const DCMI_IRQ: i32 = 78;

pub struct Dcmi {
    cr: u32, sr: u32, ris: u32, ier: u32,
    escr: u32, esur: u32, cwstrt: u32, cwsiz: u32, dr: u32,
    pattern: u32, vsync: bool, hsync: bool,
}

impl Default for Dcmi {
    fn default() -> Self {
        Self {
            cr: 0, sr: 0, ris: 0, ier: 0,
            escr: 0, esur: 0, cwstrt: 0, cwsiz: 0, dr: 0,
            pattern: 0, vsync: false, hsync: false,
        }
    }
}

impl Dcmi {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DCMI" { Some(Box::new(Self::default())) } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.ris & self.ier != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(DCMI_IRQ);
        }
    }
}

impl Peripheral for Dcmi {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => {
                if self.cr & 1 != 0 { self.sr |= 4; }
                else { self.sr &= !4; }
                self.sr
            }
            0x08 => self.ris,
            0x0C => self.ier,
            0x10 => { let v = self.ris; self.ris = 0; v }
            0x14 => self.escr,
            0x18 => self.esur,
            0x1C => self.cwstrt,
            0x20 => self.cwsiz,
            0x28 => {
                if self.cr & 1 != 0 {
                    self.sr |= 4;
                    self.pattern = self.pattern.wrapping_add(0x01020304);
                    self.dr = self.pattern;
                    self.ris |= 1 << 2;
                    self.fire_interrupts(sys);
                }
                self.dr
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr = value & 0x7FFF_3FFF,
            0x0C => {
                self.ier = value & 0x1F;
                self.fire_interrupts(sys);
            }
            0x10 => {
                self.ris &= !(value & 0x1F);
            }
            0x14 => self.escr = value & 0x3FF,
            0x18 => self.esur = value & 0xFF_FFFF,
            0x1C => self.cwstrt = value & 0x3FFF,
            0x20 => self.cwsiz = value & 0x3FFF,
            _ => {}
        }
    }
}
