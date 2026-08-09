use crate::system::System;
use super::Peripheral;

fn i2s_irq(name: &str) -> Option<i32> {
    match name {
        "I2S1" => Some(35),
        "I2S2" => Some(36),
        "I2S3" => Some(51),
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
    cr1: u32, cr2: u32,
    sr: u32, dr: u32,
    crcpr: u32, rxcrcr: u32, txcrcr: u32,
    i2scfgr: u32, i2spr: u32,
    rx_buffer: u32, tx_buffer: u32,
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
            sr: 0x03,
            ..Self::default()
        }))
    }

    fn generate_audio_data(&mut self) -> u32 {
        let idx = self.audio_data_counter;
        self.audio_data_counter = self.audio_data_counter.wrapping_add(1);
        let phase = idx & 0xFF;
        let sample = if phase < 128 { phase } else { 255 - phase };
        let sample_16 = ((sample as u32) << 7) | (sample as u32);
        if idx & 1 != 0 { sample_16 } else { sample_16 ^ 0x8000 }
    }

    fn update_sr_and_fire(&mut self, sys: &System, new_bits: u32) {
        self.sr |= new_bits;
        let txeie = (self.cr2 >> 1) & 1;
        let rxneie = self.cr2 & 1;
        let errie = (self.cr2 >> 2) & 1;
        let tx_irq = txeie != 0 && self.sr & (1 << 1) != 0;
        let rx_irq = rxneie != 0 && self.sr & 1 != 0;
        let err_irq = errie != 0 && self.sr & ((1 << 3) | (1 << 4) | (1 << 7)) != 0;
        if tx_irq || rx_irq || err_irq {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }
}

impl Default for I2s {
    fn default() -> Self {
        Self {
            name: String::new(),
            cr1: 0, cr2: 0,
            sr: 0x03, dr: 0,
            crcpr: 0, rxcrcr: 0, txcrcr: 0,
            i2scfgr: 0, i2spr: 0,
            rx_buffer: 0, tx_buffer: 0,
            irq_num: 0,
            audio_data_counter: 0,
            base_addr: 0,
        }
    }
}

impl Peripheral for I2s {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.sr,
            0x0C => {
                let v = self.rx_buffer;
                self.rx_buffer = 0;
                self.update_sr_and_fire(sys, 1 << 1);
                self.sr &= !1;
                v
            }
            0x10 => self.crcpr,
            0x14 => self.rxcrcr,
            0x18 => self.txcrcr,
            0x1C => self.i2scfgr,
            0x20 => self.i2spr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.cr1 = value;
            }
            0x04 => {
                self.cr2 = value & 0xFFF7;
                self.update_sr_and_fire(sys, 0);
            }
            0x0C => {
                self.tx_buffer = value;
                self.rx_buffer = self.generate_audio_data();
                self.update_sr_and_fire(sys, 3);
            }
            0x10 => {
                self.crcpr = value & 0xFFFF;
            }
            0x14 => {
                self.rxcrcr = value & 0xFFFF;
            }
            0x18 => {
                self.txcrcr = value & 0xFFFF;
            }
            0x1C => {
                self.i2scfgr = value & 0xFFF;
            }
            0x20 => {
                self.i2spr = value & 0x3FF;
            }
            _ => {}
        }
    }
}
