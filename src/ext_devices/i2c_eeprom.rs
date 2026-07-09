use anyhow::{Context, Result};
use serde::Deserialize;

use crate::{util, system::System};
use super::ExtDevice;

#[derive(Debug, Deserialize, Default)]
pub struct I2cEepromConfig {
    pub peripheral: String,
    pub address: u8,
    pub file: String,
    pub size: usize,
}

pub struct I2cEeprom {
    pub config: I2cEepromConfig,
    name: String,
    mem: Vec<u8>,
    addr: usize,
    addr_bytes: Vec<u8>,
    phase: EepromPhase,
}

enum EepromPhase {
    Addr,
    Data,
}

impl I2cEeprom {
    pub fn new(config: I2cEepromConfig) -> Result<Self> {
        let mut mem = util::read_file(&config.file)
            .with_context(|| format!("Failed to read {}", &config.file))?;
        mem.resize(config.size, 0);
        Ok(Self {
            config,
            name: String::new(),
            mem,
            addr: 0,
            addr_bytes: Vec::new(),
            phase: EepromPhase::Addr,
        })
    }
}

impl ExtDevice<(), u8> for I2cEeprom {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} i2c-eeprom", peri_name);
        self.name.clone()
    }

    fn reset(&mut self) {
        self.phase = EepromPhase::Addr;
        self.addr_bytes.clear();
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        self.phase = EepromPhase::Data;
        let v = self.mem.get(self.addr).copied().unwrap_or(0);
        self.addr = (self.addr + 1) % self.config.size;
        v
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        match self.phase {
            EepromPhase::Addr => {
                self.addr_bytes.push(v);
                let addr_width = if self.config.size > 256 { 2 } else { 1 };
                if self.addr_bytes.len() >= addr_width {
                    self.addr = if addr_width == 2 {
                        (self.addr_bytes[0] as usize) << 8 | self.addr_bytes[1] as usize
                    } else {
                        self.addr_bytes[0] as usize
                    };
                    self.addr = self.addr % self.config.size;
                    self.phase = EepromPhase::Data;
                }
            }
            EepromPhase::Data => {
                if self.addr < self.mem.len() {
                    self.mem[self.addr] = v;
                }
                self.addr = (self.addr + 1) % self.config.size;
            }
        }
    }
}
