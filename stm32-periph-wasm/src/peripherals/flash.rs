use crate::system::System;
use super::Peripheral;

#[derive(Default)]
pub struct Flash {
    acr: u32,
    keyr: u32,
    optkeyr: u32,
    sr: u32,
    cr: u32,
    optcr: u32,
    optcr1: u32,
    flash_locked: bool,
    cr_psize: u32,
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FLASH" || name == "FLASH_Trusted" { Some(Box::new(Self::default())) } else { None }
    }
}

impl Peripheral for Flash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.optkeyr,
            0x0C => self.sr,
            0x10 => self.cr,
            0x14 => self.optcr,
            0x18 => self.optcr1,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // ACR: PRFTEN, ICEN, DCEN, wait states
                self.acr = value & 0x1F7;
                // Auto-increment wait states based on LATENCY field
            }
            0x04 => {
                // KEYR: write unlock key sequence
                // First write 0x45670123, second write 0xCDEF89AB unlocks
                if self.keyr == 0x45670123 && value == 0xCDEF89AB {
                    self.flash_locked = false;
                    self.cr &= !(1 << 31); // clear LOCK
                }
                self.keyr = value;
            }
            0x08 => {
                // OPTKEYR
                self.optkeyr = value;
            }
            0x0C => {
                // SR: clear error flags by writing 1
                self.sr &= !value;
            }
            0x10 => {
                if !self.flash_locked {
                    let locked = value & (1 << 31);
                    self.cr = value & 0x7FFE_FFFF | (locked << 31);
                    if locked != 0 {
                        self.flash_locked = true;
                    }
                    self.cr_psize = (value >> 8) & 0x3;
                }
            }
            0x14 => {
                self.optcr = value & 0x7FDF_FFFF;
            }
            0x18 => {
                self.optcr1 = value & 1;
            }
            _ => {}
        }
    }
}
