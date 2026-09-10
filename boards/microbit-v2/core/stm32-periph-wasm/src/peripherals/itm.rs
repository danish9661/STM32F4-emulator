use crate::system::{System, get_uart_output};
use super::Peripheral;

/// ITM stimulus console (0xE0000000 block): STIM0 writes go to the UART
/// output when TCR.ITMENA (bit 0) and TER[0] are both set — the standard
/// ITM_SendChar debug-console idiom, sunk into our terminal instead of a
/// (nonexistent) SWO trace port. Everything else is RAZ/WI. With tracing
/// disabled (reset state) all stimulus writes are ignored, exactly like a
/// debugger-disconnected target. One byte per write is delivered (the low
/// byte; the bus layer merges sub-word accesses there), matching the
/// byte-oriented SendChar pattern.
pub struct Itm {
    ter: u32,
    tpr: u32,
    tcr: u32,
}

impl Itm {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "ITM" {
            Some(Box::new(Self { ter: 0, tpr: 0, tcr: 0 }))
        } else {
            None
        }
    }

    fn enabled(&self) -> bool {
        self.tcr & 1 != 0 && self.ter & 1 != 0
    }
}

impl Peripheral for Itm {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            // STIM0: nonzero (FIFO ready) only when tracing is on, which is
            // what CMSIS ITM_SendChar polls on before writing.
            0x0 => {
                if self.enabled() {
                    1
                } else {
                    0
                }
            }
            0xE00 => self.ter,
            0xE40 => self.tpr,
            0xE80 => self.tcr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x0 => {
                if self.enabled() {
                    get_uart_output().lock().unwrap().push(value as u8 as char);
                }
            }
            0xE00 => self.ter = value,
            0xE40 => self.tpr = value,
            0xE80 => self.tcr = value,
            _ => {}
        }
    }
}
