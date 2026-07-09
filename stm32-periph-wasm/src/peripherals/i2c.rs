use crate::{system::System, ext_devices::{ExtDevices, ExtDevice, I2cDeviceEntry}};
use super::Peripheral;
use std::{rc::Rc, cell::RefCell, collections::VecDeque};

#[derive(Default, Clone)]
pub struct I2c {
    name: String,
    cr1: u32,
    cr2: u32,
    oar1: u32,
    oar2: u32,
    dr: u32,
    sr1: u32,
    sr2: u32,
    ccr: u32,
    trise: u32,
    fltr: u32,
    devices: Vec<I2cDeviceEntry>,
    tx_buf: VecDeque<u8>,
    rx_buf: VecDeque<u8>,
    addr: u8,
    state: I2cState,
}

#[derive(Clone, PartialEq)]
enum I2cState { Idle, Addr, Data }

impl Default for I2cState { fn default() -> Self { I2cState::Idle } }

impl I2c {
    pub fn new(name: &str, ext_devices: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("I2C") {
            let devices = ext_devices.find_i2c_devices(name);
            Some(Box::new(Self { name: name.to_string(), devices, ..Default::default() }))
        } else { None }
    }

    fn find_device(&self) -> Option<Rc<RefCell<dyn ExtDevice<(), u8>>>> {
        for d in &self.devices {
            if d.address == self.addr {
                return Some(d.device.clone());
            }
        }
        None
    }
}

impl Peripheral for I2c {
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr1,
            0x04 => self.cr2,
            0x08 => self.oar1,
            0x0C => self.oar2,
            0x14 => {
                if let Some(dev) = self.find_device() {
                    let mut d = dev.borrow_mut();
                    d.read(sys, ());
                    let v = self.rx_buf.pop_front().unwrap_or(0);
                    self.sr1 &= !0x40;
                    v as u32
                } else { 0 }
            }
            0x18 => self.sr1,
            0x1C => self.sr2,
            0x20 => self.ccr,
            0x24 => self.trise,
            0x28 => self.fltr,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.cr1 = value,
            0x04 => self.cr2 = value,
            0x08 => self.oar1 = value,
            0x0C => self.oar2 = value,
            0x14 => {
                self.dr = value & 0xFF;
                if self.state == I2cState::Idle || self.state == I2cState::Addr {
                    self.addr = ((value & 0xFF) >> 1) as u8;
                    if let Some(dev) = self.find_device() {
                        dev.borrow_mut().reset();
                    }
                    self.state = I2cState::Data;
                    self.sr1 |= 0x02;
                    self.sr1 |= 0x80;
                } else {
                    if let Some(dev) = self.find_device() {
                        let mut d = dev.borrow_mut();
                        d.write(sys, (), value as u8);
                        let rx = d.read(sys, ());
                        self.rx_buf.push_back(rx);
                    }
                    self.sr1 |= 0x40;
                    self.sr1 |= 0x80;
                }
            }
            0x18 => self.sr1 &= !(value & 0xFFFF),
            0x20 => self.ccr = value,
            0x24 => self.trise = value,
            0x28 => self.fltr = value,
            _ => {}
        }
    }
}
