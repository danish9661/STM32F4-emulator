use crate::system::{System, self};
use super::Peripheral;
use wasm_bindgen::prelude::*;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn error(s: &str);
}

// Interrupt bits for DMASR
const DMA_TS:  u32 = 1 << 0;  // Transmit status
const DMA_TPSS: u32 = 1 << 1; // Transmit process stopped
const DMA_TBUS: u32 = 1 << 2; // Transmit buffer unavailable
const DMA_TJTS: u32 = 1 << 3; // Transmit jabber timeout
const DMA_ROS:  u32 = 1 << 4;  // Receive overflow
const DMA_TUS:  u32 = 1 << 5;  // Transmit underflow
const DMA_RS:   u32 = 1 << 6;  // Receive status
const DMA_RBUS: u32 = 1 << 7;  // Receive buffer unavailable
const DMA_RPSS: u32 = 1 << 8;  // Receive process stopped
const DMA_PWTS: u32 = 1 << 9;  // Pause time status
const DMA_ETS:  u32 = 1 << 10; // Early transmit status
const DMA_FBE:  u32 = 1 << 11; // Fatal bus error
const DMA_ERS:  u32 = 1 << 12; // Early receive status
const DMA_AIS:  u32 = 1 << 14; // Abnormal interrupt summary
const DMA_NIS:  u32 = 1 << 16; // Normal interrupt summary

// Interrupt enable bits for DMAIER (same positions as DMASR)
const DMAIER_NIE: u32 = 1 << 16;
const DMAIER_AIE: u32 = 1 << 14;
const DMAIER_ERE: u32 = 1 << 12;
const DMAIER_FBE: u32 = 1 << 11;
const DMAIER_ETE: u32 = 1 << 10;
const DMAIER_RSE: u32 = 1 << 8;
const DMAIER_RBE: u32 = 1 << 7;
const DMAIER_RTE: u32 = 1 << 6;
const DMAIER_TUE: u32 = 1 << 5;
const DMAIER_ROE: u32 = 1 << 4;
const DMAIER_TJE: u32 = 1 << 3;
const DMAIER_TBU: u32 = 1 << 2;
const DMAIER_TPSE: u32 = 1 << 1;
const DMAIER_TSE: u32 = 1 << 0;

const ETH_IRQ: i32 = 61;

const WRITE1CLEAR: u32 = DMA_TS | DMA_TPSS | DMA_TBUS | DMA_TJTS | DMA_ROS | DMA_TUS
    | DMA_RS | DMA_RBUS | DMA_RPSS | DMA_PWTS | DMA_ETS | DMA_FBE | DMA_ERS;

enum BlockType { Mac, Mmc, Ptp, Dma }

pub struct EthernetMac {
    block: BlockType,
    maccr: u32, macffr: u32, machthr: u32, machtlr: u32,
    macmiiar: u32, macmiidr: u32, macfcr: u32, macvlantr: u32,
    macpmtcsr: u32, macsr: u32, macimr: u32,
    maca0hr: u32, maca0lr: u32,
    maca1hr: u32, maca1lr: u32,
    maca2hr: u32, maca2lr: u32,
    maca3hr: u32, maca3lr: u32,
    mmccr: u32, mmcrir: u32, mmctir: u32,
    mmcrimr: u32, mmctimr: u32,
    mmctgfsccr: u32, mmctgfmsccr: u32, mmctgfcr: u32,
    mmcrfcecr: u32, mmcrfaecr: u32, mmcrgufcr: u32,
    ptptscr: u32, ptpssir: u32, ptptshr: u32, ptptslr: u32,
    ptptshur: u32, ptptslur: u32, ptptsar: u32,
    ptptthr: u32, ptpttlr: u32, ptptssr: u32, ptpppscr: u32,
    dmabmr: u32, dmatpdr: u32, dmarpdr: u32,
    dmardlar: u32, dmatdlar: u32, dmasr: u32, dmaomr: u32,
    dmaier: u32, dmamfbocr: u32, dmarswtr: u32,
    dmachtdr: u32, dmachrdr: u32,
    dmachtbar: u32, dmachrbar: u32,
    rx_enabled: bool, tx_enabled: bool,
    pending_tx_done: bool, pending_rx_done: bool,
}

impl EthernetMac {
    fn new_default(block: BlockType) -> Self {
        Self {
            block,
            maccr: 0x0008000, macffr: 0, machthr: 0, machtlr: 0,
            macmiiar: 0, macmiidr: 0, macfcr: 0, macvlantr: 0,
            macpmtcsr: 0, macsr: 0, macimr: 0,
            maca0hr: 0x0010FFFF, maca0lr: 0xFFFFFFFF,
            maca1hr: 0x0000FFFF, maca1lr: 0xFFFFFFFF,
            maca2hr: 0x0000FFFF, maca2lr: 0xFFFFFFFF,
            maca3hr: 0x0000FFFF, maca3lr: 0xFFFFFFFF,
            mmccr: 0, mmcrir: 0, mmctir: 0,
            mmcrimr: 0, mmctimr: 0,
            mmctgfsccr: 0, mmctgfmsccr: 0, mmctgfcr: 0,
            mmcrfcecr: 0, mmcrfaecr: 0, mmcrgufcr: 0,
            ptptscr: 0x00002000, ptpssir: 0, ptptshr: 0, ptptslr: 0,
            ptptshur: 0, ptptslur: 0, ptptsar: 0,
            ptptthr: 0, ptpttlr: 0, ptptssr: 0, ptpppscr: 0,
            dmabmr: 0x00002101, dmatpdr: 0, dmarpdr: 0,
            dmardlar: 0, dmatdlar: 0, dmasr: 0, dmaomr: 0,
            dmaier: 0, dmamfbocr: 0, dmarswtr: 0,
            dmachtdr: 0, dmachrdr: 0,
            dmachtbar: 0, dmachrbar: 0,
            rx_enabled: false, tx_enabled: false,
            pending_tx_done: false, pending_rx_done: false,
        }
    }
}

impl EthernetMac {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        match name {
            "Ethernet_MAC" => Some(Box::new(Self::new_default(BlockType::Mac))),
            "Ethernet_MMC" => Some(Box::new(Self::new_default(BlockType::Mmc))),
            "Ethernet_PTP" => Some(Box::new(Self::new_default(BlockType::Ptp))),
            "Ethernet_DMA" => Some(Box::new(Self::new_default(BlockType::Dma))),
            _ => None,
        }
    }

    fn deliver_pending_done(&mut self, sys: &System) {
        if self.pending_tx_done && (self.dmasr & DMA_TS) == 0 {
            self.dmasr |= DMA_TS;
            self.pending_tx_done = false;
        }
        if self.pending_rx_done && (self.dmasr & DMA_RS) == 0 {
            self.dmasr |= DMA_RS;
            self.pending_rx_done = false;
        }
        self.update_interrupt(sys);
    }

    fn update_interrupt(&mut self, sys: &System) {
        let pending = self.dmasr & self.dmaier;
        let has_abnormal = pending & (DMA_TPSS | DMA_TBUS | DMA_TJTS | DMA_ROS | DMA_TUS | DMA_RBUS | DMA_RPSS | DMA_FBE) != 0;
        let has_normal = pending & (DMA_TS | DMA_RS) != 0;
        let ais = has_abnormal;
        let nis = has_normal || has_abnormal;
        let mut set = 0u32;
        if ais { set |= DMA_AIS; }
        if nis { set |= DMA_NIS; }
        self.dmasr = (self.dmasr & !(DMA_AIS | DMA_NIS)) | set;
        let masked = (self.dmasr & self.dmaier) | (if (self.dmasr & (DMA_AIS | DMA_NIS)) != 0 { 0 } else { 0 });
        if (self.dmasr & self.dmaier) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(ETH_IRQ);
        }
    }
}

impl Peripheral for EthernetMac {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match self.block {
            BlockType::Mac => match offset {
                0x00 => self.maccr, 0x04 => self.macffr,
                0x08 => self.machthr, 0x0C => self.machtlr,
                0x10 => self.macmiiar, 0x14 => self.macmiidr,
                0x18 => self.macfcr, 0x1C => self.macvlantr,
                0x2C => self.macpmtcsr, 0x34 => 0,
                0x38 => self.macsr, 0x3C => self.macimr,
                0x40 => self.maca0hr | (1 << 31), 0x44 => self.maca0lr,
                0x48 => self.maca1hr, 0x4C => self.maca1lr,
                0x50 => self.maca2hr, 0x54 => self.maca2lr,
                0x58 => self.maca3hr, 0x5C => self.maca3lr,
                _ => 0,
            },
            BlockType::Mmc => match offset {
                0x00 => self.mmccr, 0x04 => self.mmcrir,
                0x08 => self.mmctir, 0x0C => self.mmcrimr,
                0x10 => self.mmctimr,
                0x4C => self.mmctgfsccr, 0x50 => self.mmctgfmsccr,
                0x68 => self.mmctgfcr,
                0x94 => self.mmcrfcecr, 0x98 => self.mmcrfaecr,
                0xC4 => self.mmcrgufcr,
                _ => 0,
            },
            BlockType::Ptp => match offset {
                0x00 => self.ptptscr, 0x04 => self.ptpssir,
                0x08 => self.ptptshr, 0x0C => self.ptptslr,
                0x10 => self.ptptshur, 0x14 => self.ptptslur,
                0x18 => self.ptptsar,
                0x1C => self.ptptthr, 0x20 => self.ptpttlr,
                0x28 => self.ptptssr, 0x2C => self.ptpppscr,
                _ => 0,
            },
            BlockType::Dma => match offset {
                0x00 => self.dmabmr, 0x04 => self.dmatpdr,
                0x08 => self.dmarpdr, 0x0C => self.dmardlar,
                0x10 => self.dmatdlar, 0x14 => self.dmasr,
                0x18 => self.dmaomr, 0x1C => self.dmaier,
                0x20 => self.dmamfbocr, 0x24 => self.dmarswtr,
                0x48 => self.dmachtdr, 0x4C => self.dmachrdr,
                0x50 => self.dmachtbar, 0x54 => self.dmachrbar,
                _ => 0,
            },
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match self.block {
            BlockType::Mac => match offset {
                0x00 => self.maccr = value & 0x1FF7F,
                0x04 => self.macffr = value & 0x8000007F,
                0x08 => self.machthr = value,
                0x0C => self.machtlr = value,
                0x10 => {
                    if value & 1 != 0 {
                        let reg = ((value >> 6) & 0x1F) as u8;
                        let phy = ((value >> 11) & 0x1F) as u8;
                        if value & 2 == 0 {
                            let phy_data = match (phy, reg) {
                                (0, 0) => 0x3100,
                                (0, 1) => 0x786D,
                                (0, 2) => 0x0001,
                                (0, 3) => 0x0000,
                                _ => 0,
                            };
                            self.macmiidr = phy_data;
                            self.macsr |= 1 << 3;
                        }
                    }
                    self.macmiiar = value & 0xFFFF;
                }
                0x14 => self.macmiidr = value & 0xFFFF,
                0x18 => self.macfcr = value & 0x1FF0F,
                0x1C => self.macvlantr = value & 0x300FF,
                0x2C => self.macpmtcsr = (self.macpmtcsr & 0xFFFFFF20) | (value & 0x1FF),
                0x38 => self.macsr &= !(value & 0x4F8),
                0x3C => self.macimr = value & 0x250,
                0x40 => self.maca0hr = value & 0xFFFF,
                0x44 => self.maca0lr = value,
                0x48 => self.maca1hr = value,
                0x4C => self.maca1lr = value,
                0x50 => self.maca2hr = value,
                0x54 => self.maca2lr = value,
                0x58 => self.maca3hr = value,
                0x5C => self.maca3lr = value,
                _ => {}
            },
            BlockType::Mmc => match offset {
                0x00 => self.mmccr = value & 0x3F,
                0x04 => self.mmcrir = value,
                0x0C => self.mmcrimr = value,
                0x10 => self.mmctimr = value,
                _ => {}
            },
            BlockType::Ptp => match offset {
                0x00 => self.ptptscr = value & 0x7FDFF,
                0x04 => self.ptpssir = value & 0xFF,
                0x10 => self.ptptshur = value,
                0x14 => self.ptptslur = value,
                0x18 => self.ptptsar = value,
                0x1C => self.ptptthr = value,
                0x20 => self.ptpttlr = value,
                _ => {}
            },
            BlockType::Dma => match offset {
                0x00 => {
                    if value & 1 != 0 {
                        let block = std::mem::replace(&mut self.block, BlockType::Dma);
                        *self = Self::new_default(block);
                        return;
                    }
                    self.dmabmr = value & 0x7FC7FF7;
                }
                0x04 => {
                    self.dmatpdr = value;
                    if self.tx_enabled {
                        system::eth_signal_tx_poll(self.dmatdlar);
                    }
                }
                0x08 => {
                    self.dmarpdr = value;
                    if self.rx_enabled {
                        system::eth_signal_rx_poll(self.dmardlar);
                    }
                }
                0x0C => self.dmardlar = value & !3,
                0x10 => self.dmatdlar = value & !3,
                0x14 => {
                    self.dmasr &= !(value & WRITE1CLEAR);
                    self.deliver_pending_done(sys);
                }
                0x18 => {
                    self.dmaomr = value & 0x1FFFF;
                    self.rx_enabled = (value >> 1) & 1 != 0;
                    self.tx_enabled = (value >> 13) & 1 != 0;
                    if self.rx_enabled {
                        system::eth_signal_rx_poll(self.dmardlar);
                    }
                    if self.tx_enabled {
                        system::eth_signal_tx_poll(self.dmatdlar);
                    }
                }
                0x1C => {
                    self.dmaier = value & 0x1FFFF;
                    self.deliver_pending_done(sys);
                }
                0x20 => self.dmamfbocr = value & 0xFF00FF,
                0x24 => self.dmarswtr = value & 0x3FF,
                _ => {}
            },
        }
    }

    fn tick(&mut self, sys: &System) {
        if !matches!(self.block, BlockType::Dma) { return; }
        let done = system::eth_take_done();
        if done & 1 != 0 {
            self.pending_tx_done = true;
        }
        if done & 2 != 0 {
            self.pending_rx_done = true;
        }
        self.deliver_pending_done(sys);
    }
}
