use crate::system::System;
use super::Peripheral;
use aes::cipher::{BlockEncrypt, BlockDecrypt, KeyInit, generic_array::GenericArray};
use aes::Aes128;
use aes::Aes192;
use aes::Aes256;
use des::Des;
use des::TdesEde3;

const AES_BLOCK: usize = 16;
const MAX_KEY_WORDS: usize = 8;
const MAX_IV_WORDS: usize = 4;

pub struct Cryp {
    cr: u32, sr: u32, dmacr: u32, imscr: u32,
    key: [u32; MAX_KEY_WORDS],
    iv: [u32; MAX_IV_WORDS],
    in_buf: Vec<u8>,
    out_buf: Vec<u8>,
    din: u32, dout: u32,
    ctr: [u8; 16],          // CTR mode counter
    // GCM/CCM state
    ghash_h: [u8; 16],      // H = AES(K, 0^128)
    ghash_y: [u8; 16],      // current GHASH state
    ghash_aad_done: bool,
    ghash_data_done: bool,
    gcm_j0: [u8; 16],       // J0 for GCM
    gcm_ctr: [u8; 16],      // current GCM counter
    gcm_aad_len: u64,       // AAD bit-length for GCM tag
    gcm_data_len: u64,      // ciphertext/plaintext bit-length for GCM tag
    ccm_b0: [u8; 16],       // B0 for CCM
    ccm_mac: [u8; 16],      // running CBC-MAC
    ccm_aad_len: usize,
    ccm_data_len: usize,
    gcm_phase_done: bool,
}

impl Default for Cryp {
    fn default() -> Self {
        Self {
            cr: 0, sr: 0x03, dmacr: 0, imscr: 0,
            key: [0; MAX_KEY_WORDS], iv: [0; MAX_IV_WORDS],
            in_buf: Vec::new(), out_buf: Vec::new(),
            din: 0, dout: 0,
            ctr: [0; 16],
            ghash_h: [0; 16], ghash_y: [0; 16],
            ghash_aad_done: false, ghash_data_done: false,
            gcm_j0: [0; 16], gcm_ctr: [0; 16],
            gcm_aad_len: 0, gcm_data_len: 0,
            ccm_b0: [0; 16], ccm_mac: [0; 16],
            ccm_aad_len: 0, ccm_data_len: 0,
            gcm_phase_done: false,
        }
    }
}

impl Cryp {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CRYP" { Some(Box::new(Self::default())) } else { None }
    }

    fn algomode(&self) -> u8 {
        let lo = ((self.cr >> 3) & 0x07) as u8;
        let hi = ((self.cr >> 19) & 0x01) as u8;
        (hi << 3) | lo
    }

    fn keysize(&self) -> usize {
        if self.algomode() >= 8 {
            return 0; // DES/TDES handled separately
        }
        match (self.cr >> 8) & 0x03 {
            0 => 16, 1 => 24, _ => 32,
        }
    }

    fn is_encrypt(&self) -> bool { (self.cr & 4) == 0 }
    fn datatype(&self) -> u8 { ((self.cr >> 6) & 0x03) as u8 }
    fn gcm_ccmph(&self) -> u8 { ((self.cr >> 16) & 0x03) as u8 }
    fn is_des_mode(&self) -> bool { self.algomode() >= 8 }

    fn key_bytes(&self) -> Vec<u8> {
        if self.is_des_mode() {
            let nw = if self.algomode() == 8 { 2 } else { 6 }; // DES=2, TDES=6
            let mut k = Vec::with_capacity(nw * 4);
            for i in 0..nw {
                k.extend_from_slice(&self.key[i].to_be_bytes());
            }
            return k;
        }
        let nw = self.keysize() / 4;
        let mut k = Vec::with_capacity(self.keysize());
        for i in 0..nw {
            k.extend_from_slice(&self.key[i].to_be_bytes());
        }
        k
    }

    fn iv_bytes(&self) -> [u8; 16] {
        let mut iv = [0u8; 16];
        for i in 0..MAX_IV_WORDS {
            let b = self.iv[i].to_be_bytes();
            iv[i * 4..(i + 1) * 4].copy_from_slice(&b);
        }
        iv
    }

    // DATATYPE byte swapping
    fn apply_datatype(buf: &mut [u8; 16], dt: u8) {
        match dt {
            1 => { // 16-bit: swap bytes within each 16-bit halfword
                for chunk in buf.chunks_mut(2) {
                    chunk.swap(0, 1);
                }
            }
            2 => { // 8-bit: reverse all bytes
                buf.reverse();
            }
            _ => {}
        }
    }

    fn apply_datatype_rev(buf: &mut [u8; 16], dt: u8) {
        Self::apply_datatype(buf, dt);
    }

    // GF(2^128) multiplication for GHASH
    fn gf128_mul(x: &[u8; 16], y: &[u8; 16]) -> [u8; 16] {
        let mut z = [0u8; 16];
        let mut v = *y;
        let r = [0xE1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

        for i in 0..128 {
            let byte_idx = i / 8;
            let bit_idx = 7 - (i % 8);
            if (x[byte_idx] >> bit_idx) & 1 != 0 {
                xor_into(&mut z, &v);
            }
            let lsb = v[15] & 1;
            shift_right(&mut v);
            if lsb != 0 {
                xor_into(&mut v, &r);
            }
        }
        z
    }

    // GHASH core
    fn ghash(h: &[u8; 16], data: &[u8], init_y: &[u8; 16]) -> [u8; 16] {
        let mut y = *init_y;
        for chunk in data.chunks(16) {
            let mut block = [0u8; 16];
            block[..chunk.len()].copy_from_slice(chunk);
            xor_into(&mut y, &block);
            y = Self::gf128_mul(&y, h);
        }
        y
    }

    // Increment counter (32-bit big-endian increment)
    fn inc32(ctr: &mut [u8; 16]) {
        for i in (12..16).rev() {
            ctr[i] = ctr[i].wrapping_add(1);
            if ctr[i] != 0 { break; }
        }
    }

    fn compute_init_gcm(&mut self) {
        let key = self.key_bytes();
        let iv = self.iv_bytes();
        // H = AES(K, 0^128)
        if let Ok(cipher) = Aes128::new_from_slice(&key[..16]) {
            let mut ga = GenericArray::from(self.ghash_h);
            cipher.encrypt_block(&mut ga);
            self.ghash_h.copy_from_slice(ga.as_slice());
        }
        // J0 = IV || 0^31 || 1 for 96-bit IV
        self.gcm_j0 = [0u8; 16];
        self.gcm_j0[..12].copy_from_slice(&iv[..12]);
        self.gcm_j0[15] = 1;
        self.gcm_ctr = self.gcm_j0;
        // Reset GHASH state
        self.ghash_y = [0u8; 16];
        self.ghash_aad_done = false;
        self.ghash_data_done = false;
        self.gcm_aad_len = 0;
        self.gcm_data_len = 0;
        self.gcm_phase_done = false;
    }

    fn compute_init_ccm(&mut self) {
        // For CCM, B0 = flags(0) || nonce(15-L) || L(m)
        // simplified: just use IV as nonce
        self.ccm_b0 = [0u8; 16];
        let iv = self.iv_bytes();
        // B0 = 0x01 || nonce (13 bytes) || length encoding
        self.ccm_b0[0] = 0x01; // flags: L=1 (2-byte length field)
        self.ccm_b0[1..14].copy_from_slice(&iv[1..14]);
        self.ccm_data_len = 0;
        self.ccm_aad_len = 0;
        self.ccm_mac = [0u8; 16];
    }

    // Process a single block for any mode
    fn process_block(&mut self, block: &[u8; 16]) -> [u8; 16] {
        let algomode = self.algomode();
        let encrypt = self.is_encrypt();

        if self.is_des_mode() {
            return self.process_des_block(block, algomode, encrypt);
        }
        self.process_aes_block(block, algomode, encrypt)
    }

    fn process_des_block8(&self, block8: &[u8; 8], mode: u8, encrypt: bool) -> [u8; 8] {
        let key = self.key_bytes();
        let iv = self.iv_bytes();
        let mut result8 = *block8;

        match mode {
            8 => {
                if key.len() >= 8 {
                    let cipher = Des::new_from_slice(&key[..8]).unwrap();
                    let mut ga = GenericArray::from(result8);
                    if encrypt { cipher.encrypt_block(&mut ga); } else { cipher.decrypt_block(&mut ga); }
                    result8.copy_from_slice(ga.as_slice());
                }
            }
            9 => {
                if key.len() >= 8 {
                    let cipher = Des::new_from_slice(&key[..8]).unwrap();
                    if encrypt { for i in 0..8 { result8[i] ^= iv[i]; } }
                    let mut ga = GenericArray::from(result8);
                    if encrypt { cipher.encrypt_block(&mut ga); } else { cipher.decrypt_block(&mut ga); }
                    result8.copy_from_slice(ga.as_slice());
                    if !encrypt { for i in 0..8 { result8[i] ^= iv[i]; } }
                }
            }
            10 => {
                if key.len() >= 24 {
                    let cipher = TdesEde3::new_from_slice(&key[..24]).unwrap();
                    let mut ga = GenericArray::from(result8);
                    if encrypt { cipher.encrypt_block(&mut ga); } else { cipher.decrypt_block(&mut ga); }
                    result8.copy_from_slice(ga.as_slice());
                }
            }
            11 => {
                if key.len() >= 24 {
                    let cipher = TdesEde3::new_from_slice(&key[..24]).unwrap();
                    if encrypt { for i in 0..8 { result8[i] ^= iv[i]; } }
                    let mut ga = GenericArray::from(result8);
                    if encrypt { cipher.encrypt_block(&mut ga); } else { cipher.decrypt_block(&mut ga); }
                    result8.copy_from_slice(ga.as_slice());
                    if !encrypt { for i in 0..8 { result8[i] ^= iv[i]; } }
                }
            }
            _ => {}
        }
        result8
    }

    fn process_des_block(&self, block: &[u8; 16], mode: u8, encrypt: bool) -> [u8; 16] {
        let dt = self.datatype();
        let mut result = *block;
        Self::apply_datatype(&mut result, dt);
        let b0: [u8; 8] = result[..8].try_into().unwrap();
        let b1: [u8; 8] = result[8..].try_into().unwrap();
        let r0 = self.process_des_block8(&b0, mode, encrypt);
        let r1 = self.process_des_block8(&b1, mode, encrypt);
        result[..8].copy_from_slice(&r0);
        result[8..].copy_from_slice(&r1);
        Self::apply_datatype_rev(&mut result, dt);
        result
    }

    fn process_aes_block(&mut self, block: &[u8; 16], mode: u8, encrypt: bool) -> [u8; 16] {
        let key = self.key_bytes();
        let iv = self.iv_bytes();
        let mut result = *block;
        let dt = self.datatype();

        Self::apply_datatype(&mut result, dt);

        let key_sz = self.keysize();

        match mode & 0x07 {
            0 => { // ECB
                Self::aes_ecb(&mut result, &key, key_sz, encrypt);
            }
            1 => { // CBC
                let ctx = iv;
                Self::aes_cbc(&mut result, &ctx, &key, key_sz, encrypt);
            }
            2 => { // CTR
                let mut enc_ctr = self.ctr;
                Self::aes_ecb(&mut enc_ctr, &key, key_sz, true);
                for i in 0..16 { result[i] ^= enc_ctr[i]; }
                // Increment counter after use (big-endian)
                for i in (12..16).rev() {
                    self.ctr[i] = self.ctr[i].wrapping_add(1);
                    if self.ctr[i] != 0 { break; }
                }
            }
            3 => { // GCM data phase: CTR encrypt
                Self::inc32(&mut self.gcm_ctr);
                let mut enc_ctr = self.gcm_ctr;
                Self::aes_ecb(&mut enc_ctr, &key, key_sz, true);
                for i in 0..16 { result[i] ^= enc_ctr[i]; }
            }
            4 => { // CCM
                let mut ctr = [0u8; 16];
                ctr[0] = 1;
                ctr[1..14].copy_from_slice(&iv[1..14]);
                Self::aes_ecb(&mut ctr, &key, key_sz, true);
                for i in 0..16 { result[i] ^= ctr[i]; }
            }
            _ => {}
        }

        Self::apply_datatype_rev(&mut result, dt);
        result
    }

    fn aes_ecb(block: &mut [u8; 16], key: &[u8], keysize: usize, encrypt: bool) {
        match keysize {
            16 => {
                if let Ok(cipher) = Aes128::new_from_slice(&key[..16]) {
                    let mut ga = GenericArray::from(*block);
                    if encrypt { cipher.encrypt_block(&mut ga); }
                    else { cipher.decrypt_block(&mut ga); }
                    block.copy_from_slice(ga.as_slice());
                }
            }
            24 => {
                if let Ok(cipher) = Aes192::new_from_slice(&key[..24]) {
                    let mut ga = GenericArray::from(*block);
                    if encrypt { cipher.encrypt_block(&mut ga); }
                    else { cipher.decrypt_block(&mut ga); }
                    block.copy_from_slice(ga.as_slice());
                }
            }
            _ => {
                if let Ok(cipher) = Aes256::new_from_slice(&key[..32]) {
                    let mut ga = GenericArray::from(*block);
                    if encrypt { cipher.encrypt_block(&mut ga); }
                    else { cipher.decrypt_block(&mut ga); }
                    block.copy_from_slice(ga.as_slice());
                }
            }
        }
    }

    fn aes_cbc(block: &mut [u8; 16], ctx: &[u8; 16], key: &[u8], keysize: usize, encrypt: bool) {
        let prev = *block;
        if encrypt {
            for i in 0..16 { block[i] ^= ctx[i]; }
        }
        Self::aes_ecb(block, key, keysize, encrypt);
        if !encrypt {
            for i in 0..16 { block[i] ^= ctx[i]; }
        }
    }

    fn process_fifo(&mut self, sys: &System) {
        if self.in_buf.len() < 16 {
            return;
        }

        // Determine block size (8 for DES, 16 for AES)
        let bsize = if self.is_des_mode() { 8 } else { 16 };

        while self.in_buf.len() >= bsize {
            let mut block = [0u8; 16];
            block[..bsize].copy_from_slice(&self.in_buf[..bsize]);

            let result = if self.is_des_mode() {
                self.process_des_block(&block, self.algomode(), self.is_encrypt())
            } else {
                let mode = self.algomode() & 0x07;
                // Handle GCM/CCM special phases
                match mode {
                    3 => self.process_gcm_block(&block),
                    4 => self.process_ccm_block(&block),
                    _ => self.process_aes_block(&block, mode, self.is_encrypt()),
                }
            };

            self.out_buf.extend_from_slice(&result[..bsize]);
            self.in_buf.drain(..bsize);
        }

        // Update SR status
        self.sr |= 0x14; // BUSY + OFNE
        if self.in_buf.is_empty() { self.sr |= 1; } // IFEM
        if self.in_buf.len() < 64 { self.sr |= 2; } // IFNF
        if !self.out_buf.is_empty() { self.sr |= 4; } // OFNE
        if self.out_buf.len() >= 64 { self.sr |= 8; } // OFFU

        // Signal interrupt if output FIFO non-empty and OUTIM enabled
        if !self.out_buf.is_empty() && (self.imscr & 1) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(79);
        }
    }

    fn process_gcm_block(&mut self, block: &[u8; 16]) -> [u8; 16] {
        let phase = self.gcm_ccmph();
        let key = self.key_bytes();
        let ks = self.keysize();
        let mut result = *block;

        match phase {
            0 => {
                // Init phase (preparation) - no data processing
                result = *block;
                self.gcm_phase_done = true;
            }
            1 => {
                // Header (AAD) phase - feed to GHASH, no encryption
                xor_into(&mut self.ghash_y, &result);
                self.ghash_y = Self::gf128_mul(&self.ghash_y, &self.ghash_h);
                self.gcm_aad_len += 128; // each block = 128 bits
                self.ghash_aad_done = true;
            }
            2 => {
                // Data phase - CTR encrypt + feed ciphertext to GHASH
                Self::inc32(&mut self.gcm_ctr);
                let mut enc_ctr = self.gcm_ctr;
                Self::aes_ecb(&mut enc_ctr, &key, ks, true);
                if self.is_encrypt() {
                    for i in 0..16 { result[i] ^= enc_ctr[i]; }
                    xor_into(&mut self.ghash_y, &result);
                } else {
                    xor_into(&mut self.ghash_y, &block);
                    for i in 0..16 { result[i] ^= enc_ctr[i]; }
                }
                self.ghash_y = Self::gf128_mul(&self.ghash_y, &self.ghash_h);
                self.gcm_data_len += 128;
                self.ghash_data_done = true;
            }
            3 => {
                // Final phase - compute tag
                // T = GHASH(H, A, C) XOR AES(K, J0)
                let mut len_block = [0u8; 16];
                len_block[..8].copy_from_slice(&self.gcm_aad_len.to_be_bytes());
                len_block[8..].copy_from_slice(&self.gcm_data_len.to_be_bytes());
                xor_into(&mut self.ghash_y, &len_block);
                self.ghash_y = Self::gf128_mul(&self.ghash_y, &self.ghash_h);
                let mut enc_j0 = self.gcm_j0;
                Self::aes_ecb(&mut enc_j0, &key, ks, true);
                for i in 0..16 { result[i] = self.ghash_y[i] ^ enc_j0[i]; }
                self.gcm_phase_done = true;
            }
            _ => {}
        }
        result
    }

    fn process_ccm_block(&mut self, block: &[u8; 16]) -> [u8; 16] {
        let phase = self.gcm_ccmph();
        let key = self.key_bytes();
        let ks = self.keysize();
        let mut result = *block;

        match phase {
            0 => {
                // Init: set up B0 (flags + nonce + length)
                // Already done in compute_init_ccm
                self.ccm_b0 = self.iv_bytes();
                self.ccm_b0[0] = 0x01;
                self.ccm_mac = self.ccm_b0;
                Self::aes_ecb(&mut self.ccm_mac, &key, ks, true);
                self.gcm_phase_done = true;
            }
            1 => {
                // Header (AAD) - CBC-MAC with first block being B0
                xor_into(&mut self.ccm_mac, &block);
                Self::aes_ecb(&mut self.ccm_mac, &key, ks, true);
            }
            2 => {
                // Data: CTR encrypt + update CBC-MAC
                // CBC-MAC is computed on plaintext for both enc/dec
                xor_into(&mut self.ccm_mac, &result);
                Self::aes_ecb(&mut self.ccm_mac, &key, ks, true);
                // CTR encrypt/decrypt
                let mut ctr = [0u8; 16];
                ctr[0] = 1;
                let iv = self.iv_bytes();
                ctr[1..14].copy_from_slice(&iv[1..14]);
                // increment counter within the block
                for j in (14..16).rev() {
                    ctr[j] = ctr[j].wrapping_add(1);
                    if ctr[j] != 0 { break; }
                }
                Self::aes_ecb(&mut ctr, &key, ks, true);
                for i in 0..16 { result[i] ^= ctr[i]; }
            }
            3 => {
                // Final: output CBC-MAC as tag (truncated to 4-16 bytes)
                result = self.ccm_mac;
            }
            _ => {}
        }
        result
    }

}

fn xor_into(a: &mut [u8; 16], b: &[u8; 16]) {
    for i in 0..16 { a[i] ^= b[i]; }
}

fn shift_right(buf: &mut [u8; 16]) {
    let mut carry = 0u8;
    for i in (0..16).rev() {
        let next_carry = buf[i] & 1;
        buf[i] = (buf[i] >> 1) | (carry << 7);
        carry = next_carry;
    }
}

impl Peripheral for Cryp {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr & 0xF_8FFF,
            0x04 => {
                if self.cr & 0x8000 != 0 {
                    self.process_fifo(sys);
                }
                let mut sr = 0u32;
                if self.in_buf.is_empty() { sr |= 1; }
                if self.in_buf.len() < 64 { sr |= 2; }
                if !self.out_buf.is_empty() { sr |= 4; }
                if self.out_buf.len() >= 64 { sr |= 8; }
                if self.cr & 0x8000 != 0 { sr |= 0x10; }
                sr
            }
            0x08 => self.din,
            0x0C => {
                if self.cr & 0x8000 != 0 {
                    self.process_fifo(sys);
                }
                let bsize = if self.is_des_mode() { 8 } else { 16 };
                if self.out_buf.len() >= 4 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(&self.out_buf.drain(..4).collect::<Vec<_>>());
                    self.dout = u32::from_be_bytes(bytes);
                } else if self.out_buf.is_empty() && (self.cr & 0x8000) != 0 {
                    self.dout = 0;
                } else {
                    // drain remaining
                    let mut bytes = [0u8; 4];
                    let n = self.out_buf.len().min(4);
                    for i in 0..n { bytes[i] = self.out_buf[i]; }
                    self.out_buf.drain(..n);
                    self.dout = u32::from_be_bytes(bytes);
                }
                self.dout
            }
            0x10 => self.dmacr & 0x07,
            0x14 => self.imscr & 0x03,
            0x18 => {
                let mut risr = 0x01;
                if !self.out_buf.is_empty() { risr |= 0x02; }
                risr
            }
            0x1C => { // MISR = masked interrupt status
                let ris = if !self.out_buf.is_empty() { 0x02 } else { 0x00 };
                ris & (self.imscr & 0x03)
            }
            0x20..=0x3C => self.key[((offset - 0x20) / 4) as usize],
            0x40..=0x4C => self.iv[((offset - 0x40) / 4) as usize],
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let was_en = (self.cr & 0x8000) != 0;
                self.cr = value & 0xF_8FFF;
                let now_en = (self.cr & 0x8000) != 0;
                if !was_en && now_en {
                    // Enabled: init counter and GCM/CCM if needed
                    let mode = self.algomode() & 0x07;
                    self.ctr = self.iv_bytes();
                    if mode == 2 { /* CTR counter already loaded */ }
                    if mode == 3 { self.compute_init_gcm(); }
                    if mode == 4 { self.compute_init_ccm(); }
                    self.sr = 0x03;
                } else if !now_en {
                    self.sr = 0x03;
                    self.out_buf.clear();
                    self.in_buf.clear();
                }
                if value & 0x4000 != 0 {
                    self.in_buf.clear();
                    self.out_buf.clear();
                    self.ghash_y = [0u8; 16];
                    self.ghash_aad_done = false;
                    self.ghash_data_done = false;
                    self.gcm_aad_len = 0;
                    self.gcm_data_len = 0;
                    self.gcm_phase_done = false;
                    self.sr = 0x03;
                }
            }
            0x08 => {
                self.din = value;
                if self.cr & 0x8000 != 0 {
                    let b = value.to_be_bytes();
                    self.in_buf.extend_from_slice(&b);
                    self.process_fifo(sys);
                }
            }
            0x10 => self.dmacr = value & 0x07,
            0x14 => self.imscr = value & 0x03,
            0x20..=0x3C => self.key[((offset - 0x20) / 4) as usize] = value,
            0x40..=0x4C => self.iv[((offset - 0x40) / 4) as usize] = value,
            _ => {}
        }
    }
}
