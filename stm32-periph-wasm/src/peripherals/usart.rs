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
    /// Queued TX break (SBK semantics): set by `sbk_request`, consumed by
    /// the next DR write (silicon transmits the break ahead of the byte).
    break_queued: bool,
    /// Smartcard NACK state: retries used on the current byte + armed flag
    /// (harness = the card rejecting a byte; see `sc_nack_next`).
    sc_retry: u8,
    sc_nack_armed: bool,
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
                break_queued: false,
                sc_retry: 0,
                sc_nack_armed: false,
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

    // ── LIN master/slave break handling (CR2 LINEN) ─────────────────────
    // Silicon LIN: a 13-bit dominant break + 0x55 sync + ID. The model
    // tracks the observable register contract:
    // - TX break: SBK (CR1 bit 0) queues one break ahead of the next byte
    //   (silicon sends 10/11 zeros + stop). The break "transmits" on the
    //   next DR write (or SBK-clear) — TXE/TC follow the DR write as usual.
    // - RX break: the harness delivers a break via `lin_break()` (the LIN
    //   master on the wire). It latches LBD (SR bit 8) + RXNE with a 0x00
    //   data byte, and fires the IRQ when LBDIE (CR2 bit 6) is set.
    // No baud re-measurement (LBDL/LBCL) — length detection needs edge
    // timing the instruction clock cannot provide honestly.
    /// Harness = the LIN master: deliver a break frame (latches LBD +
    /// RXNE with 0x00 data; IRQ when LBDIE). Only when LINEN is set —
    /// a break on a non-LIN port is line noise (dropped, like silicon
    /// ignoring sub-break glitches outside LIN mode).
    pub fn lin_break(&mut self, sys: &System) {
        if self.cr2 & (1 << 14) == 0 {
            return;
        }
        if self.muted() {
            return; // muted receiver drops the break silently
        }
        if self.rx_buf.len() < 64 {
            self.rx_buf.push(0x00);
            self.sr |= (1 << 5) | (1 << 8); // RXNE + LBD
        } else {
            self.sr |= 1 << 3; // ORE
        }
        self.sr |= 0x00C0;
        if self.cr2 & (1 << 6) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(self.irq_num);
        }
    }

    // ── Smartcard T=0 handling (CR3 SCEN) ───────────────────────────────
    // Silicon T=0: the card pulls I/O low during the guard time to signal
    // a bad parity byte (NACK); the F4 transmitter retries until ACK
    // (no SCARCNT field on F4 — the model caps at 8 so a dead card
    // terminates, documented substitute), then flags
    // retry-exhausted. The model implements the retry loop against the
    // harness NACK injector (`sc_nack_next`):
    // - NACK set (CR3 bit 4) + a NACK armed: the just-written byte is NOT
    //   sunk (card rejected it), TC stays clear, retry counter advances.
    // - After N+1 failed tries: byte dropped, NE latched (SR bit 2 — F4
    //   has no TEACK/FEACK bits; NE is the retry-exhausted observable).
    // - Without NACK armed: byte sinks normally, TC sets (card ACKed).
    // GTPR guard time (GT[15:8]) + prescaler (PSC[7:0]) are stored; the
    // guard delay itself is not timed (instruction clock has no baud
    // domain) — firmware polls GTPR readback, never the delay.
    /// Harness = the smartcard: NACK the next transmitted byte (parity
    /// error from the card's view). One-shot per call.
    pub fn sc_nack_next(&mut self) {
        self.sc_nack_armed = true;
    }

    /// Smartcard retry counter (scope probe: retries used on the current
    /// byte — lets a test assert the NACK loop retried, then exhausted).
    pub fn sc_retries(&self) -> u8 {
        self.sc_retry
    }

    /// Harness = the IR transmitter: inject a byte with a pulse-width class
    /// (0 = normal 3/16 pulse, 1 = low-power 1.6µs pulse). A class mismatch
    /// against the receiver's IRLP (CR3 bit 2) latches NE (noise error, SR
    /// bit 2) alongside RXNE — the honest observable of a pulse mismatch.
    /// Matching pulses land cleanly with no flags.
    pub fn irda_rx(&mut self, sys: &System, byte: u8, low_power: bool) {
        if self.muted() {
            return; // muted receiver drops the byte silently
        }
        let rx_low = low_power;
        let want_low = self.cr3 & (1 << 2) != 0;
        if self.rx_buf.len() < 64 {
            self.rx_buf.push(byte);
            self.sr |= 1 << 5; // RXNE
            if rx_low != want_low {
                self.sr |= 1 << 2; // NE: pulse-class mismatch
            }
        } else {
            self.sr |= 1 << 3; // ORE
        }
        self.sr |= 0x00C0;
        self.update_interrupt(sys);
    }

    // ── IrDA pulse envelope (CR3 IREN) ──────────────────────────────────
    // Silicon IrDA: TX bits are 3/16-bit-time pulses (normal) or 1.6µs
    // pulses (low-power, IRLP=1); RX expects the same. The model has no
    // baud-rate time base, so it implements the observable envelope:
    // - TX: each byte sinks to the console prefixed with its pulse-width
    //   class in the tx trace (normal vs low-power) — `irda_tx_class()`
    //   reports it; the byte itself sinks unchanged (the console is not
    //   an IR demodulator).
    // - RX: `irda_rx` injects a byte with a pulse-width class; out-of-
    //   class pulses (normal byte into a low-power receiver and vice
    //   versa) latch NE (noise error, SR bit 2) alongside RXNE — the
    //   honest observable of a pulse mismatch.
    /// Pulse class of the last TX byte under IREN (0 = normal 3/16 pulse,
    /// 1 = low-power 1.6µs pulse from IRLP). Meaningful only when the
    /// byte sank with IREN set; stale otherwise (not latched per byte —
    /// the console trace is the record).
    pub fn irda_tx_class(&self) -> u8 {
        if self.cr3 & (1 << 2) != 0 { 1 } else { 0 }
    }

    fn update_interrupt(&mut self, sys: &System) {
        let mut pending = false;
        if self.cr1 & (1 << 6) != 0 && self.sr & (1 << 6) != 0 { pending = true; } // TCIE + TC
        if self.cr1 & (1 << 7) != 0 && self.sr & (1 << 7) != 0 { pending = true; } // TXEIE + TXE
        if self.cr1 & (1 << 5) != 0 && self.sr & (1 << 5) != 0 { pending = true; } // RXNEIE + RXNE
        if self.cr1 & (1 << 4) != 0 && self.sr & (1 << 4) != 0 { pending = true; } // IDLEIE + IDLE
        if self.cr1 & (1 << 8) != 0 && self.sr & (1 << 0) != 0 { pending = true; } // PEIE + PE
        // EIE (CR3 bit 0): framing/overrun/noise faults pend the IRQ.
        if self.cr3 & 1 != 0
            && self.sr & ((1 << 1) | (1 << 2) | (1 << 3)) != 0
        {
            pending = true; // FE/NE/ORE + EIE (PE is on PEIE, not EIE)
        }
        // CTSIE (CR3 bit 10): CTS edge pends the IRQ.
        if self.cr3 & (1 << 10) != 0 && self.sr & (1 << 10) != 0 {
            pending = true; // CTSIF + CTSIE
        }
        // LBDIE (CR2 bit 6): LIN break detect pends the IRQ.
        if self.cr2 & (1 << 6) != 0 && self.sr & (1 << 8) != 0 {
            pending = true; // LBD + LBDIE
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

    // ── Mute mode / receiver wakeup (CR1 RWU bit 1, WAKE bit 11) ───────
    // Silicon mute: with RWU set the receiver ignores incoming bytes
    // (no RXNE, no faults — only the address/mute-exit logic watches).
    // Exit needs a WAKE-selected event: WAKE=0 (idle-line: the harness
    // `idle_event` clears RWU — the line went idle a full frame); WAKE=1
    // (address-mark: a received byte with MSB=1 clears RWU and IS
    // delivered — silicon compares ADD[3:0] when ADDIE... the F4 has no
    // address-compare engine, so any mark byte wakes; the byte itself is
    // the observable). While muted, rx_byte() and lin_break()/irda_rx()
    // deliveries are dropped silently (no flags, no IRQ — the receiver
    // is deaf, not erroring).
    /// Whether the receiver is muted (RWU set — scope probe).
    pub fn muted(&self) -> bool {
        self.cr1 & (1 << 1) != 0
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
        // A DR read clears latched PE/FE/NE/IDLE (silicon: read SR then DR;
        // the SR read is implied here — single-call model, same contract
        // as the SPI SR→DR sequence but consumed at once).
        self.sr &= !((1 << 0) | (1 << 1) | (1 << 2) | (1 << 4));
        self.sr |= 0x00C0; // TXE, TC
        self.update_interrupt(sys);
        dr
    }

    /// CTS-gated transmit: with CTSE (CR3 bit 9) set and CTS deasserted
    /// the byte is held (not sunk to the console, TXE/TC stay clear) —
    /// silicon blocks the shifter while CTS is high. Returns true when
    /// the byte was accepted. Smartcard NACK (SCEN + NACK armed, see
    /// `sc_nack_next`) rejects the byte first: TC stays clear and the
    /// retry counter advances; after SCARCNT+1 tries the byte drops with
    /// NE latched (retry-exhausted flag — F4 has no TEACK bit).
    fn write_dr(&mut self, value: u32, sys: &System) -> bool {
        if self.cr3 & (1 << 9) != 0 && !self.cts_asserted {
            // Held: TXE/TC clear so firmware polls correctly.
            self.sr &= !0x00C0;
            return false;
        }
        // Smartcard T=0 NACK loop (SCEN bit 5 + NACK bit 4 + armed).
        // NOTE: F4 USART_CR3[7:5] is DMAT/DMAR/SCEN — there is NO SCARCNT
        // field on F4 (it appears on F7/L4). F4 silicon retries a NACKed
        // byte until the card ACKs (firmware aborts by clearing UE); the
        // model caps consecutive retries at 8 so a dead card terminates
        // instead of hanging the harness (documented substitute).
        if self.cr3 & (1 << 5) != 0 && self.cr3 & (1 << 4) != 0 && self.sc_nack_armed {
            self.sc_nack_armed = false; // one NACK per arm
            let max = 8u8; // fixed cap (no SCARCNT on F4)
            self.sc_retry += 1;
            if self.sc_retry > max {
                // Retries exhausted: byte dropped, NE latched (the F4
                // retry-exhausted observable), counter resets.
                self.sc_retry = 0;
                self.sr |= 1 << 2; // NE
                self.sr |= 0x00C0;
                self.update_interrupt(sys);
                return true; // consumed (dropped), shifter free
            }
            // Retry: byte held for retransmission, TC stays clear (the
            // card hasn't ACKed yet), TXE set (DR free for the retry).
            self.sr = (self.sr & !(1 << 6)) | (1 << 7);
            return true;
        }
        self.sc_retry = 0; // clean ACK resets the retry counter
        // SBK break (queued by `sbk_request`): consumed ahead of the byte.
        // The byte sinks normally; the break is the observable (firmware
        // polls `break_pending()` or just relies on the wire order).
        self.break_queued = false;
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

    /// Harness = the idle line: latch IDLE (SR bit 4, line idle one frame).
    /// Fires the IRQ when IDLEIE (CR1 bit 4) is set. Cleared on the next
    /// DR read (same SR→DR contract as PE/FE/NE).
    pub fn idle_event(&mut self, sys: &System) {
        // Idle-line doubles as the WAKE=0 mute exit: the line went idle
        // a full frame, so a muted receiver wakes (RWU clears) — the
        // IDLE flag itself still latches only when the receiver can see
        // the line (unmuted, or the waking edge itself).
        if self.muted() {
            if self.cr1 & (1 << 11) == 0 {
                self.cr1 &= !(1 << 1); // RWU clears: idle-line wakeup
            } else {
                return; // WAKE=1 wants an address mark, not idle
            }
        }
        self.sr |= 1 << 4; // IDLE
        self.update_interrupt(sys);
    }

    /// Harness = queue a TX break: SBK (CR1 bit 0) sends one break ahead of
    /// the next byte. Silicon transmits 10/11 zeros + stop then sets SBK
    /// back; the model latches a `break_queued` flag consumed by the next
    /// DR write (which sinks the byte normally — the break itself is the
    /// observable: `break_pending()` reports it for the mock).
    pub fn sbk_request(&mut self) {
        self.break_queued = true;
    }

    /// Whether a TX break is queued (SBK semantics scope probe).
    pub fn break_pending(&self) -> bool {
        self.break_queued
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
                // SBK (bit 0) set queues a TX break ahead of the next byte
                // (silicon transmits zeros + stop, then self-clears SBK).
                // The model latches `break_queued` (consumed by the next DR
                // write) and clears the CR1 bit at once (write-then-clear
                // like the silicon self-clear — firmware polls the queued
                // flag via `break_pending`, never a stuck SBK).
                if value & 1 != 0 {
                    self.break_queued = true;
                }
                self.cr1 = (value & 0xFFFF) & !1;
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
        // Mute mode (CR1 RWU): the receiver is deaf — bytes drop with no
        // flags and no IRQ. WAKE=1 (address-mark) exits on a mark byte
        // (MSB set): the byte IS delivered and RWU clears. WAKE=0
        // (idle-line) only exits via idle_event, so bytes keep dropping.
        if self.muted() {
            if self.cr1 & (1 << 11) != 0 && byte & 0x80 != 0 {
                self.cr1 &= !(1 << 1); // RWU clears: mark-byte wakeup
                // fall through: the waking byte is delivered normally
            } else {
                self.rx_fault = 0; // faults drop with the byte
                return;
            }
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn usart() -> (std::rc::Rc<System>, Box<dyn Peripheral>) {
        let sys = crate::system::test_dummy_system();
        let u = Usart::new("USART1", &crate::ext_devices::ExtDevices::default()).unwrap();
        (sys, u)
    }

    // Mute mode: RWU drops bytes silently (no RXNE, no faults); WAKE=0
    // exits on idle_event (RWU clears, IDLE latches); WAKE=1 ignores idle
    // and exits on a mark byte (MSB set), which IS delivered.
    #[test]
    fn mute_drops_until_wakeup() {
        let (sys, mut boxed) = usart();
        let u = boxed.as_any_mut().downcast_mut::<Usart>().unwrap();
        u.write(&sys, 0x0C, (1 << 13) | (1 << 2) | (1 << 1)); // UE+RE+RWU (WAKE=0)
        assert!(u.muted());
        u.rx_byte(&sys, 0x41);
        assert_eq!(u.sr & (1 << 5), 0, "muted: no RXNE");
        assert!(u.rx_buf.is_empty(), "muted: byte dropped");
        u.idle_event(&sys); // idle-line wakeup (WAKE=0)
        assert!(!u.muted(), "idle clears RWU when WAKE=0");
        assert_ne!(u.sr & (1 << 4), 0, "IDLE latches on the waking edge");
        u.rx_byte(&sys, 0x42);
        assert_ne!(u.sr & (1 << 5), 0, "unmuted: RXNE sets");
    }

    // WAKE=1: idle does NOT wake; a mark byte wakes AND delivers.
    #[test]
    fn mute_mark_wakeup_needs_msb() {
        let (sys, mut boxed) = usart();
        let u = boxed.as_any_mut().downcast_mut::<Usart>().unwrap();
        u.write(&sys, 0x0C, (1 << 13) | (1 << 2) | (1 << 1) | (1 << 11)); // UE+RE+RWU+WAKE
        u.idle_event(&sys);
        assert!(u.muted(), "WAKE=1: idle does not wake");
        assert_eq!(u.sr & (1 << 4), 0, "WAKE=1: idle latches no IDLE while muted");
        u.rx_byte(&sys, 0x41); // no MSB: drops, stays muted
        assert!(u.muted(), "non-mark byte keeps mute");
        assert!(u.rx_buf.is_empty());
        u.rx_byte(&sys, 0xC1); // mark byte: wakes + delivers
        assert!(!u.muted(), "mark byte wakes");
        assert_eq!(u.rx_buf, vec![0xC1], "waking byte delivered");
    }
}
