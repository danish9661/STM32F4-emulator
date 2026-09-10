use crate::system::System;
use super::{ExtDevice, parse_pin};

pub struct SpiTapConfig {
    pub peripheral: String,
    pub cs: Option<String>,
    pub dc: Option<String>,
}

/// Protocol-agnostic SPI bus tap. Every byte written by the SPI controller
/// while this device is CS-selected is queued for the JS hardware layer
/// (`spi_take_events`); bytes pushed from JS (`spi_push_miso`) are returned
/// on the MISO line. CS edges are queued too, preserving byte/CS ordering.
/// An optional DC (data/command) pin is sampled at each DR write and its
/// level is reported in bit 29 of the byte event, so the JS device can
/// distinguish command bytes from display data (TFT, etc.).
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

    fn write(&mut self, sys: &System, _addr: (), v: u8) {
        let mut ev = v as u32 & 0xFF;
        if let Some(dc) = &self.config.dc {
            let (port, pin) = parse_pin(dc);
            let level = sys.p.gpio.borrow_mut().read_port(sys, port);
            if (level >> pin) & 1 != 0 {
                ev |= 1 << 29;
            }
        }
        crate::system::spi_tap_push_byte(&self.config.peripheral, ev);
    }

    fn cs_changed(&mut self, _sys: &System, asserted: bool) {
        crate::system::spi_tap_push_cs(&self.config.peripheral, asserted);
    }
}
