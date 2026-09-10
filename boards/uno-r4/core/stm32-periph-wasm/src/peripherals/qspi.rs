use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::{LazyLock, Mutex};

use crate::system::System;
use super::Peripheral;

// 32-byte FIFO = 8 × u32, matching the real QUADSPI data FIFO.
const QSPI_FIFO_WORDS: usize = 8;

// Optional external flash backend, keyed by peripheral name. Populated by
// `qspi_register_flash()` *before* init() (mirrors the SPI/FSMC tap
// registration pattern). Guarded by a Mutex because cargo runs lib tests in
// parallel threads and they would otherwise race on the shared map.
static QSPI_FLASH: LazyLock<Mutex<HashMap<String, Vec<u8>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Register an external QSPI flash image for the named peripheral. Call this
/// before init() so the model can service indirect read/write transfers.
pub fn qspi_register_flash(name: &str, data: &[u8]) {
    QSPI_FLASH
        .lock()
        .unwrap()
        .insert(name.to_string(), data.to_vec());
}

pub struct Qspi {
    name: String,
    cr: u32,
    dcr: u32,
    sr: u32,
    dlr: u32,
    ccr: u32,
    ar: u32,
    abr: u32,
    psmkr: u32,
    psmar: u32,
    pir: u32,
    lptr: u32,
    fifo: VecDeque<u32>,
    // Active indirect transfer state.
    xfer_mode: u8, // 0 none, 1 indirect read, 2 indirect write, 3 auto-poll
    xfer_addr: u32,
    xfer_remaining: u32, // bytes not yet delivered to/from the guest
    flash: Option<Vec<u8>>,
}

impl Default for Qspi {
    fn default() -> Self {
        Self {
            name: String::new(),
            cr: 0,
            dcr: 0,
            sr: 0,
            dlr: 0,
            ccr: 0,
            ar: 0,
            abr: 0,
            psmkr: 0,
            psmar: 0,
            pir: 0,
            lptr: 0,
            fifo: VecDeque::new(),
            xfer_mode: 0,
            xfer_addr: 0,
            xfer_remaining: 0,
            flash: None,
        }
    }
}

impl Qspi {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name != "QUADSPI" {
            return None;
        }
        let mut q = Self::default();
        q.name = name.to_string();
        if let Some(flash) = QSPI_FLASH.lock().unwrap().get(name) {
            q.flash = Some(flash.clone());
        }
        Some(Box::new(q))
    }

    #[cfg(test)]
    pub fn with_flash(name: &str, flash: Vec<u8>) -> Self {
        let mut q = Self::default();
        q.name = name.to_string();
        q.flash = Some(flash);
        q
    }

    fn data_len(&self) -> u32 {
        if self.dlr == 0xFFFF_FFFF {
            1
        } else {
            self.dlr.wrapping_add(1)
        }
    }

    fn read_flash_word(&self, addr: u32) -> u32 {
        match &self.flash {
            Some(flash) => {
                let a = addr as usize;
                if a + 4 <= flash.len() {
                    u32::from_le_bytes([flash[a], flash[a + 1], flash[a + 2], flash[a + 3]])
                } else if a < flash.len() {
                    let mut b = [0u8; 4];
                    for i in 0..4 {
                        b[i] = *flash.get(a + i).unwrap_or(&0);
                    }
                    u32::from_le_bytes(b)
                } else {
                    0xFFFF_FFFF
                }
            }
            None => 0xFFFF_FFFF,
        }
    }

    fn write_flash_word(&mut self, addr: u32, value: u32) {
        if let Some(flash) = &mut self.flash {
            let a = addr as usize;
            let bytes = value.to_le_bytes();
            if a + 4 <= flash.len() {
                flash[a..a + 4].copy_from_slice(&bytes);
            } else if a < flash.len() {
                for i in 0..4 {
                    if a + i < flash.len() {
                        flash[a + i] = bytes[i];
                    }
                }
            }
        }
    }

    fn update_status(&mut self) {
        let level = (self.fifo.len() as u32) & 0x3F;
        self.sr = (self.sr & !0x3F00) | (level << 8);
        let thr = ((self.cr >> 8) & 0x1F) as usize;
        let thr = if thr == 0 { 1 } else { thr };
        if self.fifo.len() >= thr {
            self.sr |= 1 << 2; // FT
        } else {
            self.sr &= !(1 << 2);
        }
    }

    fn complete(&mut self) {
        self.sr |= 1 << 1; // TC
        self.sr &= !(1 << 5); // clear BUSY
        self.xfer_mode = 0;
        self.xfer_remaining = 0;
        self.fifo.clear();
        self.update_status();
    }

    fn flush_fifo(&mut self) {
        while let Some(w) = self.fifo.pop_front() {
            self.write_flash_word(self.xfer_addr, w);
            self.xfer_addr = self.xfer_addr.wrapping_add(4);
        }
    }

    fn refill_fifo(&mut self) {
        let thr = ((self.cr >> 8) & 0x1F) as usize;
        let thr = if thr == 0 { 1 } else { thr };
        while self.fifo.len() < thr && self.xfer_remaining > 0 {
            let w = self.read_flash_word(self.xfer_addr);
            self.fifo.push_back(w);
            self.xfer_addr = self.xfer_addr.wrapping_add(4);
            self.xfer_remaining = self.xfer_remaining.saturating_sub(4);
        }
        self.update_status();
    }

    fn start_xfer(&mut self) {
        if self.cr & 1 == 0 {
            return; // peripheral not enabled
        }
        let fmode = (self.ccr >> 28) & 3;
        let dmode = (self.ccr >> 24) & 3;
        self.sr |= 1 << 5; // BUSY
        self.sr &= !(1 << 1); // clear TC
        match fmode {
            0 => {
                // indirect write
                if dmode != 0 {
                    self.xfer_mode = 2;
                    self.xfer_addr = self.ar;
                    self.xfer_remaining = self.data_len();
                } else {
                    self.complete(); // command-only
                }
            }
            1 => {
                // indirect read
                if dmode != 0 {
                    self.xfer_mode = 1;
                    self.xfer_addr = self.ar;
                    self.xfer_remaining = self.data_len();
                    self.refill_fifo();
                } else {
                    self.complete();
                }
            }
            2 => {
                // auto status polling
                let v = self.read_flash_word(self.ar);
                if (v & self.psmkr) == (self.psmar & self.psmkr) {
                    self.sr |= 1 << 3; // SMF
                }
                self.complete();
            }
            _ => {
                // mem-mapped mode: not backed by a real memory map here
                self.complete();
            }
        }
    }

    fn read_dr(&mut self) -> u32 {
        if self.xfer_mode != 1 {
            return 0;
        }
        if self.fifo.is_empty() {
            if self.xfer_remaining > 0 {
                self.refill_fifo();
            } else {
                self.complete();
                return 0;
            }
        }
        let w = self.fifo.pop_front().unwrap_or(0);
        self.update_status();
        if self.fifo.is_empty() && self.xfer_remaining == 0 {
            self.complete();
        }
        w
    }

    fn write_dr(&mut self, value: u32) {
        if self.xfer_mode != 2 {
            return;
        }
        self.fifo.push_back(value);
        self.xfer_remaining = self.xfer_remaining.saturating_sub(4);
        if self.fifo.len() >= QSPI_FIFO_WORDS || self.xfer_remaining == 0 {
            self.flush_fifo();
        }
        self.update_status();
        if self.xfer_remaining == 0 {
            self.complete();
        }
    }
}

impl Peripheral for Qspi {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.dcr,
            0x08 => {
                self.update_status();
                self.sr
            }
            0x0C => 0, // FCR is write-only (flag clear)
            0x10 => self.dlr,
            0x14 => self.ccr,
            0x18 => self.ar,
            0x1C => self.abr,
            0x20 => self.read_dr(),
            0x24 => self.psmkr,
            0x28 => self.psmar,
            0x2C => self.pir,
            0x30 => self.lptr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = value & 0xFC7F_0F1F;
                if self.cr & (1 << 1) != 0 {
                    // ABORT: clear all status flags and pending transfer
                    self.sr &= !((1 << 5) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4));
                    self.fifo.clear();
                    self.xfer_mode = 0;
                    self.xfer_remaining = 0;
                    self.cr &= !(1 << 1);
                }
            }
            0x04 => self.dcr = value & 0x00FF_FFFF,
            0x08 => {} // SR is read-only
            0x0C => {
                // FCR: clear the flagged bits
                self.sr &= !(value & 0x1F);
                self.update_status();
            }
            0x10 => self.dlr = value,
            0x14 => {
                self.ccr = value;
                self.start_xfer();
            }
            0x18 => self.ar = value,
            0x1C => self.abr = value,
            0x20 => self.write_dr(value),
            0x24 => self.psmkr = value,
            0x28 => self.psmar = value,
            0x2C => self.pir = value,
            0x30 => self.lptr = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::test_dummy_system;

    // The flash map is process-global; serialize QSPI tests that touch it.
    static QSPI_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn registers_read_write() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let mut q = Qspi::with_flash("QUADSPI_T1", vec![]);
        q.write(&sys, 0x00, 0x0000_0001); // EN
        assert_eq!(q.read(&sys, 0x00), 0x0000_0001);
        q.write(&sys, 0x04, 0x0000_0F05); // DCR: CSHT=0xF, CKDIV=5
        assert_eq!(q.read(&sys, 0x04) & 0x00FF_FFFF, 0x0000_0F05);
        q.write(&sys, 0x10, 0x1234); // DLR
        assert_eq!(q.read(&sys, 0x10), 0x1234);
    }

    #[test]
    fn indirect_read_returns_flash_bytes() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let flash = vec![
            0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88, 0x99, 0xAA, 0xBB, 0xCC,
        ];
        let mut q = Qspi::with_flash("QUADSPI_T2", flash);
        q.write(&sys, 0x00, 1); // CR EN
        q.write(&sys, 0x18, 0x00); // AR = 0
        q.write(&sys, 0x10, 3); // DLR = 3 -> 4 bytes
        let ccr = (1 << 28) | (1 << 24) | 0xEB; // FMODE read, DMODE 1-line, cmd 0xEB
        q.write(&sys, 0x14, ccr);
        assert!(q.read(&sys, 0x08) & (1 << 5) != 0, "BUSY set on read start");
        let w0 = q.read(&sys, 0x20);
        assert_eq!(w0, 0x44_33_22_11, "first word little-endian from flash");
        let sr = q.read(&sys, 0x08);
        assert!(sr & (1 << 1) != 0, "TC set after all data read");
        assert!(sr & (1 << 5) == 0, "BUSY cleared after completion");
    }

    #[test]
    fn indirect_write_stores_flash_bytes() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let mut q = Qspi::with_flash("QUADSPI_T3", vec![0u8; 8]);
        q.write(&sys, 0x00, 1);
        q.write(&sys, 0x18, 0x00);
        q.write(&sys, 0x10, 3); // 4 bytes
        let ccr = (0 << 28) | (1 << 24) | 0x02; // FMODE write, DMODE 1-line
        q.write(&sys, 0x14, ccr);
        q.write(&sys, 0x20, 0xDD_CC_BB_AA);
        let sr = q.read(&sys, 0x08);
        assert!(sr & (1 << 1) != 0, "TC set after write complete");
        assert!(sr & (1 << 5) == 0, "BUSY cleared");
        assert_eq!(q.read_flash_word(0), 0xDD_CC_BB_AA, "flash updated");
    }

    #[test]
    fn multi_word_transfer() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let flash = vec![0u8; 16];
        let mut q = Qspi::with_flash("QUADSPI_T4", flash);
        q.write(&sys, 0x00, 1);
        q.write(&sys, 0x18, 0x00);
        q.write(&sys, 0x10, 15); // 16 bytes = 4 words
        let ccr = (1 << 28) | (1 << 24) | 0xEB; // indirect read
        q.write(&sys, 0x14, ccr);
        let w0 = q.read(&sys, 0x20);
        let w1 = q.read(&sys, 0x20);
        let w2 = q.read(&sys, 0x20);
        let w3 = q.read(&sys, 0x20);
        assert_eq!([w0, w1, w2, w3], [0, 0, 0, 0]); // blank flash
        assert!(q.read(&sys, 0x08) & (1 << 1) != 0, "TC after 4 words");
    }

    #[test]
    fn abort_clears_flags() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let mut q = Qspi::with_flash("QUADSPI_T5", vec![0u8; 16]);
        q.write(&sys, 0x00, 1);
        q.write(&sys, 0x10, 7); // 8 bytes
        q.write(&sys, 0x14, (1 << 28) | (1 << 24));
        assert!(q.read(&sys, 0x08) & (1 << 5) != 0, "BUSY set");
        q.write(&sys, 0x00, 1 | (1 << 1)); // ABORT
        let sr = q.read(&sys, 0x08);
        assert!(
            sr & ((1 << 5) | (1 << 1) | (1 << 2) | (1 << 3) | (1 << 4)) == 0,
            "all flags cleared after abort"
        );
    }

    #[test]
    fn command_only_completes() {
        let _l = QSPI_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let sys = test_dummy_system();
        let mut q = Qspi::with_flash("QUADSPI_T6", vec![]);
        q.write(&sys, 0x00, 1);
        // FMODE write, DMODE 0 (no data) -> command-only transfer completes
        q.write(&sys, 0x14, (0 << 28) | 0x06); // WREN
        let sr = q.read(&sys, 0x08);
        assert!(sr & (1 << 1) != 0, "TC set for command-only transfer");
        assert!(sr & (1 << 5) == 0, "BUSY cleared");
    }
}
