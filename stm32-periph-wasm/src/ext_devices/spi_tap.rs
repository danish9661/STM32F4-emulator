use crate::system::System;
use super::ExtDevice;

pub struct SpiTapConfig {
    pub peripheral: String,
    pub cs: Option<String>,
}

/// Protocol-agnostic SPI bus tap. Every byte written by the SPI controller
/// while this device is CS-selected is queued for the JS hardware layer
/// (`spi_take_events`); bytes pushed from JS (`spi_push_miso`) are returned
/// on the MISO line. CS edges are queued too, preserving byte/CS ordering.
/// This is chip-side plumbing: it knows nothing about any device protocol.
pub struct SpiTap {
    pub config: SpiTapConfig,
    name: String,
}

impl SpiTap {
    pub fn new(config: SpiTapConfig) -> Self {
        Self { config, name: String::new() }
    }
}

impl ExtDevice<(), u8> for SpiTap {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} spi-tap", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, _addr: ()) -> u8 {
        crate::system::spi_tap_miso_pop(&self.config.peripheral)
    }

    fn write(&mut self, _sys: &System, _addr: (), v: u8) {
        crate::system::spi_tap_push_byte(&self.config.peripheral, v);
    }

    fn cs_changed(&mut self, _sys: &System, asserted: bool) {
        crate::system::spi_tap_push_cs(&self.config.peripheral, asserted);
    }
}
