use crate::system::System;
use super::Peripheral;

pub struct Iwdg {
    kr: u32,
    pr: u32,
    rlr: u32,
    sr: u32,
}

impl Iwdg {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "IWDG" {
            Some(Box::new(Iwdg { kr: 0, pr: 0, rlr: 0xFFF, sr: 0 }))
        } else {
            None
        }
    }
}

impl Peripheral for Iwdg {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x04 => self.pr,
            0x08 => self.rlr,
            0x0C => self.sr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                self.kr = value;
                    match value & 0xFFFF {
                        0x5555 => {}
                        0xAAAA => {}
                        0xCCCC => {}
                        _ => {}
                    }
            }
            0x04 => {
                if self.kr == 0x5555 {
                    self.pr = value & 7;
                }
            }
            0x08 => {
                if self.kr == 0x5555 {
                    self.rlr = value & 0xFFF;
                }
            }
            _ => {}
        }
    }
}
