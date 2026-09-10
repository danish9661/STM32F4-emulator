use crate::system::System;
use super::Peripheral;

const SDIO_IRQ: i32 = 49;

#[derive(Clone, Copy, PartialEq)]
enum SdState { Idle, Ready, Ident, Stby, Tran }

pub struct Sdio {
    power: u32, clkcr: u32, arg: u32, cmd: u32, respcmd: u32,
    resp: [u32; 4], dtimer: u32, dlen: u32, dctrl: u32, dcount: u32,
    sta: u32, icr: u32, mask: u32, fifocnt: u32, fifo: u32,
    sd_state: SdState, rca: u16, data_xfer_active: bool,
}

impl Sdio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "SDIO" {
            Some(Box::new(Sdio { sta: 0x7E48_0000, ..Default::default() }))
        } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.sta & self.mask != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(SDIO_IRQ);
        }
    }
}

impl Default for Sdio {
    fn default() -> Self {
        Self {
            power: 0, clkcr: 0, arg: 0, cmd: 0, respcmd: 0,
            resp: [0; 4], dtimer: 0, dlen: 0, dctrl: 0, dcount: 0,
            sta: 0, icr: 0, mask: 0, fifocnt: 0, fifo: 0,
            sd_state: SdState::Idle, rca: 0, data_xfer_active: false,
        }
    }
}

impl Peripheral for Sdio {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
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

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => self.power = value & 3,
            0x04 => self.clkcr = value & 0x3FFF,
            0x08 => self.arg = value,
            0x0C => {
                self.cmd = value & 0xFFFF;
                if value & 0x40 != 0 {
                    let cmd_index = value as u8 & 0x3F;
                    let wait_type = (value >> 6) & 3;
                    self.respcmd = cmd_index as u32;
                    self.resp = [0; 4];

                    match (self.sd_state, cmd_index) {
                        (SdState::Idle, 0)  => { self.resp[0] = 0x00FF_FF80; }
                        (SdState::Idle, 2)  => { self.resp[0] = 0x00FF_FF80; self.sd_state = SdState::Ident; }
                        (SdState::Idle, 5)  => { self.resp[0] = 0x20_FF80; }
                        (SdState::Idle, 55) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Idle, _)  if cmd_index >= 41 && cmd_index <= 52 =>
                            { self.resp[0] = 0x50_FF80; self.sd_state = SdState::Ready; }
                        (SdState::Ready, 41) => { self.resp[0] = 0x50_FF80; }
                        (SdState::Ready, 55) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Ready, 5)  => { self.resp[0] = 0x20_FF80; }
                        (SdState::Ready, 2)  => { self.resp[0] = 0x00FF_FF80; self.sd_state = SdState::Ident; }
                        (SdState::Ident, 2)  => { self.resp[0] = 0x00FF_FF80; }
                        (SdState::Ident, 3)  => { self.rca = 0x01D0; self.resp[0] = 0x01D0_0000; self.sd_state = SdState::Stby; }
                        (SdState::Ident, 9)  => { self.resp[0] = 0x10_FFFF; self.resp[1] = 0x7F_FF80_9A; }
                        (SdState::Ident, 10) => { self.resp[0] = 0x10_FFFF; self.resp[1] = 0x7F_FF80_9A; }
                        (SdState::Stby, 7)  => { if (self.arg >> 16) as u16 == self.rca { self.resp[0] = 0x1D0_0000; self.sd_state = SdState::Tran; } }
                        (SdState::Stby, 3)  => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 7)  => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 13) => { self.resp[0] = 0x100; }
                        (SdState::Tran, 16) => { self.resp[0] = 0x200; }
                        (SdState::Tran, 17) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 18) => { self.resp[0] = 0x1D0_0000; }
                        (SdState::Tran, 55) => { self.resp[0] = 0x1D0_0000; }
                        (_, 8) => { self.resp[0] = 0x1AA; }
                        _ => {}
                    }

                    self.sta |= 1 << 6;
                    if wait_type != 0 { self.sta |= 1 << 10; }

                    if cmd_index == 17 || cmd_index == 18 {
                        self.data_xfer_active = true;
                        self.dcount = self.dlen;
                        self.sta |= (1 << 1) | (1 << 3) | (1 << 11);
                        self.fifocnt = self.dlen.min(512);
                    }

                    self.fire_interrupts(sys);
                }
            }
            0x24 => self.dtimer = value,
            0x28 => { self.dlen = value & 0x1FF_FFFF; self.dcount = value & 0x1FF_FFFF; }
            0x2C => {
                self.dctrl = value & 0x1F3F;
                if value & 1 != 0 {
                    self.sta &= !0x3F;
                    self.data_xfer_active = false;
                    self.fifocnt = 0;
                    self.sta |= 1 << 3;
                    self.sta |= 1 << 5;
                    self.fire_interrupts(sys);
                }
            }
            0x38 => {
                self.sta &= !value;
            }
            0x3C => {
                self.mask = value & 0x7FFF_FFFF;
                self.fire_interrupts(sys);
            }
            0x80 => self.fifo = value,
            _ => {}
        }
    }
}
