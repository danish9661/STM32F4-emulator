use crate::system::System;
use super::Peripheral;

pub struct Sdio {
    power: u32,
    clkcr: u32,
    arg: u32,
    cmd: u32,
    respcmd: u32,
    resp: [u32; 4],
    dtimer: u32,
    dlen: u32,
    dctrl: u32,
    dcount: u32,
    sta: u32,
    icr: u32,
    mask: u32,
    fifocnt: u32,
    fifo: u32,
}

impl Sdio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SDIO" {
            Some(Box::new(Sdio {
                sta: 0x7E48_0000,
                ..Default::default()
            }))
        } else {
            None
        }
    }
}

impl Default for Sdio {
    fn default() -> Self {
        Self {
            power: 0, clkcr: 0, arg: 0, cmd: 0, respcmd: 0,
            resp: [0; 4], dtimer: 0, dlen: 0, dctrl: 0,
            dcount: 0, sta: 0, icr: 0, mask: 0, fifocnt: 0, fifo: 0,
        }
    }
}

impl Peripheral for Sdio {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.power,
            0x04 => self.clkcr,
            0x08 => self.arg,
            0x0C => self.cmd,
            0x10 => self.respcmd,
            0x14 => self.resp[0],
            0x18 => self.resp[1],
            0x1C => self.resp[2],
            0x20 => self.resp[3],
            0x24 => self.dtimer,
            0x28 => self.dlen,
            0x2C => self.dctrl,
            0x30 => self.dcount,
            0x34 => self.sta,
            0x38 => self.icr,
            0x3C => self.mask,
            0x48 => self.fifocnt,
            0x80 => self.fifo,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.power = value & 3,
            0x04 => self.clkcr = value & 0x3FFF,
            0x08 => self.arg = value,
            0x0C => {
                self.cmd = value & 0xFFFF;
                if value & 0x40 != 0 {
                    self.respcmd = value & 0x3F;
                    self.sta |= 1 << 6;
                    self.sta |= 1 << 10;
                }
            }
            0x24 => self.dtimer = value,
            0x28 => { self.dlen = value & 0x1FF_FFFF; self.dcount = value & 0x1FF_FFFF; }
            0x2C => {
                self.dctrl = value & 0x1F3F;
                if value & 1 != 0 {
                    self.fifo = 0;
                    self.fifocnt = self.dlen;
                    self.sta |= 1 << 1;
                    self.sta |= 1 << 3;
                    self.sta |= 1 << 11;
                }
            }
            0x38 => self.sta &= !value,
            0x3C => self.mask = value & 0x7FFF_FFFF,
            0x80 => self.fifo = value,
            _ => {}
        }
    }
}
