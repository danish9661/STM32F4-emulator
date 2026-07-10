use crate::system::System;
use super::Peripheral;

fn i2s_irq(name: &str) -> Option<i32> {
    match name {
        "I2S1" => Some(35),  // SPI1/I2S1 IRQ
        "I2S2" => Some(36),  // I2S2ext uses same IRQ 36
        "I2S3" => Some(51),  // I2S3ext uses same IRQ 51
        "I2S2ext" => Some(36),
        "I2S3ext" => Some(51),
        _ => None,
    }
}

fn i2s_base(name: &str) -> Option<u32> {
    match name {
        "I2S1" => Some(0x40013000),
        "I2S2" => Some(0x40003800),
        "I2S3" => Some(0x40003C00),
        "I2S2ext" => Some(0x40003400),
        "I2S3ext" => Some(0x40004000),
        _ => None,
    }
}

pub struct I2s {
    name: String,
    cr1: u32, cr2: u32, srm: u32, dr: u32,
    i2scfgr: u32, i2spr: u32,
    rx_buffer: u32, sr: u32, tx_buffer: u32,
    irq_num: i32,
    audio_data_counter: u16,
    base_addr: u32,
}

impl I2s {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        let irq_num = i2s_irq(name)?;
        let base_addr = i2s_base(name).unwrap_or(0);
        Some(Box::new(Self {
            name: name.to_string(),
            irq_num,
            base_addr,
            sr: 0x03, // TXE=1, RXNE=0
            ..Self::default()
        }))
    }

    fn is_master(&self) -> bool { self.i2scfgr & (1 << 9) == 0 } // MCKOE (or I2SCFG)
    fn is_mono(&self) -> bool { self.i2scfgr & (1 << 4) != 0 }
    fn datalen(&self) -> u32 {
        match (self.i2scfgr >> 5) & 0x3 {
            0 => 16, 1 => 16, 2 => 24, _ => 32,
        }
    }
    fn channel_len(&self) -> u32 {
        if self.i2scfgr & (1 << 3) != 0 { 32 } else { self.datalen().max(16) }
    }

    fn generate_audio_data(&mut self) -> u32 {
        // Generate simulated audio data - 16-bit signed sine wave pattern
        let idx = self.audio_data_counter;
        self.audio_data_counter = self.audio_data_counter.wrapping_add(1);
        // Simple sine-ish pattern using triangle wave
        let phase = idx & 0xFF;
        let sample = if phase < 128 { phase } else { 255 - phase };
        // Scale to 16-bit signed
        let sample_16 = ((sample as u32) << 7) | (sample as u32);
        // Left/right channel alternating
        if idx & 1 != 0 { sample_16 } else { sample_16 ^ 0x8000 }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        let tx_en = (self.cr2 >> 1) & 1;  // TXEIE
        let rx_en = self.cr2 & 1;          // RXNEIE
        let tx_needed = tx_en != 0 && self.sr & (1 << 1) != 0; // TXE ready
        let rx_needed = rx_en != 0 && self.sr & 1 != 0;        // RXNE ready
        if tx_needed || rx_needed {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }
}

impl Default for I2s {
    fn default() -> Self {
        Self {
            name: String::new(),
            cr1: 0, cr2: 0, srm: 0, dr: 0,
            i2scfgr: 0, i2spr: 0,
            rx_buffer: 0, sr: 0x03, tx_buffer: 0,
            irq_num: 0,
            audio_data_counter: 0,
            base_addr: 0,
        }
    }
}

impl Peripheral for I2s {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            0x0C => {
                // DR read: return rx data, clear RXNE, set TXE
                let v = self.rx_buffer;
                self.rx_buffer = 0;
                self.sr |= 1 << 1; // TXE=1
                self.sr &= !1;     // RXNE=0
                self.fire_interrupts(sys);
                v
            }
            0x10 => self.dr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value,
            0x04 => {
                self.cr2 = value;
                self.fire_interrupts(sys);
            }
            0x0C => {
                // DR write: generate audio data, fill rx_buffer
                self.tx_buffer = value;
                self.rx_buffer = self.generate_audio_data();
                self.sr |= 1;         // RXNE=1
                self.sr |= 1 << 1;    // TXE=1 (data moved to shift register)
                self.fire_interrupts(sys);
            }
            0x10 => {
                self.dr = value;
                // Simulate I2S data transfer
                self.rx_buffer = self.generate_audio_data();
                self.sr |= 1 << 1; // TXE=1
                self.sr |= 1;      // RXNE=1
                self.fire_interrupts(sys);
            }
            0x1C => self.i2scfgr = value & 0xFFF,
            0x20 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}
