use crate::system::{System, DmaTransfer, DmaDir, set_dma_intr_info};
use super::Peripheral;

#[derive(Default)]
pub struct Dma {
    name: String,
    streams: [Stream; 8],
}

impl Dma {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name.starts_with("DMA") {
            Some(Box::new(Self { name: name.to_string(), ..Self::default() }))
        } else {
            None
        }
    }

    fn stream_irq(&self, stream: usize) -> i32 {
        match self.name.as_str() {
            "DMA1" => 11 + stream as i32,
            "DMA2" => 56 + stream as i32,
            _ => panic!("Unknown DMA controller: {}", self.name),
        }
    }
}

impl Peripheral for Dma {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                let mut v = 0u32;
                for i in 0..=3 { v |= (self.streams[i].status as u32) << (i * 6); }
                v
            }
            0x04 => {
                let mut v = 0u32;
                for i in 0..=3 { v |= (self.streams[i + 4].status as u32) << (i * 6); }
                v
            }
            _ => {
                match Access::from_offset(offset) {
                    Access::StreamReg(i, o) => self.streams[i].read(&self.name, _sys, o),
                    _ => 0,
                }
            }
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x08 => {
                for i in 0..=3 {
                    let mask = (value >> (i * 6)) & 0x1F;
                    if mask != 0 { self.streams[i].status &= !(mask as u8); }
                }
            }
            0x0C => {
                for i in 0..=3 {
                    let mask = (value >> (i * 6)) & 0x1F;
                    if mask != 0 { self.streams[i + 4].status &= !(mask as u8); }
                }
            }
            _ => {
                match Access::from_offset(offset) {
                    Access::StreamReg(i, o) => {
                        self.streams[i].write(&self.name, sys, i, o, value, self.stream_irq(i));
                    }
                    _ => {}
                }
            }
        }
    }
}

#[derive(Default)]
struct Stream {
    cr: u32,
    next_cr: Option<u32>,
    ndtr: u32,
    par: u32,
    m0ar: u32,
    m1ar: u32,
    fcr: u32,
    status: u8,
}

impl Stream {
    fn channel(&self) -> u8 { ((self.cr >> 25) & 0b111) as u8 }
    fn dir(&self) -> Dir {
        match (self.cr >> 6) & 0b11 {
            0b00 => Dir::Read,
            0b01 => Dir::Write,
            0b10 => Dir::MemCopy,
            _ => Dir::Invalid,
        }
    }
    fn dma_size(bits: u32) -> usize {
        match bits { 0b00 => 1, 0b01 => 2, _ => 4 }
    }
    fn data_size(&self) -> usize {
        let msize = Self::dma_size((self.cr >> 13) & 0b11);
        let psize = Self::dma_size((self.cr >> 11) & 0b11);
        std::cmp::max(msize, psize) * self.ndtr as usize
    }
    fn data_addr(&self) -> u32 {
        if (self.cr >> 19) & 1 != 0 { self.m1ar } else { self.m0ar }
    }

    fn do_xfer(&self, name: &str, sys: &System, stream_idx: usize) {
        let dir = self.dir();
        let data_addr = self.data_addr();
        let size = self.data_size();
        let peri_addr = self.par;

        let dma_dir = match dir {
            Dir::Read => DmaDir::Read,
            Dir::Write => DmaDir::Write,
            Dir::MemCopy => DmaDir::MemCopy,
            Dir::Invalid => return,
        };

        let (src, dst) = match dir {
            Dir::Read => (peri_addr, self.m0ar),
            Dir::Write => (data_addr, peri_addr),
            Dir::MemCopy => (peri_addr, data_addr),
            Dir::Invalid => (0, 0),
        };

        let peripheral = dir != Dir::MemCopy;
        sys.queue_dma_transfer(DmaTransfer {
            direction: dma_dir,
            stream_idx,
            dma_name: name.to_string(),
            src, dst,
            size,
            peri_addr,
            peripheral,
        });

        log::debug!("{} queued DMA xfer stream={} dir={:?} src=0x{:08x} dst=0x{:08x} size={}",
            name, stream_idx, dir, src, dst, size);
    }

    fn read(&mut self, _name: &str, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x0000 => {
                let v = self.cr;
                if let Some(next_cr) = self.next_cr.take() {
                    self.cr = next_cr;
                }
                if self.dir() == Dir::Write && self.data_size() == 0 {
                    self.next_cr = Some(self.cr ^ 1);
                }
                v
            }
            0x0004 => self.ndtr,
            0x0008 => self.par,
            0x000c => self.m0ar,
            0x0010 => self.m1ar,
            0x0014 => self.fcr,
            _ => 0,
        }
    }

    fn write(&mut self, name: &str, sys: &System, stream_idx: usize, offset: u32, mut value: u32, irq: i32) {
        match offset {
            0x0000 => {
                self.cr = value;
                if value & 1 != 0 {
                    self.do_xfer(name, sys, stream_idx);
                    self.status |= 1 << 4; // TCIF
                    self.status |= 1 << 3; // HTIF
                    let tcie = ((value >> 4) & 1) as u8;
                    let htie = ((value >> 3) & 1) as u8;
                    let teie = ((value >> 2) & 1) as u8;
                    let flags = tcie | (htie << 1) | (teie << 2);
                    set_dma_intr_info(stream_idx, irq, flags);
                    if tcie != 0 || htie != 0 || teie != 0 {
                        sys.p.nvic.borrow_mut().set_intr_pending(irq);
                    }
                    value &= !1;
                    self.ndtr = 0;
                    self.next_cr = Some(value);
                }
            }
            0x0004 => { self.ndtr = value & 0xFFFF; }
            0x0008 => { self.par = value; }
            0x000c => { self.m0ar = value; }
            0x0010 => { self.m1ar = value; }
            0x0014 => { self.fcr = value; }
            _ => {}
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Dir { Read, Write, MemCopy, Invalid }

enum Access { Reg(u32), StreamReg(usize, u32) }

impl Access {
    fn from_offset(offset: u32) -> Self {
        if offset < 0x10 { Access::Reg(offset) }
        else {
            let stride = 0x18;
            let start = 0x10;
            let o = offset - start;
            Access::StreamReg((o / stride) as usize, o % stride)
        }
    }
}
