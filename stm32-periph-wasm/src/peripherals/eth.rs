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
    ptptscr: u32, ptpssir: u32,
    ptptshur: u32, ptptslur: u32, ptptsar: u32,
    ptptthr: u32, ptpttlr: u32, ptptssr: u32, ptpppscr: u32,
    dmabmr: u32, dmatpdr: u32, dmarpdr: u32,
    dmardlar: u32, dmatdlar: u32, dmasr: u32, dmaomr: u32,
    dmaier: u32, dmamfbocr: u32, dmarswtr: u32,
    dmachtdr: u32, dmachrdr: u32,
    dmachtbar: u32, dmachrbar: u32,
    rx_enabled: bool, tx_enabled: bool,
    pending_tx_done: bool, pending_rx_done: bool,
    // MDIO/MII operation latched by a MACMIIAR write with MB set; completed
    // on the next tick (firmware polls MB). Data for MW comes from MACMIIDR.
    mii_pending: bool, mii_mw: bool, mii_phy: u8, mii_reg: u8, mii_wdata: u16,
    // PHY peer (DP83848-compatible register set at address 0): BCR, AN
    // advertisement, partner ability, and the negotiated outcome. BMSR link
    // is always up (the peer is the cable); AN completes ~20000 virtual
    // instructions after enable/restart. The outcome is reported, never
    // forced into MACCR — like silicon, the driver programs FES/DM itself.
    phy_bcr: u16, phy_anar: u16, phy_anlpar: u16,
    phy_speed100: bool, phy_full: bool, phy_an_done: bool, phy_done_at: u64,
    // TX wire pacing: virtual-instruction count until the wire is free.
    // Set by eth_tx_wire_busy(len); TS completion waits for it.
    tx_busy_until: u64,
    // PTP timebase (binary 2^31 rollover): current time, update shadow,
    // target + armed flag, and the last tick the clock was advanced on.
    ptp_sec: u32, ptp_sub: u32,
    ptp_tsec: u32, ptp_tsub: u32, ptp_target_armed: bool, ptp_last: u64,
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
            ptptscr: 0x00002000, ptpssir: 0,
            ptptshur: 0, ptptslur: 0, ptptsar: 0,
            ptptthr: 0, ptpttlr: 0, ptptssr: 0, ptpppscr: 0,
            dmabmr: 0x00002101, dmatpdr: 0, dmarpdr: 0,
            dmardlar: 0, dmatdlar: 0, dmasr: 0, dmaomr: 0,
            dmaier: 0, dmamfbocr: 0, dmarswtr: 0,
            dmachtdr: 0, dmachrdr: 0,
            dmachtbar: 0, dmachrbar: 0,
            rx_enabled: false, tx_enabled: false,
            pending_tx_done: false, pending_rx_done: false,
            mii_pending: false, mii_mw: false, mii_phy: 0, mii_reg: 0, mii_wdata: 0,
            phy_bcr: 0x3100, phy_anar: 0x01E1, phy_anlpar: 0x45E1,
            phy_speed100: true, phy_full: true, phy_an_done: true, phy_done_at: 0,
            tx_busy_until: 0,
            ptp_sec: 0, ptp_sub: 0,
            ptp_tsec: 0, ptp_tsub: 0, ptp_target_armed: false, ptp_last: 0,
        }
    }

    fn block_id(&self) -> u8 {
        match self.block {
            BlockType::Mac => 0, BlockType::Mmc => 1,
            BlockType::Ptp => 2, BlockType::Dma => 3,
        }
    }

    fn mac_addr(&self) -> [u8; 6] {
        [(self.maca0hr >> 8) as u8, self.maca0hr as u8,
         (self.maca0lr >> 24) as u8, (self.maca0lr >> 16) as u8,
         (self.maca0lr >> 8) as u8, self.maca0lr as u8]
    }

    /// Live BMSR: abilities + MF-suppress + AN-complete (live) + AN-able +
    /// link (always up — the peer is the cable) + extended-status.
    fn phy_bmsr(&self) -> u16 {
        0x784D | if self.phy_an_done { 0x0020 } else { 0 }
    }

    /// DP83848-style PHYSTS (reg 0x10): link + speed + duplex + AN-done.
    fn phy_physts(&self) -> u16 {
        (1 << 0)
            | (if self.phy_speed100 { 0 } else { 1 << 1 })
            | (if self.phy_full { 1 << 2 } else { 0 })
            | (if self.phy_an_done { 1 << 4 } else { 0 })
    }

    fn phy_reg_read(&self, reg: u8) -> u16 {
        match reg {
            0 => self.phy_bcr,
            1 => self.phy_bmsr(),
            2 => 0x0007, // PHYID1 (LAN8720A OUI)
            3 => 0xC0F1, // PHYID2 (LAN8720A model + rev)
            4 => self.phy_anar,
            5 => self.phy_anlpar,
            0x10 => self.phy_physts(),
            0x1F => (1 << 0) // link, same low bits as PHYSTS
                | (if self.phy_speed100 { 0 } else { 1 << 1 })
                | (if self.phy_full { 1 << 2 } else { 0 }),
            _ => 0,
        }
    }

    /// Highest common denominator of our advertisement and the partner's:
    /// 100FD > 100HD > 10FD > 10HD (ability bits 8:5 shared by ANAR/ANLPAR).
    fn phy_resolve(&mut self) {
        let common = self.phy_anar & self.phy_anlpar & 0x01E0;
        if common & 0x0100 != 0 {
            self.phy_speed100 = true; self.phy_full = true;
        } else if common & 0x0080 != 0 {
            self.phy_speed100 = true; self.phy_full = false;
        } else if common & 0x0040 != 0 {
            self.phy_speed100 = false; self.phy_full = true;
        } else {
            self.phy_speed100 = false; self.phy_full = false;
        }
    }

    fn phy_reg_write(&mut self, reg: u8, data: u16, now: u64) {
        match reg {
            0 => {
                if data & 0x8000 != 0 {
                    // Reset: defaults back, AN re-runs to completion.
                    self.phy_bcr = 0x3100;
                    self.phy_anar = 0x01E1;
                    self.phy_speed100 = true; self.phy_full = true;
                    self.phy_an_done = false;
                    self.phy_done_at = now + 20_000;
                    return;
                }
                self.phy_bcr = data;
                if data & 0x1000 != 0 {
                    // AN enabled: restart (explicit or via the enable edge)
                    // completes later; outcome resolves then.
                    if data & 0x0200 != 0 || !self.phy_an_done {
                        self.phy_an_done = false;
                        self.phy_done_at = now + 20_000;
                    }
                } else {
                    // Forced: outcome is immediate. done_at is parked in
                    // the future so the completion check below does not
                    // instantly "resolve" AN over the forced state.
                    self.phy_an_done = false;
                    self.phy_done_at = u64::MAX;
                    self.phy_speed100 = data & 0x2000 != 0;
                    self.phy_full = data & 0x0100 != 0;
                }
            }
            4 => self.phy_anar = data & 0x1FFF,
            _ => {}
        }
    }

    /// Complete a latched MDIO operation and clear MB (called from tick, so
    /// firmware polling MB observes it set, then clear — like silicon).
    fn phy_tick(&mut self, now: u64) {
        if !self.phy_an_done && now >= self.phy_done_at {
            self.phy_an_done = true;
            self.phy_resolve();
        }
        if self.mii_pending {
            self.mii_pending = false;
            if self.mii_phy == 0 {
                if self.mii_mw {
                    self.phy_reg_write(self.mii_reg, self.mii_wdata, now);
                } else {
                    self.macmiidr = self.phy_reg_read(self.mii_reg) as u32;
                }
            } else {
                // No other PHY on the MDIO bus: reads return 0xFFFF
                // (pulled-up MDIO), writes go nowhere.
                if !self.mii_mw {
                    self.macmiidr = 0xFFFF;
                }
            }
            self.macmiiar &= !1; // MB clears on completion
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
        let now = system::instruction_count();
        // TX wire pacing: TS completion waits until the frame has
        // left the wire (set by eth_tx_wire_busy from the frame length
        // and MACCR FES). Pending stays latched until then.
        if self.pending_tx_done && (self.dmasr & DMA_TS) == 0 && now >= self.tx_busy_until {
            self.dmasr |= DMA_TS;
            self.pending_tx_done = false;
        }
        if self.pending_rx_done && (self.dmasr & DMA_RS) == 0 {
            self.dmasr |= DMA_RS;
            self.pending_rx_done = false;
        }
        self.update_interrupt(sys);
    }

    /// Time math only (no target check): shared by tick and live reads.
    fn ptp_now_advance(&mut self, now: u64) {
        if self.ptptscr & 1 == 0 {
            self.ptp_last = now;
            return;
        }
        let el = now.saturating_sub(self.ptp_last);
        self.ptp_last = now;
        if el > 0 {
            let step = match self.ptpssir & 0xFF {
                0 => 13,
                s => s as u64,
            };
            let (sub, carry) = self.ptp_sub.overflowing_add((el * step) as u32);
            self.ptp_sub = sub;
            if carry {
                self.ptp_sec = self.ptp_sec.wrapping_add(1);
            }
        }
    }

    /// Advance the PTP clock to `now` (binary 2^31 subsecond rollover)
    /// and fire the target interrupt. TSE gates everything, like silicon.
    fn ptp_advance(&mut self, sys: &System, now: u64) {
        self.ptp_now_advance(now);
        if self.ptp_target_armed
            && (self.ptp_sec > self.ptp_tsec
                || (self.ptp_sec == self.ptp_tsec && self.ptp_sub >= self.ptp_tsub))
        {
            self.ptp_target_armed = false;
            // Target-time interrupt (TSITE). Like silicon it asserts the
            // ETH line; the driver routes it to the guest handler.
            if self.ptptscr & (1 << 4) != 0 {
                sys.p.nvic.borrow_mut().set_intr_pending(ETH_IRQ);
            }
        }
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
        if (self.dmasr & self.dmaier) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(ETH_IRQ);
        }
    }
}

impl Peripheral for EthernetMac {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
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
                // Live time: advance on read so back-to-back reads
                // observe the running clock even between ticks.
                0x08 => { let n = system::instruction_count(); self.ptp_now_advance(n); self.ptp_sec }
                0x0C => { let n = system::instruction_count(); self.ptp_now_advance(n); self.ptp_sub }
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
                0x04 => self.macffr = value & 0x800007FF,
                0x08 => self.machthr = value,
                0x0C => self.machtlr = value,
                0x10 => {
                    // Latch the MDIO op; MB stays set until phy_tick
                    // completes it (firmware polls MB, like silicon).
                    if value & 1 != 0 && !self.mii_pending {
                        self.mii_pending = true;
                        self.mii_mw = value & 2 != 0;
                        self.mii_reg = ((value >> 6) & 0x1F) as u8;
                        self.mii_phy = ((value >> 11) & 0x1F) as u8;
                        self.mii_wdata = (self.macmiidr & 0xFFFF) as u16;
                    }
                    self.macmiiar = value & 0xFFFF;
                }
                0x14 => self.macmiidr = value & 0xFFFF,
                0x18 => self.macfcr = value & 0x1FF0F,
                // VLANTI is 16 bits (15:0); bit 16 selects 12/16-bit
                // compare, bit 17 inverts the match. The old mask kept
                // only the low 8 VID bits.
                0x1C => self.macvlantr = value & 0x3FFFF,
                // PMT: control bits stored; MPR/RWKPR (6:5) are
                // write-1-to-clear status. WFFRPR(31) has no filter
                // pointer state here and reads back 0.
                0x2C => {
                    let status = self.macpmtcsr & 0x60;
                    self.macpmtcsr = (value & 0x687) | (status & !(value & 0x60));
                }
                0x38 => self.macsr &= !(value & 0x4F8),
                0x3C => self.macimr = value & 0x208,
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
                0x00 => {
                    // Command bits act on write and self-clear (edge
                    // semantics): TSSTI inits time from SHUR/SLUR, TSSTU
                    // adds SHUR/SLUR, TTSARU arms the target from
                    // PTPTTHR/PTPTTLR. TSFCU/TTSARU fine-correction is
                    // stored but has no drift model behind it.
                    let now = system::instruction_count();
                    if value & (1 << 2) != 0 {
                        self.ptp_sec = self.ptptshur;
                        self.ptp_sub = self.ptptslur;
                        self.ptp_last = now;
                    }
                    if value & (1 << 3) != 0 {
                        let (s, c) = self.ptp_sub.overflowing_add(self.ptptslur);
                        self.ptp_sub = s;
                        self.ptp_sec = self.ptp_sec.wrapping_add(self.ptptshur + c as u32);
                        self.ptp_last = now;
                    }
                    if value & (1 << 5) != 0 {
                        self.ptp_tsec = self.ptptthr;
                        self.ptp_tsub = self.ptpttlr;
                        self.ptp_target_armed = true;
                    }
                    self.ptptscr = value & 0x7FDFF & !(0x2C);
                }
                0x04 => self.ptpssir = value & 0xFF,
                // PTPTSHR/SLR are read-only current time on silicon;
                // writes are ignored.
                0x10 => self.ptptshur = value,
                0x14 => self.ptptslur = value,
                0x18 => self.ptptsar = value,
                0x1C => {
                    self.ptptthr = value;
                    self.ptp_tsec = value;
                    self.ptp_target_armed = true;
                }
                0x20 => {
                    self.ptpttlr = value;
                    self.ptp_tsub = value;
                    self.ptp_target_armed = true;
                }
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
        let now = system::instruction_count();
        match self.block {
            BlockType::Mac => self.phy_tick(now),
            BlockType::Ptp => self.ptp_advance(sys, now),
            BlockType::Dma => {
                let done = system::eth_take_done();
                if done & 1 != 0 {
                    self.pending_tx_done = true;
                }
                if done & 2 != 0 {
                    self.pending_rx_done = true;
                }
                self.deliver_pending_done(sys);
            }
            BlockType::Mmc => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Frame-level helpers shared by the JS driver (accept filtering, checksum
// offload status, Wake-on-LAN, wire pacing, PTP snapshots). The driver owns
// guest descriptor memory, so these pure/model functions answer questions
// about a frame; the driver writes the descriptor bits.
// ---------------------------------------------------------------------------

/// Standard Ethernet CRC32 (reflected, poly 0xEDB88320).
fn eth_crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            crc = if crc & 1 != 0 { (crc >> 1) ^ 0xEDB8_8320 } else { crc >> 1 };
        }
    }
    !crc
}

/// Internet checksum (one's complement) over `data`.
fn ip_cksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;
    while i + 1 < data.len() {
        sum += ((data[i] as u32) << 8) | data[i + 1] as u32;
        i += 2;
    }
    if i < data.len() {
        sum += (data[i] as u32) << 8;
    }
    while sum >> 16 != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

/// Split an Ethernet frame into (l3_offset, is_ipv4). Skips one 802.1Q tag.
fn eth_l3(frame: &[u8]) -> Option<(usize, bool)> {
    if frame.len() < 14 {
        return None;
    }
    let mut off = 12;
    let mut et = ((frame[off] as u16) << 8) | frame[off + 1] as u16;
    off += 2;
    if et == 0x8100 {
        if frame.len() < 18 {
            return None;
        }
        off += 2;
        et = ((frame[off] as u16) << 8) | frame[off + 1] as u16;
        off += 2;
    }
    Some((off, et == 0x0800))
}

fn with_mac(sys: &System, f: impl FnOnce(&EthernetMac) -> u32) -> u32 {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(mac) = b.as_any_mut().downcast_mut::<EthernetMac>() {
            if mac.block_id() == 0 {
                return f(mac);
            }
        }
    }
    0
}

fn with_mac_mut(sys: &System, f: impl FnOnce(&mut EthernetMac)) {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(mac) = b.as_any_mut().downcast_mut::<EthernetMac>() {
            if mac.block_id() == 0 {
                f(mac);
                return;
            }
        }
    }
}

fn with_dma_mut(sys: &System, f: impl FnOnce(&mut EthernetMac)) {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(mac) = b.as_any_mut().downcast_mut::<EthernetMac>() {
            if mac.block_id() == 3 {
                f(mac);
                return;
            }
        }
    }
}

fn with_ptp(sys: &System, f: impl FnOnce(&EthernetMac) -> u64) -> u64 {
    for slot in &sys.p.peripherals {
        let mut b = slot.peripheral.borrow_mut();
        if let Some(mac) = b.as_any_mut().downcast_mut::<EthernetMac>() {
            if mac.block_id() == 2 {
                return f(mac);
            }
        }
    }
    0
}

impl EthernetMac {
    /// MAC destination-address filtering (perfect slots + hash table +
    /// broadcast/multicast/promiscuous rules) plus VLAN tag filtering.
    /// Mirrors the silicon accept path; the driver drops rejected frames.
    fn accept(&self, frame: &[u8]) -> bool {
        if frame.len() < 14 {
            return false; // runt
        }
        // VLAN tag gate (applies even in promiscuous mode, like silicon).
        let vlanti = self.macvlantr & 0xFFFF;
        if vlanti != 0 {
            let tagged = frame.len() >= 18 && frame[12] == 0x81 && frame[13] == 0x00;
            if !tagged {
                return false;
            }
            let vid12 = (((frame[14] & 0x0F) as u32) << 8) | frame[15] as u32;
            let full16 = ((frame[14] as u32) << 8) | frame[15] as u32;
            let want = if self.macvlantr & (1 << 16) != 0 { vid12 } else { full16 & 0xFFFF };
            let hit = want == vlanti;
            // Bit 17 inverts the match.
            let reject = if self.macvlantr & (1 << 17) != 0 { hit } else { !hit };
            if reject {
                return false;
            }
        }
        let ff = self.macffr;
        if ff & 1 != 0 {
            return true; // PR: promiscuous
        }
        let dst = &frame[0..6];
        let src = &frame[6..12];
        let bcast = dst == &[0xFF; 6];
        let mcast = dst[0] & 1 != 0;
        // ROD (receive-own disable): drop our own frames (loopback path).
        if self.maccr & (1 << 13) != 0 && src == &self.mac_addr() {
            return false;
        }
        // Perfect-match slots 0..3 (slot 0 is always enabled when the MAC
        // is programmed; slots 1..3 honor AE + MBC byte masks + SA/DA).
        let slots = [
            (self.maca0hr, self.maca0lr, true, 0u32, false),
            (self.maca1hr, self.maca1lr, self.maca1hr & (1 << 31) != 0,
                (self.maca1hr >> 24) & 0x3F, self.maca1hr & (1 << 30) != 0),
            (self.maca2hr, self.maca2lr, self.maca2hr & (1 << 31) != 0,
                (self.maca2hr >> 24) & 0x3F, self.maca2hr & (1 << 30) != 0),
            (self.maca3hr, self.maca3lr, self.maca3hr & (1 << 31) != 0,
                (self.maca3hr >> 24) & 0x3F, self.maca3hr & (1 << 30) != 0),
        ];
        let mut perfect = false;
        for (hr, lr, enabled, mbc, sa) in slots {
            if !enabled {
                continue;
            }
            let want = [
                (hr >> 8) as u8, hr as u8,
                (lr >> 24) as u8, (lr >> 16) as u8, (lr >> 8) as u8, lr as u8,
            ];
            // MBC bit x set = byte x is don't-care (slots 1..3; slot 0
            // passes mbc=0 so every byte compares).
            let cmp = if sa { src } else { dst };
            if cmp.iter().zip(want.iter()).enumerate()
                .all(|(i, (&a, &b))| (mbc >> i) & 1 != 0 || a == b)
            {
                perfect = true;
                break;
            }
        }
        // Hash table (HU unicast / HM multicast): CRC32 upper 6 bits.
        let hash_hit = || {
            let idx = (eth_crc32(dst) >> 26) as u32;
            let bit = if idx < 32 {
                (self.machtlr >> idx) & 1
            } else {
                (self.machthr >> (idx - 32)) & 1
            };
            bit != 0
        };
        if bcast {
            return ff & (1 << 5) == 0; // DBF disables broadcast
        }
        if mcast {
            return perfect
                || ff & (1 << 4) != 0 // PM: pass all multicast
                || (ff & (1 << 2) != 0 && hash_hit()); // HM
        }
        let hit = perfect || (ff & (1 << 1) != 0 && hash_hit()); // HU
        // DAIF inverts unicast destination filtering.
        if ff & (1 << 3) != 0 { !hit } else { hit }
    }
}

/// Driver entry: does the MAC accept this received frame?
pub fn eth_mac_accept(sys: &System, frame: &[u8]) -> bool {
    with_mac(sys, |m| m.accept(frame) as u32) != 0
}

/// Driver entry: RX checksum status for descriptor bits.
/// Bit 0 = has IPv4, bit 1 = IP header OK, bit 2 = has TCP/UDP/ICMP,
/// bit 3 = L4 checksum OK. Maps to RDES0 IPHCE(7)/PCE(0).
pub fn eth_rx_csum_status(frame: &[u8]) -> u32 {
    let Some((l3, is_ip)) = eth_l3(frame) else { return 0 };
    if !is_ip || frame.len() < l3 + 20 {
        return 0;
    }
    let ihl = ((frame[l3] & 0x0F) as usize) * 4;
    if ihl < 20 || frame.len() < l3 + ihl {
        return 0;
    }
    let mut st = 1u32;
    if ip_cksum(&frame[l3..l3 + ihl]) == 0 {
        st |= 2;
    }
    let proto = frame[l3 + 9];
    let l4 = l3 + ihl;
    let ok = match proto {
        6 => {
            // TCP over pseudo-header.
            if frame.len() < l4 + 20 {
                return st;
            }
            let tcp_len = frame.len() - l4;
            let mut pseudo = Vec::with_capacity(12 + tcp_len);
            pseudo.extend_from_slice(&frame[l3 + 12..l3 + 20]);
            pseudo.push(0);
            pseudo.push(6);
            pseudo.push((tcp_len >> 8) as u8);
            pseudo.push((tcp_len & 0xFF) as u8);
            pseudo.extend_from_slice(&frame[l4..]);
            st |= 4;
            ip_cksum(&pseudo) == 0
        }
        17 => {
            if frame.len() < l4 + 8 {
                return st;
            }
            let udp_len = (((frame[l4 + 4] as usize) << 8) | frame[l4 + 5] as usize).max(8);
            if frame[l4 + 6] == 0 && frame[l4 + 7] == 0 {
                st |= 4; // checksum field zero = none transmitted; not an error
                return st | 8;
            }
            let take = (l4 + udp_len).min(frame.len());
            let mut pseudo = Vec::with_capacity(12 + take - l4);
            pseudo.extend_from_slice(&frame[l3 + 12..l3 + 20]);
            pseudo.push(0);
            pseudo.push(17);
            pseudo.push((udp_len >> 8) as u8);
            pseudo.push((udp_len & 0xFF) as u8);
            pseudo.extend_from_slice(&frame[l4..take]);
            st |= 4;
            ip_cksum(&pseudo) == 0
        }
        1 => {
            if frame.len() < l4 + 4 {
                return st;
            }
            st |= 4;
            ip_cksum(&frame[l4..]) == 0
        }
        _ => return st,
    };
    if ok {
        st |= 8;
    }
    st
}

/// Driver entry: Wake-on-LAN inspection. Returns bit 0 if a magic packet
/// (6xFF + 16x our MAC) is seen; latches MPR when MPE is set and pends
/// the PMT interrupt (IRQ 62) when PMTIM is unmasked. Wakeup-frame CRC
/// matching is not modeled: RWKPR never sets (documented gap).
pub fn eth_check_wol(sys: &System, frame: &[u8]) -> u32 {
    let mut magic = false;
    with_mac(sys, |m| {
        let mac = m.mac_addr();
        if frame.len() >= 102 {
            'scan: for off in 0..=(frame.len() - 102) {
                if frame[off..off + 6] != [0xFF; 6] {
                    continue;
                }
                for r in 0..16 {
                    if frame[off + 6 + r * 6..off + 12 + r * 6] != mac {
                        continue 'scan;
                    }
                }
                magic = true;
                break;
            }
        }
        0
    });
    if magic {
        with_mac_mut(sys, |m| {
            if m.macpmtcsr & 0x2 != 0 {
                m.macpmtcsr |= 0x20; // MPR
                if m.macimr & 0x8 != 0 {
                    sys.p.nvic.borrow_mut().set_intr_pending(62);
                }
            }
        });
        1
    } else {
        0
    }
}

/// Driver entry: arm TX wire pacing for a frame of `len` bytes.
/// Wire time = (len + preamble/SFD + IFG) at MACCR FES speed on a
/// 168 MHz virtual clock; TS completion waits for it.
pub fn eth_tx_wire_busy(sys: &System, len: u32) {
    let mhz: u64 = if with_mac(sys, |m| (m.maccr >> 14) & 1) != 0 { 100 } else { 10 };
    let inst = (len as u64 + 20) * 8 * 168 / mhz;
    let until = system::instruction_count() + inst;
    with_dma_mut(sys, |d| d.tx_busy_until = until);
}

/// Driver entry: current MACCR (FES/DM/LM/ROD checks).
pub fn eth_get_maccr(sys: &System) -> u32 {
    with_mac(sys, |m| m.maccr)
}

/// Driver entry: loopback active (MACCR LM). The driver re-injects the
/// transmitted frame into RX; ROD filtering happens in accept().
pub fn eth_loopback_tx(sys: &System) -> bool {
    with_mac(sys, |m| (m.maccr >> 12) & 1) != 0
}

/// Driver entry: PTP TSE enabled (gates RX/TX snapshots).
pub fn eth_ptp_tse(sys: &System) -> bool {
    with_ptp(sys, |p| (p.ptptscr & 1) as u64) != 0
}

/// Driver entry: PTP seconds / subseconds for TDES6/7 + RDES6/7 snapshots.
pub fn eth_ptp_sec(sys: &System) -> u32 {
    with_ptp(sys, |p| p.ptp_sec as u64) as u32
}
pub fn eth_ptp_sub(sys: &System) -> u32 {
    with_ptp(sys, |p| p.ptp_sub as u64) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::test_dummy_system;

    static ETH_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    const MAC: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];

    fn mac_with_addr() -> EthernetMac {
        let mut m = EthernetMac::new_default(BlockType::Mac);
        // Wire MAC 02:00:00:00:00:01: HR = first two octets (MSB
        // first per IEEE), LR = remaining four.
        m.maca0hr = 0x0200;
        m.maca0lr = 0x00000001;
        assert_eq!(m.mac_addr(), MAC);
        m
    }

    fn frame_to(dst: &[u8; 6], payload_len: usize) -> Vec<u8> {
        let mut f = vec![0u8; 14 + payload_len];
        f[0..6].copy_from_slice(dst);
        f[6..12].copy_from_slice(&MAC);
        f[12] = 0x08;
        f[13] = 0x00;
        f
    }

    /// Minimal IPv4/UDP frame with correct checksums (for csum tests).
    fn udp_frame() -> Vec<u8> {
        let mut f = frame_to(&MAC, 28);
        f[14] = 0x45; // version/IHL
        f[16] = 0; f[17] = 28; // total length
        f[22] = 64; // TTL
        f[23] = 17; // UDP
        f[26..30].copy_from_slice(&[192, 168, 4, 1]); // src
        f[30..34].copy_from_slice(&[192, 168, 4, 2]); // dst
        let ihl = 20;
        let ck = ip_cksum(&f[14..14 + ihl]);
        f[24] = (ck >> 8) as u8; f[25] = (ck & 0xFF) as u8;
        let uo = 34;
        f[uo] = 0; f[uo + 1] = 53; // sport
        f[uo + 2] = 0xC0; f[uo + 3] = 1; // dport
        f[uo + 4] = 0; f[uo + 5] = 8; // len
        // UDP checksum over pseudo-header.
        let mut pseudo = Vec::new();
        pseudo.extend_from_slice(&f[26..30]);
        pseudo.extend_from_slice(&f[30..34]);
        pseudo.extend_from_slice(&[0, 17, 0, 8]);
        pseudo.extend_from_slice(&f[uo..uo + 8]);
        let uck = ip_cksum(&pseudo);
        f[uo + 6] = (uck >> 8) as u8; f[uo + 7] = (uck & 0xFF) as u8;
        f
    }

    #[test]
    fn crc32_standard_vector() {
        assert_eq!(eth_crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn accept_unicast_broadcast_other() {
        let m = mac_with_addr();
        assert!(m.accept(&frame_to(&MAC, 20)));
        assert!(m.accept(&frame_to(&[0xFF; 6], 20)));
        assert!(!m.accept(&frame_to(&[0x02, 0, 0, 0, 0, 0x02], 20)));
        assert!(!m.accept(&[0u8; 10])); // runt
    }

    #[test]
    fn accept_promiscuous_and_dbroadcast() {
        let mut m = mac_with_addr();
        m.macffr = 1; // PR
        assert!(m.accept(&frame_to(&[0x02, 0, 0, 0, 0, 0x02], 20)));
        let mut m2 = mac_with_addr();
        m2.macffr = 1 << 5; // DBF
        assert!(!m2.accept(&frame_to(&[0xFF; 6], 20)));
    }

    #[test]
    fn accept_hash_multicast() {
        let group = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x07];
        let idx = (eth_crc32(&group) >> 26) as u32;
        let mut m = mac_with_addr();
        // Program the hash bit for the group, enable HM only.
        if idx < 32 { m.machtlr |= 1 << idx; } else { m.machthr |= 1 << (idx - 32); }
        m.macffr = 1 << 2; // HM
        assert!(m.accept(&frame_to(&group, 20)));
        let other = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x08];
        let idx2 = (eth_crc32(&other) >> 26) as u32;
        if idx2 != idx {
            assert!(!m.accept(&frame_to(&other, 20)));
        }
        // PM passes all multicast regardless of the table.
        m.macffr = 1 << 4;
        assert!(m.accept(&frame_to(&other, 20)));
    }

    #[test]
    fn accept_perfect_slot_with_mask() {
        let mut m = mac_with_addr();
        // Slot 1: AE + MBC bit 0 (want[0], the first wire octet,
        // don't-care). want = [xx,09,02,00,00,09].
        m.maca1hr = (1 << 31) | (1 << 24) | 0x0009;
        m.maca1lr = 0x02000009;
        let mut hit = frame_to(&[0x55, 0x09, 0x02, 0x00, 0x00, 0x09], 20);
        assert!(m.accept(&hit));
        hit[0] = 0xAA; // masked octet: still passes
        assert!(m.accept(&hit));
        hit[1] = 0xFF; // unmasked octet must match
        assert!(!m.accept(&hit));
    }

    #[test]
    fn accept_vlan_gate() {
        let mut m = mac_with_addr();
        m.macvlantr = 7; // VLANTI=7, 16-bit compare
        assert!(!m.accept(&frame_to(&MAC, 20))); // untagged dropped
        let mut tagged = frame_to(&MAC, 24);
        tagged[12] = 0x81; tagged[13] = 0x00;
        tagged[14] = 0x00; tagged[15] = 0x07;
        assert!(m.accept(&tagged));
        tagged[15] = 0x08;
        assert!(!m.accept(&tagged));
    }

    #[test]
    fn csum_good_bad() {
        let f = udp_frame();
        assert_eq!(eth_rx_csum_status(&f), 0b1111);
        let mut bad_ip = f.clone();
        bad_ip[24] ^= 0xFF;
        assert_eq!(eth_rx_csum_status(&bad_ip) & 0b0011, 0b0001);
        let mut bad_udp = f.clone();
        bad_udp[40] ^= 0xFF;
        let st = eth_rx_csum_status(&bad_udp);
        assert_eq!(st & 0b1100, 0b0100);
    }

    #[test]
    fn phy_an_restart_and_force() {
        let mut m = EthernetMac::new_default(BlockType::Mac);
        assert_eq!(m.phy_reg_read(1) & 0x24, 0x24); // AN done + link
        // Restart AN: done clears, completes later with full resolution.
        m.phy_reg_write(0, 0x3300, 1000);
        assert_eq!(m.phy_an_done, false);
        m.phy_tick(1000);
        assert_eq!(m.phy_an_done, false);
        m.phy_tick(1000 + 20_000);
        assert_eq!(m.phy_an_done, true);
        assert!(m.phy_speed100 && m.phy_full);
        // Partner advertises 10HD only -> resolve down.
        m.phy_anlpar = 0x0021;
        m.phy_reg_write(0, 0x3300, 50000);
        m.phy_tick(50000 + 20_000);
        assert!(!m.phy_speed100 && !m.phy_full);
        // Forced 10 half: immediate, AN-done stays clear.
        m.phy_reg_write(0, 0x0000, 80000);
        assert!(!m.phy_speed100 && !m.phy_full && !m.phy_an_done);
        // PHYSTS mirrors the outcome (bit1 = 10M, bit2 = full).
        assert_eq!(m.phy_reg_read(0x10) & 0x07, 0x03);
    }

    #[test]
    fn wol_magic_and_exports() {
        let _g = ETH_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        // Program MACA0 + MPE through the real MMIO write arms.
        sys.p.write(&sys, 0x4002_8044, 4, 0x00000001); // MACA0LR
        sys.p.write(&sys, 0x4002_8040, 4, 0x0200); // MACA0HR
        sys.p.write(&sys, 0x4002_802C, 4, 0x2); // PMTCTL MPE
        // Magic packet: 6xFF + 16x MAC, MPE armed.
        let mut f = vec![0u8; 14 + 110];
        f[0..6].copy_from_slice(&MAC);
        f[6..12].copy_from_slice(&MAC);
        f[12] = 0x08; f[13] = 0x00;
        f[14..20].copy_from_slice(&[0xFF; 6]);
        for r in 0..16 {
            f[20 + r * 6..26 + r * 6].copy_from_slice(&MAC);
        }
        assert_eq!(eth_check_wol(&sys, &f), 1);
        assert_eq!(sys.p.read(&sys, 0x4002_802C, 4) & 0x20, 0x20); // MPR latched
        // W1C clears MPR.
        sys.p.write(&sys, 0x4002_802C, 4, 0x20 | 0x2);
        assert_eq!(sys.p.read(&sys, 0x4002_802C, 4) & 0x20, 0);
        // Non-magic gets nothing.
        assert_eq!(eth_check_wol(&sys, &f[..60]), 0);
        // Accept still works through the export (slot0 programmed).
        assert!(eth_mac_accept(&sys, &f[..60]));
        // MACCR round-trips through the export.
        sys.p.write(&sys, 0x4002_8000, 4, 1 << 14);
        assert_eq!(eth_get_maccr(&sys) & (1 << 14), 1 << 14);
        assert!(!eth_loopback_tx(&sys));
    }
}
