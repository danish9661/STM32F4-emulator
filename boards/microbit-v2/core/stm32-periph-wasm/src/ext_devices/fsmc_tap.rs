use crate::system::System;
use super::ExtDevice;

pub struct FsmcTapConfig {
    /// Bank index 0..3, i.e. FSMC.BANK1..BANK4.
    pub bank: usize,
}

/// Protocol-agnostic FSMC memory-bank tap. Every data-space access to the
/// tapped bank is queued for the JS hardware layer (`fsmc_take_events`), and
/// reads are answered from a JS-pushed queue (`fsmc_push_data`).
///
/// Unlike the SPI/I2C taps, an FSMC access carries an ADDRESS as well as a
/// value: memory-mapped displays in Intel-8080 mode decode one address line
/// (usually A16 or A18) as RS/DC — command vs data — so the offset is the
/// only thing that distinguishes a register write from a pixel write. Both
/// are therefore reported; see `system::fsmc_tap_push` for the encoding.
///
/// This is chip-side plumbing: it knows nothing about any device protocol.
pub struct FsmcTap {
    pub config: FsmcTapConfig,
    name: String,
}

impl FsmcTap {
    pub fn new(config: FsmcTapConfig) -> Self {
        Self { config, name: String::new() }
    }
}

impl ExtDevice<u32, u32> for FsmcTap {
    fn connect_peripheral(&mut self, peri_name: &str) -> String {
        self.name = format!("{} fsmc-tap", peri_name);
        self.name.clone()
    }

    fn read(&mut self, _sys: &System, addr: u32) -> u32 {
        let v = crate::system::fsmc_tap_data_pop(self.config.bank);
        crate::system::fsmc_tap_push(self.config.bank, false, addr, v);
        v
    }

    fn write(&mut self, _sys: &System, addr: u32, v: u32) {
        crate::system::fsmc_tap_push(self.config.bank, true, addr, v);
    }
}
