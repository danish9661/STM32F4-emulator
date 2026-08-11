use crate::system::System;
use super::ExtDevice;

pub struct I2cTapConfig {
    pub peripheral: String,
    pub address: u8,
}

/// Protocol-agnostic I2C slave. The address is acknowledged on the bus like
/// any other registered slave; bytes the master writes are queued for the JS
/// hardware layer (`i2c_take_tx`), and bytes pushed from JS (`i2c_push_rx`)
/// are returned on master reads (0xFF when the queue is empty). Chip-side
/// plumbing only — no device protocol knowledge.
pub struct I2cTap {
    pub config: I2cTapConfig,
    name: String,
}

impl I2cTap {
    pub fn new(config: I2cTapConfig) -> Self {
        Self { config, name: String::new() }
    }
}

impl ExtDevice<(), u8> for I2cTap {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} i2c-tap", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        crate::system::i2c_tap_rx_pop(&self.config.peripheral)
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        crate::system::i2c_tap_push_tx(&self.config.peripheral, v);
    }
}
