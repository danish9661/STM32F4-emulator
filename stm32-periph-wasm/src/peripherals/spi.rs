use crate::{system::System, ext_devices::{ExtDevice, SpiDeviceEntry, ExtDevices}};
use super::{Peripheral, gpio::{GpioPorts, Pin}};
use std::{rc::Rc, cell::RefCell};

#[derive(Default)]
pub struct Spi {
    pub name: String,
    pub cr1: u32,
    pub cr2: u32,
    pub srm: u32,
    pub dr: u32,
    pub rxcrcr: u32,
    pub txcrcr: u32,
    pub rx_buffer: u32,
    pub ready_toggle: bool,
    pub i2scfgr: u32,
    pub i2spr: u32,
    wave_counter: u16,
    devices: Vec<SpiDeviceEntry>,
    last_device: Option<Rc<RefCell<dyn ExtDevice<(), u8>>>>,
}

impl Spi {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("SPI") {
            let mut devices = ext_devices.find_serial_devices(name);
            if devices.is_empty() {
                if let Some(d) = ext_devices.find_serial_device(name) {
                    let n = d.borrow_mut().connect_peripheral(name);
                    devices.push(SpiDeviceEntry { cs: None, device: d, name: n.clone() });
                }
            } else {
                for d in &mut devices {
                    d.name = d.device.borrow_mut().connect_peripheral(&d.name);
                }
            }
            Some(Box::new(Self { name: name.to_string(), devices, ..Default::default() }))
        } else { None }
    }

    pub fn is_16bits(&self) -> bool { self.cr1 & (1 << 11) != 0 }
    fn is_i2s(&self) -> bool { self.i2scfgr & 1 != 0 } // I2SMOD

    fn active_device(&mut self, sys: &System) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        let selected = self.sel_state(sys);
        selected.0.clone()
    }

    fn sel_state(&self, sys: &System) -> (Option<Rc<RefCell<dyn ExtDevice<(), u8>>>>, bool) {
        let mut gpio = sys.p.gpio.borrow_mut();
        for d in &self.devices {
            let low = match d.cs {
                Some((port, pin)) => (gpio.read_port(sys, port) >> pin) & 1 == 0,
                None => true,
            };
            if low { return (Some(d.device.clone()), true); }
        }
        // Fall back to a CS-less device (e.g. usart_probe) only when no other
        // device exists; a device with a CS pin must be explicitly selected.
        let first = match self.devices.first() {
            Some(f) => f,
            None => return (None, false),
        };
        if first.cs.is_none() {
            (Some(first.device.clone()), true)
        } else {
            (None, false)
        }
    }

    /// Register a GPIO write callback for each device's CS pin so CS edges
    /// (asserted/deasserted) reach the device immediately, exactly like the
    /// software-SPI path. CS is active-low; GPIO 1 = deselected.
pub fn register_cs_callbacks(&mut self, gpio: &mut GpioPorts) {
        for entry in &self.devices {
            if let Some((port, pin)) = entry.cs {
                let d = entry.device.clone();
                let pin = Pin::new(port, pin);
                gpio.add_write_callback(pin, move |sys, value| {
                    d.borrow_mut().cs_changed(sys, !value);
                });
            }
        }
    }

    fn generate_i2s_audio(&mut self) -> u32 {
        let idx = self.wave_counter;
        self.wave_counter = self.wave_counter.wrapping_add(1);
        let phase = idx & 0xFF;
        let sample = if phase < 128 { phase } else { 255 - phase };
        let sample_16 = ((sample as u32) << 7) | (sample as u32);
        if idx & 1 != 0 { sample_16 } else { sample_16 ^ 0x8000 }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.name.starts_with("SPI") && !self.is_i2s() {
            // SPI mode: fire when TXEIE or RXNEIE enabled and ready
            let irq = match self.name.as_str() {
                "SPI1" | "SPI4" => Some(35),
                "SPI2" | "SPI5" => Some(36),
                "SPI3" | "SPI6" => Some(51),
                _ => None,
            };
            if let Some(irq) = irq {
                let txeie = (self.cr2 >> 1) & 1;
                let rxneie = self.cr2 & 1;
                if (txeie != 0 || rxneie != 0) && self.ready_toggle {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }
}

impl Peripheral for Spi {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => self.cr1,
            0x0004 => self.cr2,
            0x0008 => {
                self.ready_toggle = !self.ready_toggle;
                if self.is_i2s() {
                    // I2S SR: RXNE, TXE, etc
                    (if self.ready_toggle { 0b11 } else { 0 })
                } else {
                    let sr = if self.ready_toggle { 0b11 } else { 0 };
                    self.fire_interrupts(sys);
                    sr
                }
            }
            0x000C => {
                let v = if self.is_i2s() {
                    // I2S mode: generate received audio data
                    self.generate_i2s_audio()
                } else {
                    self.rx_buffer
                };
                self.rx_buffer = 0;
                v
            }
             0x0010 => self.dr,
             0x0014 => self.rxcrcr,
             0x0018 => self.txcrcr,
             0x001C => self.i2scfgr,
             0x0020 => self.i2spr,
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x0000 => self.cr1 = value,
            0x0004 => {
                self.cr2 = value;
                self.fire_interrupts(sys);
            }
             0x000C => {
                if self.is_i2s() {
                    // I2S mode: write data generates receive data
                    self.rx_buffer = self.generate_i2s_audio();
                } else {
                    let device = self.active_device(sys);
                    if let Some(ref d) = device {
                        let mut d = d.borrow_mut();
                        if self.is_16bits() {
                            d.write(sys, (), (value >> 8) as u8);
                            self.rx_buffer = (d.read(sys, ()) as u32) << 8;
                            d.write(sys, (), value as u8);
                            self.rx_buffer |= d.read(sys, ()) as u32;
                        } else {
                            let v = value as u8;
                            d.write(sys, (), v);
                            self.rx_buffer = d.read(sys, ()) as u32;
                        }
                    } else {
                        self.rx_buffer = 0xFF;
                    }
                }
            }
             0x0010 => self.dr = value,
             0x0014 => self.rxcrcr = value,
             0x0018 => self.txcrcr = value,
             0x001C => self.i2scfgr = value & 0xFFF,
             0x0020 => self.i2spr = value & 0x3FF,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sys_with_flash() -> ::std::rc::Rc<crate::system::System> {
        let mut ext = crate::system::get_ext_devices().lock().unwrap();
        let flash = crate::ext_devices::spi_flash::SpiFlash::new(
            crate::ext_devices::spi_flash::SpiFlashConfig {
                peripheral: "SPI3".into(), jedec_id: 0xEF4015,
                content: vec![0xFF; 0x1000], size: 0x1000, cs: Some("PB12".into()),
            });
        ext.spi_flashes.push(std::rc::Rc::new(std::cell::RefCell::new(flash)));
        drop(ext);
        crate::system::test_dummy_system()
    }

    fn w(sys: &crate::system::System, addr: u32, v: u32) {
        sys.p.write(sys, addr, 4, v);
    }
    fn r(sys: &crate::system::System, addr: u32) -> u32 {
        sys.p.read(sys, addr, 4)
    }

    #[test]
    fn firmware_flow_via_gpio_cs() {
        let sys = sys_with_flash();
        // GPIOB MODER: PB12 output, PB13-15 AF5
        w(&sys, 0x40020400, (1u32 << 24) | (2 << 26) | (2 << 28) | (2 << 30));
        w(&sys, 0x40020414, 1 << 12); // cs high
        w(&sys, 0x40003C00, 0x364);   // SPI3 CR1
        w(&sys, 0x40003C00, 0x364 | 0x40); // SPE
        // JEDEC
        w(&sys, 0x40020414, 1 << (12+16)); // cs low
        w(&sys, 0x40003C0C, 0x9F);
        let _dummy = r(&sys, 0x40003C0C); // MISO during cmd byte
        w(&sys, 0x40003C0C, 0);
        let j0 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let j1 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let j2 = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12); // cs high
        assert_eq!((j0 & 0xFF, j1 & 0xFF, j2 & 0xFF), (0xEF, 0x40, 0x15), "jedec");
        // WEL
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x06);
        w(&sys, 0x40020414, 1 << 12);
        // status
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x05);
        let _st_dummy = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0x00);
        let st = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12);
        assert_eq!(st & 0xFF, 0x02, "WEL");
        // page program 3 bytes at 0x10
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x02);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x10);
        w(&sys, 0x40003C0C, b'A' as u32);
        w(&sys, 0x40003C0C, b'B' as u32);
        w(&sys, 0x40003C0C, b'C' as u32);
        w(&sys, 0x40020414, 1 << 12); // cs high -> commit, WEL cleared
        // status: WEL cleared after program
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x05);
        let _st2_dummy = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0x00);
        let st2 = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12);
        assert_eq!(st2 & 0xFF, 0x00, "WEL cleared after program");
        // read back
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x03);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x10);
        let _b_dummy = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let b0 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let b1 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let b2 = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12);
        assert_eq!((b0 & 0xFF, b1 & 0xFF, b2 & 0xFF), (b'A' as u32, b'B' as u32, b'C' as u32), "readback");
        // WREN + sector erase 4k, then verify content is 0xFF again
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x06);
        w(&sys, 0x40020414, 1 << 12);
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x20);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40020414, 1 << 12);
        // status: WEL cleared after erase
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x05);
        let _st3_dummy = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0x00);
        let st3 = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12);
        assert_eq!(st3 & 0xFF, 0x00, "WEL cleared after erase");
        // read back: all 0xFF
        w(&sys, 0x40020414, 1 << (12+16));
        w(&sys, 0x40003C0C, 0x03);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x00);
        w(&sys, 0x40003C0C, 0x10);
        let _e_dummy = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let e0 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let e1 = r(&sys, 0x40003C0C);
        w(&sys, 0x40003C0C, 0);
        let e2 = r(&sys, 0x40003C0C);
        w(&sys, 0x40020414, 1 << 12);
        assert_eq!((e0 & 0xFF, e1 & 0xFF, e2 & 0xFF), (0xFF, 0xFF, 0xFF), "erased");
    }
}
