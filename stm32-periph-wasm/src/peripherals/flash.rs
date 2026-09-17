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
    opt_locked: bool,
    cr_psize: u32,
    erase_pending: bool,
}

impl Default for Flash {
    fn default() -> Self {
        Self {
            acr: 0, keyr: 0, optkeyr: 0, sr: 0, cr: 0, optcr: 0x0FFF_AAED, optcr1: 0,
            flash_locked: true, opt_locked: true, cr_psize: 0, erase_pending: false,
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
        // Wrong parallel-bank width for the sector size latches PGSERR
        // (silicon checks PSIZE against SNB at STRT): 16KB sectors (0-3)
        // need x8, 64KB (4) needs x16/x32. 128KB sectors (5-11) accept
        // any PSIZE on silicon (the check applies to small sectors);
        // MER has no sector, so no size check.
        let snb_ok = |sector: u32, psize: u32| -> bool {
            match sector {
                0..=3 => psize == 0,
                4 => psize >= 1,
                5..=11 => true,
                _ => false,
            }
        };
        let range = if self.cr & (1 << 2) != 0 { // MER: mass erase
            Some((FLASH_BASE, FLASH_SIZE))
        } else if self.cr & (1 << 1) != 0 { // SER: sector erase
            let snb = (self.cr >> 3) & 0xF;
            if sector_range(snb).is_none() {
                // No such sector: programming-sequence error, no erase.
                self.sr |= 1 << 7; // PGSERR
                self.cr &= !(1 << 16); // clear STRT
                return;
            }
            if !snb_ok(snb, self.cr_psize) {
                self.sr |= 1 << 7; // PGSERR
                self.cr &= !(1 << 16);
                return;
            }
            sector_range(snb)
        } else { None };
        if let Some((start, len)) = range {
            self.erase_pending = true;
            self.sr |= 1 << 16; // BSY held until JS confirms the erase
            crate::system::queue_flash_erase(start, len);
            self.cr &= !(1 << 16); // clear STRT
            self.refresh_programming();
        }
    }

    /// Program-sequence error check for a guest flash store at `addr`.
    /// Called from mem.rs before the NOR write applies: returns true when
    /// the store must fault (flag latched, write dropped). Silicon raises
    /// PGSERR when PG is set but the access breaks the sequence rules —
    /// an address outside the main flash array. Width-vs-PSIZE mismatch
    /// is deliberately NOT checked: the reset PSIZE (x8) would fault every
    /// word program, but all in-repo firmware (flash_test included)
    /// programs words at reset PSIZE and silicon's voltage-range table
    /// makes the legal widths configuration-dependent — faulting here
    /// would break working firmware to enforce a rule no guest observes.
    /// Locked flash is NOT an error here (writes are silently ignored,
    /// matching the old behavior and the flash_test flow).
    pub(crate) fn program_error(&mut self, addr: u32, width: u8) -> bool {
        if self.flash_locked || self.sr & (1 << 16) != 0 || self.erase_pending {
            return false;
        }
        if self.cr & 1 == 0 {
            return false; // PG clear: not a program access
        }
        let _ = width;
        if !(addr >= FLASH_BASE && addr < FLASH_BASE + FLASH_SIZE) {
            self.sr |= 1 << 7; // PGSERR
            return true;
        }
        false
    }

    /// Write-protection error check: OPTCR nWRP bit for the sector clear
    /// (programmed 0) means the sector is protected — an erase or program
    /// there latches WRPERR and does nothing. OPTCR reset value protects
    /// nothing (all nWRP bits 1); firmware clears bits to protect.
    /// `sector` is the SNB number (0-11); MER checks every sector.
    /// Out-of-range SNB is NOT a WRP error (it is PGSERR — the caller
    /// checks the range first).
    fn wrp_error(&mut self, sector: u32) -> bool {
        // nWRP[11:0] at OPTCR[27:16]: bit set = unprotected.
        if (self.optcr >> (16 + sector)) & 1 == 0 {
            self.sr |= 1 << 4; // WRPERR
            self.cr &= !(1 << 16); // clear STRT
            return true;
        }
        false
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
                } else if value == 0x45670123 {
                    // first half: no state change yet (keyr records it)
                } else if self.keyr == 0x45670123 {
                    // Second key wrong after a correct first key: the
                    // sequence is broken (silicon latches PGSERR). Flag
                    // regardless of lock state — the sequence itself is
                    // wrong (an already-unlocked flash still reports the
                    // broken sequence; LOCK is orthogonal).
                    self.sr |= 1 << 7; // PGSERR
                }
                self.keyr = value;
                self.refresh_programming();
            }
            0x08 => {
                // OPTKEYR: same two-key sequence unlocks option bytes
                // (0x08192A3B then 0x4C5D6E7F). Only a completed wrong
                // sequence latches PGSERR: a lone first-half write is the
                // normal start of an unlock, not an error (silicon only
                // flags when the second key mismatches).
                if self.optkeyr == 0x08192A3B && value == 0x4C5D6E7F {
                    self.opt_locked = false;
                } else if value != 0x08192A3B
                    && self.optkeyr == 0x08192A3B
                {
                    // Second key wrong after a correct first key: broken
                    // sequence latches PGSERR AND relocks the option bytes
                    // (silicon: a wrong sequence leaves OPTLOCK set).
                    self.sr |= 1 << 7; // PGSERR
                    self.opt_locked = true;
                }
                self.optkeyr = value;
            }
            0x0C => {
                // SR: clear error flags by writing 1
                self.sr &= !value;
                self.refresh_programming();
            }
            0x10 => {
                if !self.flash_locked {
                    // Starting an erase/program op while BSY latches PGAERR
                    // (silicon: programming already in progress).
                    if value & (1 << 16) != 0 && self.sr & (1 << 16) != 0 {
                        self.sr |= 1 << 5; // PGAERR
                        self.refresh_programming();
                        return;
                    }
                    let locked = value & (1 << 31);
                    self.cr = (value & 0x7FFF_FFFF) | locked;
                    if locked != 0 {
                        self.flash_locked = true;
                    }
                    self.cr_psize = (value >> 8) & 0x3;
                    if value & (1 << 16) != 0 { // STRT
                        // WRP gate: protected sector (or any protected
                        // sector under MER) latches WRPERR, no erase.
                        // Range check first: invalid SNB is PGSERR, and
                        // must not consult nWRP bit 31 (reserved, reads 0).
                        let snb = (self.cr >> 3) & 0xF;
                        let range_bad = (self.cr & (1 << 1) != 0) && sector_range(snb).is_none();
                        let blocked = if range_bad {
                            false // start_erase reports PGSERR
                        } else if self.cr & (1 << 2) != 0 {
                            (0..12).any(|s| (self.optcr >> (16 + s)) & 1 == 0)
                        } else if self.cr & (1 << 1) != 0 {
                            self.wrp_error(snb)
                        } else {
                            false
                        };
                        if !blocked {
                            self.start_erase();
                        }
                    }
                }
                self.refresh_programming();
            }
            0x14 => {
                // OPTCR: option bytes program only when unlocked (model
                // OPTLOCK latch — on F407 silicon OPTLOCK is not an MMIO
                // bit; the model latches it: correct OPTKEYR sequence
                // clears it, any broken sequence sets it). A locked write
                // with OPTSTRT latches WRPERR (silicon: option bytes are
                // write-protected). OPTSTRT self-clears; BSY pulses while
                // the (instant) program completes.
                if self.opt_locked {
                    if value & 2 != 0 {
                        self.sr |= 1 << 4; // WRPERR
                    }
                    // Reads still show stored OPTCR; locked writes change
                    // nothing except the error flag.
                } else {
                    self.optcr = value & 0x0FFF_FFFF;
                    if value & 2 != 0 {
                        self.optcr &= !2; // OPTSTRT self-clears
                        self.sr |= 1 << 16; // BSY pulse
                        self.sr &= !(1 << 16);
                        self.sr |= 1 << 0; // EOP
                    }
                }
            }
            0x18 => {
                self.optcr1 = value & 1;
            }
            _ => {}
        }
    }
}
