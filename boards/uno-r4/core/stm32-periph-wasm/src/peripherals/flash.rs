use crate::system::System;
use super::Peripheral;

// F407 flash layout (1 MB): 4x16KB + 1x64KB + 7x128KB
pub const FLASH_BASE: u32 = 0x0800_0000;
pub const FLASH_SIZE: u32 = 0x0010_0000;

fn sector_range(sector: u32) -> Option<(u32, u32)> {
    let (start, len) = match sector {
        0..=3 => (FLASH_BASE + sector * 0x4000, 0x4000),
        4 => (FLASH_BASE + 4 * 0x4000, 0x10000),
        5..=11 => (FLASH_BASE + 0x20000 + (sector - 5) * 0x20000, 0x20000),
        _ => return None,
    };
    Some((start, len))
}

// Boot state matches hardware: CR reads back 0x80000000 (LOCK set), so the
// flash is LOCKED until the firmware performs the KEYR unlock sequence.
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
    erase_pending: bool,
}

impl Default for Flash {
    fn default() -> Self {
        Self {
            acr: 0, keyr: 0, optkeyr: 0, sr: 0, cr: 0, optcr: 0, optcr1: 0,
            flash_locked: true, cr_psize: 0, erase_pending: false,
        }
    }
}

impl Flash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "FLASH" || name == "FLASH_Trusted" { Some(Box::new(Self::default())) } else { None }
    }

    fn refresh_programming(&mut self) {
        let pg = !self.flash_locked
            && self.cr & (1 << 0) != 0   // PG
            && self.sr & (1 << 16) == 0  // !BSY
            && !self.erase_pending;
        crate::system::set_flash_programming(pg);
    }

    /// Called by the JS driver after it applied the queued erase to guest
    /// memory: clears BSY (firmware's busy-wait can proceed), sets EOP.
    fn start_erase(&mut self) {
        if self.flash_locked || self.sr & (1 << 16) != 0 || self.erase_pending { return; }
        let range = if self.cr & (1 << 2) != 0 { // MER: mass erase
            Some((FLASH_BASE, FLASH_SIZE))
        } else if self.cr & (1 << 1) != 0 { // SER: sector erase
            sector_range((self.cr >> 3) & 0xF)
        } else { None };
        if let Some((start, len)) = range {
            self.erase_pending = true;
            self.sr |= 1 << 16; // BSY held until JS confirms the erase
            crate::system::queue_flash_erase(start, len);
            self.cr &= !(1 << 16); // clear STRT
            self.refresh_programming();
        }
    }
}

impl Peripheral for Flash {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.acr,
            0x04 => self.keyr,
            0x08 => self.optkeyr,
            0x0C => self.sr,
            0x10 => self.cr | if self.flash_locked { 1 << 31 } else { 0 },
            0x14 => self.optcr,
            0x18 => self.optcr1,
            _ => 0,
        }
    }

    /// The JS driver applied the queued erase to guest memory: clear BSY
    /// (firmware's busy-wait can proceed) and set EOP.
    fn flash_erase_applied(&mut self) {
        if self.erase_pending {
            self.erase_pending = false;
            self.sr &= !(1 << 16); // !BSY
            self.sr |= 1 << 0;     // EOP
            self.refresh_programming();
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
                self.refresh_programming();
            }
            0x08 => {
                // OPTKEYR
                self.optkeyr = value;
            }
            0x0C => {
                // SR: clear error flags by writing 1
                self.sr &= !value;
                self.refresh_programming();
            }
            0x10 => {
                if !self.flash_locked {
                    let locked = value & (1 << 31);
                    self.cr = (value & 0x7FFF_FFFF) | locked;
                    if locked != 0 {
                        self.flash_locked = true;
                    }
                    self.cr_psize = (value >> 8) & 0x3;
                    if value & (1 << 16) != 0 { // STRT
                        self.start_erase();
                    }
                }
                self.refresh_programming();
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
