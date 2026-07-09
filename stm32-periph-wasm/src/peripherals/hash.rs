use crate::system::System;
use super::Peripheral;
use sha1::Digest;

pub struct Hash {
    cr: u32, nbw: u32, din: u32, str_: u32,
    hr: [u32; 8],
    imr: u32, sr: u32, csr: [u32; 54],
    hash_hr: [u32; 8],
    msg_buf: Vec<u8>,
    dcma_pending: bool,
}

impl Default for Hash {
    fn default() -> Self {
        Self {
            cr: 0, nbw: 0, din: 0, str_: 0, hr: [0; 8],
            imr: 0, sr: 0x01, csr: [0; 54], hash_hr: [0; 8],
            msg_buf: Vec::new(), dcma_pending: false,
        }
    }
}

impl Hash {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "HASH" { Some(Box::new(Self::default())) } else { None }
    }

    fn algo(&self) -> u8 {
        let al0 = ((self.cr >> 7) & 1) as u8;
        let al1 = ((self.cr >> 18) & 1) as u8;
        (al1 << 1) | al0
    }

    fn compute_digest(&mut self) {
        let nblw = self.str_ & 0x1F;
        let effective_len = if nblw == 0 {
            self.msg_buf.len()
        } else {
            let nwords = self.msg_buf.len() / 4;
            let last_word_bytes = ((nblw + 7) / 8) as usize;
            if nwords > 0 { (nwords - 1) * 4 + last_word_bytes } else { last_word_bytes }
        };
        let effective_msg = &self.msg_buf[..effective_len.min(self.msg_buf.len())];

        match self.algo() {
            0 => {
                // SHA-1
                let mut hasher = sha1::Sha1::new();
                hasher.update(effective_msg);
                let result = hasher.finalize();
                for i in 0..5 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(&result[i * 4..(i + 1) * 4]);
                    self.hr[i] = u32::from_be_bytes(bytes);
                }
                for i in 0..5 {
                    self.hash_hr[i] = self.hr[i];
                }
            }
            1 => {
                // MD5
                let mut hasher = md5::Md5::new();
                hasher.update(effective_msg);
                let result = hasher.finalize();
                for i in 0..4 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(&result[i * 4..(i + 1) * 4]);
                    self.hr[i] = u32::from_be_bytes(bytes);
                    self.hash_hr[i] = self.hr[i];
                }
            }
            2 => {
                // SHA-256
                let mut hasher = sha2::Sha256::new();
                hasher.update(effective_msg);
                let result = hasher.finalize();
                for i in 0..8 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(&result[i * 4..(i + 1) * 4]);
                    self.hash_hr[i] = u32::from_be_bytes(bytes);
                }
                // SHA-256 also reflected in HR0..3 (first 128 bits)
                for i in 0..4 {
                    self.hr[i] = self.hash_hr[i];
                }
            }
            _ => {}
        }
        self.sr |= 0x0A; // BUSY + DCIS
        self.dcma_pending = false;
    }
}

impl Peripheral for Hash {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let mut v = self.cr;
                v = (v & !0x1F00) | ((self.nbw & 0x0F) << 8);
                if !self.msg_buf.is_empty() { v |= 0x1000; } // DINNE
                v
            }
            0x04 => self.din,
            0x08 => self.str_ & 0x1_001F,
            0x0C..=0x1C => self.hr[((offset - 0x0C) / 4) as usize],
            0x20 => self.imr & 0x03,
            0x24 => self.sr,
            0xF8..=0x1CC => self.csr[((offset - 0xF8) / 4) as usize],
            0x310..=0x32C => self.hash_hr[((offset - 0x310) / 4) as usize],
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr = value;
                if value & 1 != 0 {
                    self.cr &= !1; // INIT self-clears
                    self.msg_buf.clear();
                    self.nbw = 0;
                    self.hr = [0; 8];
                    self.hash_hr = [0; 8];
                }
            }
            0x04 => {
                self.din = value;
                self.msg_buf.extend_from_slice(&value.to_be_bytes());
                self.nbw = (self.nbw + 1) & 0x0F;
            }
            0x08 => {
                self.str_ = value & 0x1_001F;
                if value & 0x100 != 0 {
                    self.compute_digest();
                }
            }
            0x20 => self.imr = value & 0x03,
            0x24 => { self.sr = (self.sr & 0xF8) | (value & 0x03); }
            0xF8..=0x1CC => self.csr[((offset - 0xF8) / 4) as usize] = value,
            _ => {}
        }
    }
}
