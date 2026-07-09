use crate::system::System;
use super::Peripheral;
use aes::cipher::{BlockEncrypt, BlockDecrypt, KeyInit, generic_array::GenericArray};
use aes::Aes128;
use aes::Aes192;
use aes::Aes256;

const AES_BLOCK: usize = 16;
const MAX_KEY_WORDS: usize = 8;
const MAX_IV_WORDS: usize = 4;

pub struct Cryp {
    cr: u32, sr: u32, dmacr: u32, imscr: u32,
    key: [u32; MAX_KEY_WORDS],
    iv: [u32; MAX_IV_WORDS],
    in_buf: Vec<u8>,
    out_buf: Vec<u8>,
    din: u32,
    dout: u32,
}

impl Default for Cryp {
    fn default() -> Self {
        Self {
            cr: 0, sr: 0x03, dmacr: 0, imscr: 0,
            key: [0; MAX_KEY_WORDS], iv: [0; MAX_IV_WORDS],
            in_buf: Vec::new(), out_buf: Vec::new(),
            din: 0, dout: 0,
        }
    }
}

impl Cryp {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "CRYP" { Some(Box::new(Self::default())) } else { None }
    }

    fn algomode(&self) -> u8 {
        let lo = ((self.cr >> 3) & 0x07) as u8;
        let hi = ((self.cr >> 16) & 0x03) as u8;
        (hi << 3) | lo
    }

    fn keysize(&self) -> usize {
        match (self.cr >> 8) & 0x03 {
            0 => 16,
            1 => 24,
            _ => 32,
        }
    }

    fn is_encrypt(&self) -> bool {
        (self.cr & 4) == 0
    }

    fn key_bytes(&self) -> Vec<u8> {
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
            let bytes = self.iv[i].to_be_bytes();
            iv[i * 4..(i + 1) * 4].copy_from_slice(&bytes);
        }
        iv
    }

    fn aes_encrypt_block(cipher: &Aes128, block: &mut [u8; 16]) {
        let mut ga = GenericArray::from(*block);
        cipher.encrypt_block(&mut ga);
        block.copy_from_slice(ga.as_slice());
    }

    fn aes_decrypt_block(cipher: &Aes128, block: &mut [u8; 16]) {
        let mut ga = GenericArray::from(*block);
        cipher.decrypt_block(&mut ga);
        block.copy_from_slice(ga.as_slice());
    }

    fn process_block(&self, mut block: [u8; 16]) -> [u8; 16] {
        let key = self.key_bytes();
        let iv = self.iv_bytes();
        let algomode = self.algomode();
        let encrypt = self.is_encrypt();

        match self.keysize() {
            16 => {
                let cipher = Aes128::new_from_slice(&key).unwrap();
                match algomode & 0x0F {
                    0 if encrypt => Self::aes_encrypt_block(&cipher, &mut block),
                    0 => Self::aes_decrypt_block(&cipher, &mut block),
                    1 if encrypt => {
                        for i in 0..16 { block[i] ^= iv[i]; }
                        Self::aes_encrypt_block(&cipher, &mut block);
                    }
                    1 => {
                        Self::aes_decrypt_block(&cipher, &mut block);
                        for i in 0..16 { block[i] ^= iv[i]; }
                    }
                    _ => {}
                }
            }
            24 => {
                let cipher = Aes192::new_from_slice(&key).unwrap();
                match algomode & 0x0F {
                    0 if encrypt => {
                        let mut ga = GenericArray::from(block);
                        cipher.encrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    0 => {
                        let mut ga = GenericArray::from(block);
                        cipher.decrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    1 if encrypt => {
                        for i in 0..16 { block[i] ^= iv[i]; }
                        let mut ga = GenericArray::from(block);
                        cipher.encrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    1 => {
                        let mut ga = GenericArray::from(block);
                        cipher.decrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                        for i in 0..16 { block[i] ^= iv[i]; }
                    }
                    _ => {}
                }
            }
            _ => {
                let cipher = Aes256::new_from_slice(&key).unwrap();
                match algomode & 0x0F {
                    0 if encrypt => {
                        let mut ga = GenericArray::from(block);
                        cipher.encrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    0 => {
                        let mut ga = GenericArray::from(block);
                        cipher.decrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    1 if encrypt => {
                        for i in 0..16 { block[i] ^= iv[i]; }
                        let mut ga = GenericArray::from(block);
                        cipher.encrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                    }
                    1 => {
                        let mut ga = GenericArray::from(block);
                        cipher.decrypt_block(&mut ga);
                        block.copy_from_slice(ga.as_slice());
                        for i in 0..16 { block[i] ^= iv[i]; }
                    }
                    _ => {}
                }
            }
        }
        block
    }

    fn process_aes_block(&mut self) {
        if self.in_buf.len() < AES_BLOCK {
            return;
        }

        let blocks: Vec<[u8; 16]> = self.in_buf.chunks_exact(AES_BLOCK)
            .map(|c| { let mut b = [0u8; 16]; b.copy_from_slice(c); b })
            .collect();

        for block in blocks {
            let result = self.process_block(block);
            self.out_buf.extend_from_slice(&result);
        }

        let consumed = (self.in_buf.len() / AES_BLOCK) * AES_BLOCK;
        self.in_buf.drain(..consumed);
    }
}

impl Peripheral for Cryp {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr & 0xF_8FFF,
            0x04 => {
                let mut sr = 0u32;
                if self.in_buf.is_empty() { sr |= 1; }
                if self.in_buf.len() < 64 { sr |= 2; }
                if self.cr & 0x8000 != 0 {
                    sr |= 0x14;
                    if self.out_buf.is_empty() {
                        self.process_aes_block();
                    }
                }
                if !self.out_buf.is_empty() { sr |= 4; }
                if self.out_buf.len() >= 64 { sr |= 8; }
                sr
            }
            0x08 => self.din,
            0x0C => {
                if self.out_buf.is_empty() && (self.cr & 0x8000) != 0 {
                    self.process_aes_block();
                }
                if self.out_buf.len() >= 4 {
                    let mut bytes = [0u8; 4];
                    bytes.copy_from_slice(&self.out_buf.drain(..4).collect::<Vec<_>>());
                    self.dout = u32::from_be_bytes(bytes);
                } else {
                    self.dout = 0;
                }
                self.dout
            }
            0x10 => self.dmacr & 0x03,
            0x14 => self.imscr & 0x03,
            0x18 => {
                let mut risr = 0x01;
                if self.cr & 0x8000 != 0 { risr |= 0x02; }
                risr
            }
            0x1C => self.imscr & 0x03,
            0x20..=0x3C => self.key[((offset - 0x20) / 4) as usize],
            0x40..=0x4C => self.iv[((offset - 0x40) / 4) as usize],
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let was_en = (self.cr & 0x8000) != 0;
                self.cr = value & 0xF_8FFF;
                if !was_en && (self.cr & 0x8000) != 0 {
                    self.sr = 0x03;
                } else if (self.cr & 0x8000) == 0 {
                    self.sr = 0x03;
                    self.out_buf.clear();
                    self.in_buf.clear();
                }
                if value & 0x4000 != 0 {
                    self.in_buf.clear();
                    self.out_buf.clear();
                }
            }
            0x08 => {
                self.din = value;
                if self.cr & 0x8000 != 0 {
                    self.in_buf.extend_from_slice(&value.to_be_bytes());
                }
            }
            0x10 => self.dmacr = value & 0x03,
            0x14 => self.imscr = value & 0x03,
            0x20..=0x3C => self.key[((offset - 0x20) / 4) as usize] = value,
            0x40..=0x4C => self.iv[((offset - 0x40) / 4) as usize] = value,
            _ => {}
        }
    }
}
