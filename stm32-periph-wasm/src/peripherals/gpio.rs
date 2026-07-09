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
        GpioPorts {
            read_callbacks,
            write_callbacks,
            input_state: [0; NUM_PORTS],
            output_state: [0; NUM_PORTS],
        }
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
        let mut v = self.input_state[port as usize];
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
                let old = self.mode;
                self.mode = value;
                Self::iter_port_reg_changes(old, value, 2, |pin, new_mode| {
                    let is_output = new_mode == 0b01 || new_mode == 0b10;
                    if !is_output {
                        sys.p.gpio.borrow_mut().set_output_pin(self.port, pin, false);
                    }
                });
            }
            0x04 => self.otype = value,
            0x08 => self.ospeed = value,
            0x0C => self.pupd = value,
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
            0x1C => self.lck = value,
            0x20 => self.afrl = value,
            0x24 => self.afrh = value,
            _ => {}
        }
    }
}
