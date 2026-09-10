use crate::system::System;
use super::ExtDevice;

/// Pointer-addressed register file (DS3231 RTC style).
///
/// Chip-side plumbing only: the first byte of every write transaction is the
/// register pointer, subsequent write bytes land at `ptr++`, and reads
/// return `regs[ptr++]`. The pointer survives across address matches (real
/// RTCs read from the last-accessed register), and `reset()` only re-arms
/// the pointer-byte phase for the next write transaction.
pub struct I2cRegFileConfig {
    pub peripheral: String,
    pub address: u8,
    pub size: usize,
    pub init: Vec<u8>,
}

pub struct I2cRegFile {
    pub config: I2cRegFileConfig,
    name: String,
    regs: Vec<u8>,
    ptr: usize,
    phase: RegFilePhase,
}

enum RegFilePhase {
    Addr,
    Data,
}

impl I2cRegFile {
    pub fn new(config: I2cRegFileConfig) -> Self {
        let mut regs = config.init.clone();
        regs.resize(config.size, 0);
        I2cRegFile {
            config,
            name: String::new(),
            regs,
            ptr: 0,
            phase: RegFilePhase::Addr,
        }
    }

    pub fn get(&self, offset: usize) -> u8 {
        self.regs.get(offset).copied().unwrap_or(0)
    }

    pub fn set(&mut self, offset: usize, v: u8) {
        if offset < self.regs.len() {
            self.regs[offset] = v;
        }
    }
}

impl ExtDevice<(), u8> for I2cRegFile {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} i2c-regfile", peri_name);
        self.name.clone()
    }

    fn reset(&mut self) {
        self.phase = RegFilePhase::Addr;
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        self.phase = RegFilePhase::Data;
        let v = self.regs.get(self.ptr).copied().unwrap_or(0);
        self.ptr = (self.ptr + 1) % self.config.size.max(1);
        v
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        match self.phase {
            RegFilePhase::Addr => {
                self.ptr = (v as usize) % self.config.size.max(1);
                self.phase = RegFilePhase::Data;
            }
            RegFilePhase::Data => {
                if self.ptr < self.regs.len() {
                    self.regs[self.ptr] = v;
                }
                self.ptr = (self.ptr + 1) % self.config.size.max(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev() -> I2cRegFile {
        I2cRegFile::new(I2cRegFileConfig {
            peripheral: "I2C1".into(),
            address: 0x68,
            size: 8,
            init: vec![0x11, 0x22, 0x33],
        })
    }

    #[test]
    fn pointer_write_then_read_transaction_starts_at_pointer() {
        let mut d = dev();
        d.reset();
        d.write(&crate::system::test_dummy_system(), (), 0x02); // pointer
        d.reset(); // new read transaction (address match)
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x33); // init[2]
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x00); // regs[3]
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x00); // auto-inc
    }

    #[test]
    fn data_write_auto_increments_from_pointer() {
        let mut d = dev();
        d.reset();
        d.write(&crate::system::test_dummy_system(), (), 0x01); // pointer
        d.write(&crate::system::test_dummy_system(), (), 0xEE); // regs[1]
        d.write(&crate::system::test_dummy_system(), (), 0xDD); // regs[2]
        assert_eq!(d.get(1), 0xEE);
        assert_eq!(d.get(2), 0xDD);
        d.reset();
        // reads continue from the register after the last write (ptr=3)
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x00);
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x00);
    }

    #[test]
    fn out_of_range_pointer_clamps_to_size() {
        let mut d = dev();
        d.reset();
        d.write(&crate::system::test_dummy_system(), (), 0xFF); // % 8 = 7
        d.write(&crate::system::test_dummy_system(), (), 0x99);
        assert_eq!(d.get(7), 0x99);
        d.reset();
        // pointer wrapped to 0 after the data write
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x11);
        assert_eq!(d.read(&crate::system::test_dummy_system(), ()), 0x22);
    }
}
