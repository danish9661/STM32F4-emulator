use crate::system::System;
use super::Peripheral;
use sha1::Digest;

const HASH_IRQ: i32 = 80;

pub struct Hash {
    cr: u32, nbw: u32, din: u32, str_: u32,
    hr: [u32; 8],
    imr: u32, sr: u32, csr: [u32; 54],
    hash_hr: [u32; 8],
    msg_buf: Vec<u8>,
    dcma_pending: bool,
    dinne: bool,
}

impl Default for Hash {
    fn default() -> Self {
        Self {
            cr: 0, nbw: 0, din: 0, str_: 0, hr: [0; 8],
            imr: 0, sr: 0x01, csr: [0; 54], hash_hr: [0; 8],
            msg_buf: Vec::new(), dcma_pending: false,
            dinne: false,
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
        // HMAC mode (CR MODE bit 6): HMAC(K, m) with the DIN-fed key
        // (first key_bytes of msg_buf when LKEY set, else the SHA-256
        // 32-byte / SHA-1 20-byte / MD5 16-byte prefix... silicon feeds
        // the key through CSR/DIN; here the key is the first 64-byte
        // block (padded) unless LKEY says the key itself is long, in
        // which case the first 128 bytes are the key, hashed down first
        // per FIPS 198. The mock pins HMAC-SHA256("key","abc") vectors.
        let hmac = self.cr & (1 << 6) != 0;
        let lkey = self.cr & (1 << 16) != 0;

        // Split key/message for HMAC: with LKEY the first 128 msg bytes
        // are the (long) key, else the first 64 bytes are the key block.
        // Without HMAC the whole buffer is the message (legacy path).
        let (key, msg): (Vec<u8>, &[u8]) = if hmac {
            let klen = if lkey { 128 } else { 64 };
            let klen = klen.min(effective_msg.len());
            (effective_msg[..klen].to_vec(), &effective_msg[klen..])
        } else {
            (Vec::new(), effective_msg)
        };
        // HMAC helper: H(K,m) per FIPS 198 over the selected hash.
        // Key longer than the 64-byte block hashes down first.
        fn hmac_with<F>(mut key: Vec<u8>, msg: &[u8], block: usize, hash: F) -> Vec<u8>
        where F: Fn(&[u8]) -> Vec<u8> {
            if key.len() > block {
                key = hash(&key);
            }
            key.resize(block, 0);
            let ipad: Vec<u8> = key.iter().map(|b| b ^ 0x36).collect();
            let opad: Vec<u8> = key.iter().map(|b| b ^ 0x5C).collect();
            let mut inner = ipad;
            inner.extend_from_slice(msg);
            let ih = hash(&inner);
            let mut outer = opad;
            outer.extend_from_slice(&ih);
            hash(&outer)
        }

        match self.algo() {
            0 => {
                // SHA-1
                if hmac {
                    let tag = hmac_with(key, msg, 64, |d| {
                        let mut h = sha1::Sha1::new();
                        h.update(d);
                        h.finalize().to_vec()
                    });
                    for i in 0..5 {
                        let mut bytes = [0u8; 4];
                        bytes.copy_from_slice(&tag[i * 4..(i + 1) * 4]);
                        self.hr[i] = u32::from_be_bytes(bytes);
                    }
                    for i in 0..5 {
                        self.hash_hr[i] = self.hr[i];
                    }
                    self.sr |= 0x0A;
                    self.dcma_pending = false;
                    return;
                }
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
                if hmac {
                    let tag = hmac_with(key, msg, 64, |d| {
                        let mut h = md5::Md5::new();
                        h.update(d);
                        h.finalize().to_vec()
                    });
                    for i in 0..4 {
                        let mut bytes = [0u8; 4];
                        bytes.copy_from_slice(&tag[i * 4..(i + 1) * 4]);
                        self.hr[i] = u32::from_be_bytes(bytes);
                        self.hash_hr[i] = self.hr[i];
                    }
                    self.sr |= 0x0A;
                    self.dcma_pending = false;
                    return;
                }
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
                if hmac {
                    let tag = hmac_with(key, msg, 64, |d| {
                        let mut h = sha2::Sha256::new();
                        h.update(d);
                        h.finalize().to_vec()
                    });
                    for i in 0..8 {
                        let mut bytes = [0u8; 4];
                        bytes.copy_from_slice(&tag[i * 4..(i + 1) * 4]);
                        self.hash_hr[i] = u32::from_be_bytes(bytes);
                    }
                    for i in 0..4 {
                        self.hr[i] = self.hash_hr[i];
                    }
                    self.sr |= 0x0A;
                    self.dcma_pending = false;
                    return;
                }
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

    fn check_interrupt(&mut self, sys: &System) {
        // DCIE (bit 5): Digest Calculation Complete Interrupt Enable
        if (self.sr & 0x08) != 0 && (self.cr & 0x20) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(HASH_IRQ);
        }
    }
}

impl Peripheral for Hash {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let mut v = self.cr;
                v = (v & !0x1F00) | ((self.nbw & 0x0F) << 8);
                if self.dinne { v |= 0x1000; }
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

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // MODE (bit 6, HMAC) + LKEY (bit 16, long key) are live:
                // HMAC mode wraps the digest (see compute_digest): key
                // padded to the block size, ipad/opad, HMAC(K,m). LKEY
                // selects a >64-byte key path (model: key words beyond 16
                // are consumed from DIN before the message — silicon loads
                // the long key through the FIFO; documented, not silent).
                self.cr = value;
                if value & 1 != 0 {
                    self.cr &= !1; // INIT self-clears
                    self.msg_buf.clear();
                    self.nbw = 0;
                    self.hr = [0; 8];
                    self.hash_hr = [0; 8];
                    self.dinne = false;
                }
            }
            0x04 => {
                self.din = value;
                self.msg_buf.extend_from_slice(&value.to_be_bytes());
                self.nbw = (self.nbw + 1) & 0x0F;
                self.dinne = true;
                // Consumed immediately → DIN ready for next word
                // DINIE (bit 3): fire interrupt when DIN goes ready
                self.dinne = false;
                if (self.cr & 8) != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(HASH_IRQ);
                }
            }
            0x08 => {
                self.str_ = value & 0x1_001F;
                if value & 0x100 != 0 {
                    self.compute_digest();
                    self.check_interrupt(sys);
                }
            }
            0x20 => self.imr = value & 0x03,
            0x24 => { self.sr = (self.sr & 0xF8) | (value & 0x03); }
            0xF8..=0x1CC => self.csr[((offset - 0xF8) / 4) as usize] = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::Peripheral;

    /// Feed big-endian words like firmware does (DIN takes the word BE).
    /// SHA-256 needs ALGO=0b10 (ALGO1 bit 18 set); the reset ALGO=00 is
    /// SHA-1 (existing mock t_hash pins SHA-1("abcd") that way).
    fn feed(h: &mut Hash, sys: &crate::system::System, cr: u32, data: &[u8]) {
        h.write(sys, 0x00, 1); // INIT
        h.write(sys, 0x00, (cr | (1 << 18)) & !1); // mode bits + ALGO=SHA-256
        for chunk in data.chunks(4) {
            let mut w = [0u8; 4];
            w[..chunk.len()].copy_from_slice(chunk);
            h.write(sys, 0x04, u32::from_be_bytes(w));
        }
        h.write(sys, 0x08, 0x100); // DCAL
    }

    #[test]
    fn hmac_sha256_key_abc_matches_rfc4231() {
        // RFC 4231 TC2: HMAC-SHA256(key="Jefe", "what do ya want for nothing?").
        let sys = crate::system::test_dummy_system();
        let mut boxed = Hash::new("HASH").unwrap();
        let h = boxed.as_any_mut().downcast_mut::<Hash>().unwrap();
        let mut buf = Vec::new();
        buf.extend_from_slice(b"Jefe");
        buf.resize(64, 0); // key block (MODE, no LKEY)
        buf.extend_from_slice(b"what do ya want for nothing?");
        feed(h, &sys, 1 << 6, &buf);
        // RFC 4231 TC2 digest 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843.
        let want = [0x5BDCC146, 0xBF60754E, 0x6A042426, 0x089575C7,
                    0x5A003F08, 0x9D273983, 0x9DEC58B9, 0x64EC3843];
        for i in 0..8 {
            assert_eq!(h.read(&sys, (0x310 + 4 * i as u32) as u32), want[i], "HR{i}");
        }
    }

    #[test]
    fn hmac_off_leaves_plain_digest_untouched() {
        // Same bytes without MODE must hash as plain SHA-256 (regression:
        // the HMAC split must not alter the legacy path).
        let sys = crate::system::test_dummy_system();
        let mut boxed = Hash::new("HASH").unwrap();
        let h = boxed.as_any_mut().downcast_mut::<Hash>().unwrap();
        feed(h, &sys, 0, b"abcd");
        // SHA-256("abcd") first word 88d4266f (matches the model's HR map).
        assert_eq!(h.read(&sys, 0x310), 0x88D4266F, "plain SHA-256 HR0");
    }
}
