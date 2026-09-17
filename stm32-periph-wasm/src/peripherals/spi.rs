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
    pub crcpr: u32,
    pub rxcrcr: u32,
    pub txcrcr: u32,
    pub rx_buffer: u32,
    pub ready_toggle: bool,
    pub i2scfgr: u32,
    pub i2spr: u32,
    wave_counter: u16,
    devices: Vec<SpiDeviceEntry>,
    last_device: Option<Rc<RefCell<dyn ExtDevice<(), u8>>>>,
    /// Live CRC shift registers (TX and RX paths). Reset to all-ones on
    /// CRCEN set and on SPE set (silicon resets the CRC calculation when
    /// the peripheral is enabled); advanced per transferred byte/word
    /// with the CRCPR polynomial while CRCEN is set. RXCRCR/TXCRCR reads
    /// return these (masked to the frame width).
    crc_tx: u16,
    crc_rx: u16,
    /// Error-flag latches (SR bits, cleared by the SR-followed-by-DR
    /// sequence — i.e. a DR access after an SR read — plus OVR's extra
    /// CR2 ERRIE path; see read_sr/read_dr below):
    /// - OVR (bit 6): a new transfer completed while rx_buffer was still
    ///   unread (previous byte never drained).
    /// - MODF (bit 5): master-mode fault — NSS pulled low while MSTR+SSM=0
    ///   (hardware slave management). The harness drives NSS via
    ///   `spi_fault_modf` (no pin layer exists to pull it).
    /// - FRE (bit 8): TI-mode frame-format error — set when FRF (CR2 bit 4)
    ///   is set and a transfer completes (header desync substitute).
    /// - CRCERR (bit 4): CRC mismatch — set when the CRCNEXT (CR1 bit 12)
    ///   transfer's received CRC word differs from the computed RX CRC.
    modf_latched: bool,
    ovr_latched: bool,
    fre_latched: bool,
    crc_err_latched: bool,
    /// Transfer-in-flight latch for SR BSY (bit 7): set on a DR write,
    /// cleared when the byte is drained by a DR read.
    bsy_flight: bool,
    /// SR read arms the DR-clear sequence: the next DR read/write clears
    /// OVR/MODF/FRE (silicon: read SR then access DR). Tracked per access
    /// so a lone DR access without a preceding SR read does NOT clear.
    sr_seen: bool,
    /// Slave-select state (harness = the master NSS pin, see
    /// `slave_select`): true = NSS asserted (low). Meaningful only with
    /// MSTR=0 + SSM=0; otherwise stored and ignored.
    slave_nss_asserted: bool,
    /// Slave TX preload (silicon TX buffer): a DR write while the slave
    /// gate is closed preloads the next MISO word instead of clocking the
    /// bus (no SCK from the slave side). Shipped out by `slave_clock`.
    slave_tx: u32,
    slave_tx_valid: bool,
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

    /// Slave mode selected (CR1 MSTR bit 2 clear): when set, transfers
    /// are NSS-gated (see `slave_gate_open`) and SCK comes from the
    /// harness (`spi_slave_clock`), not from DR writes alone.
    pub fn slave_selected(&self) -> bool { self.cr1 & (1 << 2) == 0 }

    /// Slave gate state: true when a slave transfer may proceed — MSTR=0
    /// (slave) AND NSS asserted (harness `spi_slave_select`) AND the
    /// peripheral enabled (SPE). With SSM=1 + SSI=1 the internal NSS is
    /// high (software slave management) and the gate is open without the
    /// harness (silicon: SSI drives NSS internally). Otherwise the harness
    /// NSS level decides (no pin layer exists to pull it).
    pub fn slave_gate_open(&self) -> bool {
        if self.cr1 & (1 << 2) != 0 {
            return true; // master: no gate
        }
        if self.cr1 & (1 << 9) != 0 && self.cr1 & (1 << 8) != 0 {
            return true; // SSM+SSI: internally selected
        }
        self.slave_nss_asserted
    }

    /// Harness = the SPI master: drive the slave's NSS level (true =
    /// asserted/low). Only meaningful with MSTR=0 and SSM=0 (hardware
    /// slave management); elsewhere it is stored and ignored.
    pub fn slave_select(&mut self, asserted: bool) {
        self.slave_nss_asserted = asserted;
    }

    /// Harness = the SPI master clock: shift one frame through the slave.
    /// `mosi` is the byte/word the master clocks in; returns the MISO
    /// byte/word the slave shifts out (the preloaded slave TX word — set
    /// by a DR write while the gate is closed, like silicon's TX buffer;
    /// 0xFF when nothing was preloaded). The received word lands in
    /// rx_buffer + RXNE semantics + CRC advance, exactly like a master
    /// transfer. No-op (returns 0xFF/0xFFFF, flags untouched) when the
    /// gate is closed or the peripheral is in master mode — clocking a
    /// deselected slave moves no bits, like silicon.
    pub fn slave_clock(&mut self, sys: &System, mosi: u32) -> u32 {
        if self.cr1 & (1 << 2) != 0 || !self.slave_gate_open() {
            return if self.is_16bits() { 0xFFFF } else { 0xFF };
        }
        let miso = if self.slave_tx_valid {
            self.slave_tx_valid = false;
            self.slave_tx
        } else {
            0xFF
        };
        let had_unread = self.rx_buffer != 0;
        self.rx_buffer = mosi & if self.is_16bits() { 0xFFFF } else { 0xFF };
        if had_unread {
            self.ovr_latched = true;
        }
        if self.cr2 & (1 << 4) != 0 {
            self.fre_latched = true;
        }
        self.crc_advance(mosi, self.rx_buffer);
        self.bsy_flight = true;
        miso
    }

    /// CRC-16 step with the CRCPR polynomial (MSB-first, no reflection —
    /// the STM32 SPI CRC block). `crc` is the running register, `data`
    /// the newly transferred byte.
    fn crc_step(&self, mut crc: u16, data: u8) -> u16 {
        let poly = (self.crcpr & 0xFFFF) as u16;
        crc ^= (data as u16) << 8;
        for _ in 0..8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ poly;
            } else {
                crc <<= 1;
            }
        }
        crc
    }

    /// Advance both CRC registers over one transferred frame unit (byte
    /// in 8-bit mode, both bytes MSB-first in 16-bit mode). No-op unless
    /// CRCEN (CR1 bit 13) is set. DFF width mismatch between the peers
    /// is a firmware bug, not modeled — both sides advance identically.
    fn crc_advance(&mut self, value: u32, rx: u32) {
        if self.cr1 & (1 << 13) == 0 {
            return;
        }
        if self.is_16bits() {
            for b in [(value >> 8) as u8, value as u8] {
                self.crc_tx = self.crc_step(self.crc_tx, b);
            }
            for b in [(rx >> 8) as u8, rx as u8] {
                self.crc_rx = self.crc_step(self.crc_rx, b);
            }
        } else {
            self.crc_tx = self.crc_step(self.crc_tx, value as u8);
            self.crc_rx = self.crc_step(self.crc_rx, rx as u8);
        }
        self.rxcrcr = self.crc_rx as u32;
        self.txcrcr = self.crc_tx as u32;
    }

    /// Harness = the NSS pin fault: latch a master-mode fault (MODF, SR
    /// bit 5). Silicon sets it when NSS is pulled low on a master with
    /// hardware slave management (SSM=0); the emulator has no NSS pin, so
    /// the harness drives the fault directly. Cleared by the SR→DR
    /// sequence like a real MODF (plus MSTR/SPE handling in read_dr).
    pub fn fault_modf(&mut self) {
        self.modf_latched = true;
    }

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
                // ERRIE (CR2 bit 5): any latched error flag pends the
                // same IRQ (silicon ORs MODF/OVR/FRE/CRCERR into it).
                if self.cr2 & (1 << 5) != 0
                    && (self.ovr_latched
                        || self.modf_latched
                        || self.fre_latched
                        || self.crc_err_latched)
                {
                    sys.p.nvic.borrow_mut().set_intr_pending(irq);
                }
            }
        }
    }

    /// Harness = the faulty peer: corrupt the next received CRC word so a
    /// CRCNEXT compare mismatches and latches CRCERR. (With an honest
    /// loopback the CRC always matches, so no test could observe the
    /// flag otherwise.) Corrupts the computed register itself (a bit-flip
    /// on the wire is equivalent), so the RXCRCR read path — which serves
    /// the live register while CRCEN is set — shows the corruption too.
    pub fn fault_crc(&mut self) {
        self.crc_rx ^= 0x00FF;
        if self.cr1 & (1 << 13) == 0 {
            self.rxcrcr = self.crc_rx as u32;
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
                // Live SR: TXE/RXNE toggle with the ready bit (as before);
                // BSY mirrors SPE (busy while enabled); OVR/MODF/FRE are
                // the latched error flags; CRCERR is computed on CRCNEXT
                // compares (see DR write path). Reading SR arms the
                // SR→DR clear sequence for the latched flags.
                self.sr_seen = true;
                let mut sr = if self.ready_toggle { 0b11 } else { 0 };
                // BSY follows an actual transfer in flight: set on a DR
                // write, cleared when the byte is drained by a DR read
                // (silicon: BSY while the shift register is moving). It is
                // NOT bare SPE — CR1=0xF7 has SPE set with nothing moving,
                // and the firmware SR-default check pins SR=0x3 there.
                if self.bsy_flight {
                    sr |= 1 << 7;
                }
                if self.ovr_latched { sr |= 1 << 6; }
                if self.modf_latched { sr |= 1 << 5; }
                if self.fre_latched { sr |= 1 << 8; }
                if self.crc_err_latched { sr |= 1 << 4; }
                if self.is_i2s() {
                    // I2S SR: RXNE, TXE, etc (no SPI error flags)
                    let v = if self.ready_toggle { 0b11 } else { 0 };
                    self.sr_seen = false;
                    return v;
                }
                self.fire_interrupts(sys);
                sr
            }
            0x000C => {
                let v = if self.is_i2s() {
                    // I2S mode: consume the WAV source when loaded (DMA
                    // PERIPH->MEM reads go through this path), else the
                    // synthetic generator.
                    crate::system::audio_source_next()
                        .map(|s| s as u32 & 0xFFFF)
                        .unwrap_or_else(|| self.generate_i2s_audio())
                } else {
                    self.rx_buffer
                };
                self.rx_buffer = 0;
                // Draining the byte ends the flight (BSY clears).
                self.bsy_flight = false;
                // DR read completes the SR→DR sequence: clear latched
                // error flags only if an SR read armed it (a lone DR read
                // with no preceding SR read leaves flags set, like silicon).
                // MODF additionally forces master mode off (MSTR clear +
                // SPE clear per RM0090 — the peripheral drops to slave).
                if self.sr_seen {
                    self.ovr_latched = false;
                    self.fre_latched = false;
                    self.crc_err_latched = false;
                    if self.modf_latched {
                        self.modf_latched = false;
                        self.cr1 &= !((1 << 2) | (1 << 6)); // MSTR+SPE clear
                    }
                    self.sr_seen = false;
                }
                v
            }
             0x0010 => self.crcpr,
             0x0014 => {
                 // RXCRCR reads the live RX CRC register when CRCEN is set
                 // (computed over received frames); with CRCEN clear it is
                 // plain storage (legacy spi_tft_test writes 0xAA there).
                 if self.cr1 & (1 << 13) != 0 {
                     self.crc_rx as u32
                 } else {
                     self.rxcrcr
                 }
             }
             0x0018 => {
                 if self.cr1 & (1 << 13) != 0 {
                     self.crc_tx as u32
                 } else {
                     self.txcrcr
                 }
             }
             0x001C => self.i2scfgr,
             0x0020 => self.i2spr,
            _ => 0
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            // CR1 stores verbatim (MSTR bit 2 selects master/slave, SSM/SSI
            // software-slave management, LSBFIRST/BR/CPOL/CPHA, CRCEN/CRCNEXT
            // CRC control, BIDIMODE/BIDIOE, RXONLY). Slave mode (MSTR=0) is
            // stored but transfers still run the master path: the model has
            // no external SCK driver, so NSS never gates and DR writes still
            // clock the attached device (documented substitute — firmware
            // probing MSTR=0 observes the stored bit + working transfers,
            // never slave-gated behavior). CRCEN/CRCNEXT store; the CRC
            // registers (RXCRCR/TXCRCR) are readable/writable storage, never
            // computed (documented substitute).
            0x0000 => {
                let old = self.cr1;
                self.cr1 = value;
                // CRC calculation resets when CRCEN is newly set or when
                // the peripheral is newly enabled (silicon behavior).
                if value & (1 << 13) != 0 && old & (1 << 13) == 0 {
                    self.crc_tx = 0xFFFF;
                    self.crc_rx = 0xFFFF;
                }
                if value & (1 << 6) != 0 && old & (1 << 6) == 0 {
                    self.crc_tx = 0xFFFF;
                    self.crc_rx = 0xFFFF;
                }
            }
            0x0004 => {
                self.cr2 = value;
                self.fire_interrupts(sys);
            }
             0x000C => {
                if self.is_i2s() {
                    // I2S mode: TX data push to the capture FIFO; RX mirror
                    // comes from the generator.
                    crate::system::audio_capture_push(value as u16);
                    self.rx_buffer = self.generate_i2s_audio();
                } else if self.cr1 & (1 << 2) == 0 {
                    // Slave mode (MSTR=0): DR writes PRELOAD the slave TX
                    // buffer when the gate is closed (no SCK from the slave
                    // side — silicon buffers for the master's clock). When
                    // the gate is open (SSM+SSI software select), the write
                    // still preloads (a slave never self-clocks); the byte
                    // ships on the next `slave_clock`. RXNE/BSY follow the
                    // preload so firmware polls correctly.
                    self.slave_tx = value & if self.is_16bits() { 0xFFFF } else { 0xFF };
                    self.slave_tx_valid = true;
                    self.bsy_flight = true;
                    // A DR write also completes the SR→DR sequence.
                    if self.sr_seen {
                        self.ovr_latched = false;
                        self.fre_latched = false;
                        self.crc_err_latched = false;
                        if self.modf_latched {
                            self.modf_latched = false;
                            self.cr1 &= !((1 << 2) | (1 << 6));
                        }
                        self.sr_seen = false;
                    }
                } else {
                    // CRCNEXT (CR1 bit 12): this transfer carries the CRC
                    // word, not data — compare the received word against
                    // the computed RX CRC and latch CRCERR on mismatch
                    // (silicon checks the peer's CRC here). CRCNEXT
                    // self-clears after the transfer.
                    if self.cr1 & (1 << 12) != 0 {
                        let device = self.active_device(sys);
                        let rx = if let Some(ref d) = device {
                            let mut d = d.borrow_mut();
                            if self.is_16bits() {
                                d.write(sys, (), (value >> 8) as u8);
                                let hi = d.read(sys, ()) as u32;
                                d.write(sys, (), value as u8);
                                (hi << 8) | d.read(sys, ()) as u32
                            } else {
                                d.write(sys, (), value as u8);
                                d.read(sys, ()) as u32
                            }
                        } else {
                            0xFF
                        };
                        let mask = if self.is_16bits() { 0xFFFF } else { 0xFF };
                        if (rx & mask) != (self.crc_rx as u32 & mask) {
                            self.crc_err_latched = true;
                        }
                        self.rx_buffer = rx;
                        self.cr1 &= !(1 << 12); // CRCNEXT self-clears
                        // NOTE: no SR→DR clear here (unlike the data path):
                        // the firmware must observe CRCERR with an SR read
                        // AFTER this transfer, and the transfer itself was
                        // a DR write — clearing here would wipe the flag
                        // before any SR read could arm. The next SR→DR
                        // sequence clears it normally.
                        return;
                    }
                    let had_unread = self.rx_buffer != 0;
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
                    // OVR: the previous byte was never drained. (rx_buffer
                    // is 0 both reset and after a DR read, so any nonzero
                    // residue means unread data — 0x00 data reads as no-OVR,
                    // the one honest blind spot, documented.)
                    if had_unread {
                        self.ovr_latched = true;
                    }
                    // FRE: TI-mode header desync substitute.
                    if self.cr2 & (1 << 4) != 0 {
                        self.fre_latched = true;
                    }
                    // CRC advances over the transferred TX/RX pair.
                    self.crc_advance(value, self.rx_buffer);
                    // The transfer is now in flight (BSY) until drained.
                    self.bsy_flight = true;
                    // A DR write also completes the SR→DR sequence.
                    if self.sr_seen {
                        self.ovr_latched = false;
                        self.fre_latched = false;
                        self.crc_err_latched = false;
                        if self.modf_latched {
                            self.modf_latched = false;
                            self.cr1 &= !((1 << 2) | (1 << 6));
                        }
                        self.sr_seen = false;
                    }
                }
            }
             0x0010 => self.crcpr = value & 0xFFFF,
             0x0014 => self.rxcrcr = value & 0xFFFF,
             0x0018 => self.txcrcr = value & 0xFFFF,
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
        ::std::rc::Rc::new(crate::system::WasmSystem::new())
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
