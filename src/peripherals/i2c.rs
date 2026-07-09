// SPDX-License-Identifier: GPL-3.0-or-later

use crate::{system::System, ext_devices::{I2cDeviceEntry, ExtDevices}};
use super::Peripheral;

#[derive(Debug, Clone, Copy, PartialEq)]
enum I2cState {
    Idle,
    StartSent,
    AddrSent { is_read: bool },
    Active { is_read: bool },
}

pub struct I2c {
    name: String,
    devices: Vec<I2cDeviceEntry>,
    active_device: Option<usize>,
    cr1: u32,
    sr1: u32,
    sr2: u32,
    dr: u32,
    state: I2cState,
    sr1_read_with_addr: bool,
}

impl I2c {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("I2C") {
            let devices = ext_devices.find_i2c_devices(name);
            let name = if let Some(d) = devices.first() {
                d.name.clone()
            } else {
                name.to_string()
            };
            for d in &devices {
                d.device.borrow_mut().connect_peripheral(&d.name);
            }
            Some(Box::new(Self {
                name,
                devices,
                state: I2cState::Idle,
                ..Default::default()
            }))
        } else {
            None
        }
    }

    fn reset(&mut self) {
        self.sr1 = 0;
        self.sr2 = 0;
        self.dr = 0;
        self.active_device = None;
        self.state = I2cState::Idle;
        self.sr1_read_with_addr = false;
    }
}

impl Default for I2c {
    fn default() -> Self {
        Self {
            name: String::new(),
            devices: Vec::new(),
            active_device: None,
            cr1: 0,
            sr1: 0,
            sr2: 0,
            dr: 0,
            state: I2cState::Idle,
            sr1_read_with_addr: false,
        }
    }
}

impl Peripheral for I2c {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x10 => {
                let v = self.dr;
                self.sr1 &= !(1 << 5);
                if let Some(idx) = self.active_device {
                    if let I2cState::Active { is_read: true } = self.state {
                        let mut d = self.devices[idx].device.borrow_mut();
                        self.dr = d.read(sys, ()) as u32;
                        self.sr1 |= 1 << 5;
                    }
                }
                v
            }
            0x14 => {
                self.sr1_read_with_addr = (self.sr1 & (1 << 1)) != 0;
                self.sr1
            }
            0x18 => {
                if self.sr1_read_with_addr {
                    self.sr1 &= !(1 << 1);
                    self.sr1_read_with_addr = false;
                    let is_read = match self.state {
                        I2cState::AddrSent { is_read } => {
                            self.state = I2cState::Active { is_read };
                            is_read
                        }
                        _ => false,
                    };
                    if is_read {
                        self.sr1 |= 1 << 5;
                    } else {
                        self.sr1 |= 1 << 6;
                    }
                }
                self.sr2
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let prev_start = self.cr1 & (1 << 8);
                let prev_pe = self.cr1 & 1;
                self.cr1 = value;

                if value & (1 << 15) != 0 {
                    self.reset();
                    return;
                }
                if prev_pe != 0 && value & 1 == 0 {
                    self.reset();
                    return;
                }

                let start = value & (1 << 8);
                let stop = value & (1 << 9);

                if start != 0 && prev_start == 0 {
                    self.state = I2cState::StartSent;
                    self.sr1 = 1;
                    self.sr2 = (1 << 0) | (1 << 1);
                    self.active_device = None;
                }

                if stop != 0 {
                    if matches!(self.state, I2cState::Active { .. } | I2cState::AddrSent { .. }) {
                        self.reset();
                    }
                }
            }
            0x10 => {
                match self.state {
                    I2cState::StartSent => {
                        let addr = (value >> 1) & 0x7F;
                        let is_read = (value & 1) != 0;

                        let found = self.devices.iter().position(|d| d.address == addr as u8);

                        if let Some(idx) = found {
                            self.active_device = Some(idx);
                            self.sr1 = 1 << 1;
                            self.sr2 = (1 << 0) | (1 << 1);
                            if is_read {
                                let mut d = self.devices[idx].device.borrow_mut();
                                self.dr = d.read(sys, ()) as u32;
                            }
                            self.state = I2cState::AddrSent { is_read };
                        } else {
                            debug!("{} NACK addr=0x{:02x}", self.name, addr);
                            self.sr1 = 1 << 9;
                            self.sr2 = (1 << 0) | (1 << 1);
                            self.state = I2cState::Idle;
                        }
                    }
                    I2cState::Active { is_read: false } => {
                        if let Some(idx) = self.active_device {
                            let mut d = self.devices[idx].device.borrow_mut();
                            d.write(sys, (), value as u8);
                        }
                        self.sr1 |= 1 << 6;
                    }
                    I2cState::AddrSent { .. } | I2cState::Active { is_read: true } => {
                        debug!("{} DR write in unexpected state {:?}", self.name, self.state);
                    }
                    I2cState::Idle => {}
                }
            }
            _ => {}
        }
    }
}
