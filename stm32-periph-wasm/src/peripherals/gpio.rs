use regex::Regex;
use crate::system::System;
use super::Peripheral;

const NUM_PORTS: usize = 11;

#[derive(Clone, Copy)]
pub struct Pin {
    port: u8,
    pin: u8,
}

impl Pin {
    pub fn from_str(name: &str) -> Self {
        let name = name.to_uppercase();
        let re = Regex::new(r"^P?([A-Z])(\d+)$").unwrap();
        let captures = re.captures(&name).expect("Pin name invalid");
        let port = captures.get(1).unwrap().as_str().chars().next().unwrap();
        let port = GpioPorts::port_index(port);
        let pin: u8 = captures.get(2).unwrap().as_str().parse().unwrap();
        assert!(pin < 16);
        Self { port, pin }
    }

    pub fn new(port: u8, pin: u8) -> Self {
        Self { port, pin }
    }
}

pub struct GpioPorts {
    pub read_callbacks: [Vec<(u8, Box<dyn FnMut(&System) -> bool>)>; NUM_PORTS],
    pub write_callbacks: [Vec<(u8, Box<dyn FnMut(&System, bool)>)>; NUM_PORTS],
    // Input state set from external (JS bridge)
    input_state: [u16; NUM_PORTS],
    // Output state from ODR writes
    output_state: [u16; NUM_PORTS],
}

impl Default for GpioPorts {
    fn default() -> Self {
        let read_callbacks: [Vec<(u8, Box<dyn FnMut(&System) -> bool>)>; NUM_PORTS] =
            Default::default();
        let write_callbacks: [Vec<(u8, Box<dyn FnMut(&System, bool)>)>; NUM_PORTS] =
            Default::default();
        let mut g = GpioPorts {
            read_callbacks,
            write_callbacks,
            input_state: [0; NUM_PORTS],
            output_state: [0; NUM_PORTS],
        };
        g.register_eth_mirrors();
        g
    }
}

impl GpioPorts {
    pub fn port_index(letter: char) -> u8 {
        match letter {
            'A'..='K' => letter as u8 - b'A',
            _ => panic!("Invalid GPIO port {}", letter),
        }
    }

    pub fn add_read_callback(&mut self, pin: Pin, cb: impl FnMut(&System) -> bool + 'static) {
        self.read_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    /// Ethernet MII/RMII pin mirrors (RMII AF11 wiring + MII COL): live
    /// activity levels ORed into IDR reads. TX_EN (PB11) is HIGH across
    /// the paced TX wire time; CRS_DV (PA7) and the RXD activity mirror
    /// (PC4/PC5) across the RX wire time; COL (PA3) from an applied
    /// collision until that TX's wire end; MDIO (PA2)/MDC (PC1) idle
    /// HIGH (pull-ups; MDIO ops complete within a tick). RMII mirrors
    /// apply when SYSCFG PMC selects RMII, COL when it selects MII;
    /// firmware muxes AF itself (the model ORs like any input — leave
    /// ODR alone on these pins). Nibble data and clocks are electrical-
    /// only: no firmware can sample 25/50 MHz data, so activity levels
    /// are the complete observable contract.
    ///
    /// PPS pin mirror (PTP PPS output, RMII AF11-adjacent PB5 on the
    /// Discovery rig): `eth_pps_pin(sys)` reads the live PPS square-wave
    /// level (50% duty at the PTPPPSCR rate, TSE-gated) ORed into IDR —
    /// the firmware-visible "PPS pin itself" (the edge counter stays the
    /// scope probe; this mirror is what guest code polls).
    pub fn register_eth_mirrors(&mut self) {
        // SYSCFG PMC bit 23: 1 = RMII, 0 = MII.
        fn rmii(sys: &System) -> bool {
            sys.p.syscfg.borrow().pmc & 0x80_0000 != 0
        }
        self.add_read_callback(Pin::new(1, 11), move |sys: &System| {
            rmii(sys) && crate::peripherals::eth::eth_mii_signals(sys).0
        });
        self.add_read_callback(Pin::new(0, 7), move |sys: &System| {
            rmii(sys) && crate::peripherals::eth::eth_mii_signals(sys).1
        });
        self.add_read_callback(Pin::new(2, 4), move |sys: &System| {
            rmii(sys) && crate::peripherals::eth::eth_mii_signals(sys).1
        });
        self.add_read_callback(Pin::new(2, 5), move |sys: &System| {
            rmii(sys) && crate::peripherals::eth::eth_mii_signals(sys).1
        });
        self.add_read_callback(Pin::new(0, 2), |_| true); // MDIO idle
        self.add_read_callback(Pin::new(2, 1), |_| true); // MDC idle
        self.add_read_callback(Pin::new(0, 3), move |sys: &System| {
            !rmii(sys) && crate::peripherals::eth::eth_mii_signals(sys).2
        });
        // PPS pin mirror (PB5): live PPS square-wave level, TSE-gated.
        // Reads the PTP block WITHOUT the with_ptp slot scan (that scan
        // borrows every peripheral slot — calling it from inside this
        // read callback, which itself runs under the GPIO slot borrow
        // held by the IDR read path, panics "already borrowed"). The
        // math below duplicates eth_pps_level on the directly-held PTP
        // block — INCLUDING the ptp_advance(now) freshness step (a pure
        // residue read goes stale whenever the dashboard is the only
        // reader: the PTP-block tick may last have run many idle ticks
        // ago, and eth_pps_level's own advance can't help from here).
        self.add_read_callback(Pin::new(1, 5), move |sys: &System| {
            for slot in &sys.p.peripherals {
                // PTP block: MAC base +0x700 (0x40028700 on F407).
                if slot.start == 0x4002_8700 {
                    let mut b = match slot.peripheral.try_borrow_mut() {
                        Ok(b) => b,
                        Err(_) => return false,
                    };
                    if let Some(mac) = b.as_any_mut().downcast_mut::<crate::peripherals::eth::EthernetMac>() {
                        mac.ptp_advance(sys, crate::system::instruction_count());
                        return mac.pps_pin_level();
                    }
                    return false;
                }
            }
            false
        });
    }

    pub fn add_write_callback(&mut self, pin: Pin, cb: impl FnMut(&System, bool) + 'static) {
        self.write_callbacks[pin.port as usize].push((pin.pin, Box::new(cb)));
    }

    pub fn set_input_pin(&mut self, port: u8, pin: u8, value: bool) {
        if value {
            self.input_state[port as usize] |= 1 << pin;
        } else {
            self.input_state[port as usize] &= !(1 << pin);
        }
    }

    pub fn read_input_pin(&self, port: u8, pin: u8) -> bool {
        (self.input_state[port as usize] >> pin) & 1 != 0
    }

    pub fn read_output_pin(&self, port: u8, pin: u8) -> bool {
        (self.output_state[port as usize] >> pin) & 1 != 0
    }

    pub fn read_port(&mut self, sys: &System, port: u8) -> u16 {
        // A pin driven as output (ODR/BSRR) reads back as that level on STM32.
        let mut v = self.input_state[port as usize] | self.output_state[port as usize];
        for (pin, cb) in &mut self.read_callbacks[port as usize] {
            if cb(sys) {
                v |= 1 << *pin;
            }
        }
        v
    }

    pub fn write_port(&mut self, sys: &System, port: u8, pin: u8, value: bool) {
        for (pin_cb, cb) in &mut self.write_callbacks[port as usize] {
            if *pin_cb == pin {
                cb(sys, value);
            }
        }
    }

    pub fn set_output_pin(&mut self, port: u8, pin: u8, value: bool) {
        if value {
            self.output_state[port as usize] |= 1 << pin;
        } else {
            self.output_state[port as usize] &= !(1 << pin);
        }
    }
}

#[derive(Default)]
pub struct Gpio {
    port_letter: char,
    port: u8,
    mode: u32,
    otype: u32,
    ospeed: u32,
    pupd: u32,
    od: u32,
    id: u32,
    lck: u32,
    /// LCKR lock-key sequence state (see the 0x1C write arm): 0 = idle,
    /// 1/2 = steps completed; `lck_pending` holds the step-1 LCKk bits.
    lck_seq: u8,
    lck_pending: u32,
    afrl: u32,
    afrh: u32,
    bsrr: u32,
}

impl Gpio {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if let Some(block) = name.strip_prefix("GPIO") {
            let port_letter = block.chars().next().unwrap();
            let port = GpioPorts::port_index(port_letter);
            Some(Box::new(Self { port_letter, port, ..Self::default() }))
        } else {
            None
        }
    }

    fn iter_port_reg_changes(old_value: u32, new_value: u32, stride: u8, mut f: impl FnMut(u8, u8)) {
        let mut changes = old_value ^ new_value;
        let stride_mask = 0xFF >> (8 - stride);
        while changes != 0 {
            let right_most_bit = changes.trailing_zeros() as u8;
            let pin = right_most_bit / stride;
            if pin <= 16 {
                let v = (new_value >> (pin * stride)) as u8 & stride_mask;
                f(pin, v);
            }
            changes &= !(stride_mask as u32) << (pin * stride);
        }
    }
}

impl Peripheral for Gpio {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => {
                // MODER
                self.mode
            }
            0x04 => self.otype,
            0x08 => self.ospeed,
            0x0C => self.pupd,
            0x10 => {
                // IDR - read pin states
                let port_idr = sys.p.gpio.borrow_mut().read_port(sys, self.port);
                port_idr as u32
            }
            0x14 => {
                // ODR
                self.od
            }
            0x18 => {
                // BSRR (write-only)
                0
            }
            0x1C => self.lck,
            0x20 => self.afrl,
            0x24 => self.afrh,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                // MODER: locked pins (LCKR LCKk + LCKK engaged) ignore
                // writes (silicon freezes the whole port config until
                // reset). Only the unlocked pins take the new mode.
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    let locked = self.lck & 0xFFFF;
                    for pin in 0..16u8 {
                        if locked & (1 << pin) != 0 {
                            let mask = 0b11 << (pin * 2);
                            value = (value & !mask) | (self.mode & mask);
                        }
                    }
                }
                let old = self.mode;
                self.mode = value;
                Self::iter_port_reg_changes(old, value, 2, |pin, new_mode| {
                    let is_output = new_mode == 0b01 || new_mode == 0b10;
                    if !is_output {
                        sys.p.gpio.borrow_mut().set_output_pin(self.port, pin, false);
                    }
                });
            }
            // OTYPER/OSPEEDR/PUPDR/AFRL/AFRH freeze under an engaged
            // LCKK for the selected pins (same rule as MODER above).
            0x04 => {
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    let locked = self.lck & 0xFFFF;
                    value = (value & !locked) | (self.otype & locked);
                }
                self.otype = value;
            }
            0x08 => {
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    for pin in 0..16u8 {
                        if self.lck & (1 << pin) != 0 {
                            let mask = 0b11 << (pin * 2);
                            value = (value & !mask) | (self.ospeed & mask);
                        }
                    }
                }
                self.ospeed = value;
            }
            0x0C => {
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    for pin in 0..16u8 {
                        if self.lck & (1 << pin) != 0 {
                            let mask = 0b11 << (pin * 2);
                            value = (value & !mask) | (self.pupd & mask);
                        }
                    }
                }
                self.pupd = value;
            }
            0x10 => { /* IDR is read-only */ }
            0x14 => {
                let old = self.od;
                self.od = value;
                Self::iter_port_reg_changes(old, value, 1, |pin, val| {
                    let is_set = val != 0;
                    sys.p.gpio.borrow_mut().set_output_pin(self.port, pin, is_set);
                    sys.p.gpio.borrow_mut().write_port(sys, self.port, pin, is_set);
                });
            }
            0x18 => {
                // BSRR
                let set = value & 0xFFFF;
                let reset = (value >> 16) & 0xFFFF;
                for pin in 0..16u8 {
                    if set & (1 << pin) != 0 {
                        self.od |= 1 << pin;
                        sys.p.gpio.borrow_mut().set_output_pin(self.port, pin, true);
                        sys.p.gpio.borrow_mut().write_port(sys, self.port, pin, true);
                    }
                    if reset & (1 << pin) != 0 {
                        self.od &= !(1 << pin);
                        sys.p.gpio.borrow_mut().set_output_pin(self.port, pin, false);
                        sys.p.gpio.borrow_mut().write_port(sys, self.port, pin, false);
                    }
                }
            }
            0x1C => {
                // LCKR: the lock key sequence is W(1<<16|LCKk) → W(LCKk) →
                // W(1<<16|LCKk) → R(LCKK set confirms). The model tracks
                // it in three steps: any other write aborts the sequence.
                // Once LCKK (bit 16) reads set, the selected LCKk pins are
                // frozen (see MODER/OTYPER/OSPEEDR/PUPDR/AFRL/AFRH arms).
                const KEY: u32 = 1 << 16;
                if value & KEY != 0 && self.lck & KEY == 0 && self.lck_seq == 0 {
                    // Step 1: write LCKK + LCKk bits.
                    self.lck_seq = 1;
                    self.lck_pending = value & 0xFFFF;
                } else if value & KEY == 0 && self.lck_seq == 1 {
                    // Step 2: write LCKk alone — must match step 1.
                    if value & 0xFFFF == self.lck_pending {
                        self.lck_seq = 2;
                    } else {
                        self.lck_seq = 0;
                    }
                } else if value & KEY != 0 && self.lck_seq == 2 {
                    // Step 3: write LCKK + LCKk again — must match: engage.
                    if value & 0xFFFF == self.lck_pending {
                        self.lck = KEY | self.lck_pending;
                    }
                    self.lck_seq = 0;
                } else {
                    // Any other write aborts (but a plain LCKK read still
                    // reports the engaged state).
                    self.lck_seq = 0;
                    if self.lck & KEY == 0 {
                        self.lck = value & 0x1FFFF;
                    }
                }
            }
            0x20 => {
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    for pin in 0..8u8 {
                        if self.lck & (1 << pin) != 0 {
                            let mask = 0xF << (pin * 4);
                            value = (value & !mask) | (self.afrl & mask);
                        }
                    }
                }
                self.afrl = value;
            }
            0x24 => {
                let mut value = value;
                if self.lck & (1 << 16) != 0 {
                    for pin in 8..16u8 {
                        if self.lck & (1 << pin) != 0 {
                            let mask = 0xF << ((pin - 8) * 4);
                            value = (value & !mask) | (self.afrh & mask);
                        }
                    }
                }
                self.afrh = value;
            }
            _ => {}
        }
    }
}
