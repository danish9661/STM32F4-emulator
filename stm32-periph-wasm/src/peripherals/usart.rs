use crate::system::{System, get_uart_output};
use crate::ext_devices::ExtDevices;
use super::Peripheral;

const USART_IRQ_OFFSET: i32 = 37;

fn usart_irq(name: &str) -> Option<i32> {
    match name {
        "USART1" => Some(37),
        "USART2" => Some(38),
        "USART3" => Some(39),
        "UART4" => Some(52),
        "UART5" => Some(53),
        "USART6" => Some(71),
        "UART7" => Some(82),
        "UART8" => Some(83),
        _ => None,
    }
}

pub struct Usart {
    sr: u32,
    dr: u32,
    brr: u32,
    cr1: u32,
    cr2: u32,
    cr3: u32,
    gtp: u32,
    tx_data: Vec<u8>,
    rx_buf: Vec<u8>,
    irq_num: i32,
    /// CTS input level from the harness (true = asserted, peer ready).
    /// Default asserted: with hardware flow control off it is never
    /// sampled, and `uart_cts` starts asserted so enabling CTSE never
    /// spuriously blocks TX.
    cts_asserted: bool,
    /// Framing/parity fault injection for the next received byte
    /// (harness = the noisy wire; see `uart_fault_rx`). Bit 0 = FE,
    /// bit 1 = PE. Consumed by the next rx_byte().
    rx_fault: u8,
}

impl Usart {
    pub fn new(name: &str, _ext: &ExtDevices) -> Option<Box<dyn Peripheral>> {
        usart_irq(name).map(|irq| {
            Box::new(Self {
                sr: 0x00C0,
                dr: 0, brr: 0, cr1: 0, cr2: 0, cr3: 0, gtp: 0,
                tx_data: Vec::new(),
                rx_buf: Vec::new(),
                irq_num: irq,
                cts_asserted: true,
                rx_fault: 0,
            }) as Box<dyn Peripheral>
        })
    }

    /// LIN mode active (CR2 LINEN bit 14): the model stores the bit but
    /// does not speak LIN — TX/RX run the plain async path (documented
    /// substitute; firmware enabling LINEN observes stored-bit + working
    /// async, never LIN break/sync framing).
    pub fn lin_active(&self) -> bool {
        self.cr2 & (1 << 14) != 0
    }

    /// Smartcard mode active (CR3 SCEN bit 5 + NACK bit 4): stored only —
    /// no T=0/T=1 protocol, no guard-time handling (GTPR stores too).
    pub fn sc_active(&self) -> bool {
        self.cr3 & (1 << 5) != 0
    }

    /// IrDA mode active (CR3 IREN bit 1, IRLP bit 2 = low-power): stored
    /// only — no 3/16-pulse modulation (the TX byte still sinks to the
    /// UART console unmodulated; documented substitute).
    pub fn irda_active(&self) -> bool {
        self.cr3 & (1 << 1) != 0
    }

    fn update_interrupt(&mut self, sys: &System) {
        let mut pending = false;
        if self.cr1 & (1 << 6) != 0 && self.sr & (1 << 6) != 0 { pending = true; } // TCIE + TC
        if self.cr1 & (1 << 7) != 0 && self.sr & (1 << 7) != 0 { pending = true; } // TXEIE + TXE
        if self.cr1 & (1 << 5) != 0 && self.sr & (1 << 5) != 0 { pending = true; } // RXNEIE + RXNE
        // EIE (CR3 bit 0): framing/overrun/noise faults pend the IRQ.
        if self.cr3 & 1 != 0
            && self.sr & ((1 << 0) | (1 << 1) | (1 << 2) | (1 << 3)) != 0
        {
            pending = true; // PE/FE/NE/ORE + EIE
        }
        // CTSIE (CR3 bit 10): CTS edge pends the IRQ.
        if self.cr3 & (1 << 10) != 0 && self.sr & (1 << 10) != 0 {
            pending = true; // CTSIF + CTSIE
        }
        if pending {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    /// CTS input level for hardware flow control (CR3 CTSE bit 9 gates
    /// TX, RTSE bit 8 drives RTS — see write_dr). Harness = the peer.
    pub fn set_cts(&mut self, asserted: bool) {
        self.cts_asserted = asserted;
    }

    /// Live CTS flag (SR bit 9, CTSF): the sampled CTS input level.
    /// Set on a CTS edge (cleared by writing it 0 — silicon clears CTSF
    /// by software sequence; here any SR write of the bit clears it and
    /// re-arms the edge detector). Read-only live level otherwise.
    fn cts_flag(&self) -> bool {
        self.sr & (1 << 10) != 0
    }

    /// Arm a framing (FE, SR bit 1) and/or parity (PE, SR bit 0) fault on
    /// the next received byte (harness = the noisy wire). The byte still
    /// lands in DR with RXNE set (silicon delivers data + flags); PE only
    /// latches when PCE (CR1 bit 10) is enabled.
    pub fn fault_rx(&mut self, fe: bool, pe: bool) {
        if fe {
            self.rx_fault |= 1;
        }
        if pe {
            self.rx_fault |= 2;
        }
    }

    fn read_dr(&mut self, sys: &System) -> u32 {
        let dr = if !self.rx_buf.is_empty() {
            self.rx_buf.remove(0) as u32
        } else {
            self.dr
        };
        if self.rx_buf.is_empty() {
            self.sr &= !(1 << 5); // Clear RXNE only when buffer empty
        }
        // A DR read clears latched PE/FE/NE (silicon: read SR then DR;
        // the SR read is implied here — single-call model, same contract
        // as the SPI SR→DR sequence but consumed at once).
        self.sr &= !((1 << 0) | (1 << 1) | (1 << 2));
        self.sr |= 0x00C0; // TXE, TC
        self.update_interrupt(sys);
        dr
    }

    /// CTS-gated transmit: with CTSE (CR3 bit 9) set and CTS deasserted
    /// the byte is held (not sunk to the console, TXE/TC stay clear) —
    /// silicon blocks the shifter while CTS is high. Returns true when
    /// the byte was accepted.
    fn write_dr(&mut self, value: u32, sys: &System) -> bool {
        if self.cr3 & (1 << 9) != 0 && !self.cts_asserted {
            // Held: TXE/TC clear so firmware polls correctly.
            self.sr &= !0x00C0;
            return false;
        }
        let ch = (value & 0xFF) as u8;
        self.tx_data.push(ch);
        get_uart_output().lock().unwrap().push(ch as char);
        self.sr |= 0x00C0; // TXE=1, TC=1
        self.update_interrupt(sys);
        true
    }

    /// Queued TX length (harness scope probe: how many bytes the guest
    /// emitted — lets a test assert CTSE held bytes back, then released).
    pub fn tx_len(&self) -> usize {
        self.tx_data.len()
    }
}

impl Peripheral for Usart {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            // SR: live CTS flag (bit 9) reflects the harness CTS level;
            // PE/FE/NE/ORE are latched fault bits (cleared on DR read).
            0x00 => {
                let mut sr = self.sr;
                if self.cts_asserted {
                    sr |= 1 << 9; // CTSF: CTS asserted
                } else {
                    sr &= !(1 << 9);
                }
                self.sr |= 0x00C0; // TXE and TC stay set after reading SR
                sr
            }
            0x04 => self.read_dr(sys),
            0x08 => self.brr,
            0x0C => self.cr1,
            0x10 => self.cr2,
            0x14 => self.cr3,
            0x18 => self.gtp,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // SR is w1c for CTSF (bit 9): writing the bit clears the
                // latched edge flag. Other bits ignore writes.
                if value & (1 << 9) != 0 {
                    self.sr &= !(1 << 9);
                }
            }
            0x04 => {
                self.write_dr(value, sys);
            }
            0x08 => self.brr = value,
            0x0C => {
                self.cr1 = value & 0xFFFF;
                self.update_interrupt(sys);
            }
            // CR2: LINEN (bit 14), STOP (13:12), CLKEN/CPOL/CPHA/LBCL,
            // LBDIE/LBDL/ADD — stored verbatim (LIN mode itself is a
            // protocol the model does not speak; see lin_active()).
            0x10 => self.cr2 = value & 0xFFFF,
            // CR3: ONEBIT/CTSIE/CTSE/RTSE/DMAT/DMAR/SCEN/NACK/HDSEL/
            // IRLP/IREN/EIE — stored verbatim (Smartcard/IrDA modes are
            // protocols the model does not speak; see sc_active()).
            // CTSE (bit 9) gates TX on the harness CTS level; RTSE
            // (bit 8) is accepted (RTS output has no pin to drive —
            // firmware observes the stored bit).
            0x14 => {
                let old = self.cr3;
                self.cr3 = value & 0xFFFF;
                // Releasing CTSE (or CTS asserting) re-arms TXE/TC so a
                // subsequently unblocked write completes promptly.
                if old & (1 << 9) != 0 && value & (1 << 9) == 0 {
                    self.sr |= 0x00C0;
                }
                self.update_interrupt(sys);
            }
            0x18 => self.gtp = value,
            _ => {}
        }
    }

    fn rx_byte(&mut self, sys: &System, byte: u8) {
        if self.rx_buf.len() < 64 {
            self.rx_buf.push(byte);
            self.sr |= 1 << 5; // RXNE
            // Injected wire faults land with the byte (silicon latches
            // FE/PE alongside RXNE; PE needs PCE enabled).
            if self.rx_fault & 1 != 0 {
                self.sr |= 1 << 1; // FE
            }
            if self.rx_fault & 2 != 0 && self.cr1 & (1 << 10) != 0 {
                self.sr |= 1 << 0; // PE (PCE-gated)
            }
            self.rx_fault = 0;
        } else {
            self.sr |= 1 << 3; // ORE
        }
        self.sr |= 0x00C0; // TXE, TC
        self.update_interrupt(sys);
    }
}
