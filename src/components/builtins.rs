use serde::Deserialize;
use std::any::Any;

use super::{Component, PinValue, PinMode, DriveStrength};

#[derive(Debug)]
pub struct Led {
    name: String,
    state: bool,
}

impl Led {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), state: false }
    }
}

impl Component for Led {
    fn name(&self) -> &str { &self.name }
    fn kind(&self) -> &str { "led" }
    fn pin_count(&self) -> usize { 2 }
    fn pin_name(&self, index: usize) -> Option<&str> {
        match index {
            0 => Some("anode"),
            1 => Some("cathode"),
            _ => None,
        }
    }
    fn pin_mode(&self, _index: usize) -> PinMode { PinMode::Input }
    fn read_pin(&self, _index: usize) -> PinValue {
        if self.state { PinValue::High } else { PinValue::Low }
    }
    fn write_pin(&mut self, _index: usize, value: PinValue, _strength: DriveStrength) {
        self.state = value == PinValue::High;
    }
    fn tick(&mut self, _sys: &crate::system::System) {
        if self.state {
            trace!("{} on", self.name);
        }
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

#[derive(Debug)]
pub struct Button {
    name: String,
    pulled_up: bool,
    pressed: bool,
}

impl Button {
    pub fn new(name: &str, pulled_up: bool) -> Self {
        Self { name: name.to_string(), pulled_up, pressed: false }
    }
    pub fn press(&mut self) { self.pressed = true; }
    pub fn release(&mut self) { self.pressed = false; }
}

impl Component for Button {
    fn name(&self) -> &str { &self.name }
    fn kind(&self) -> &str { "button" }
    fn pin_count(&self) -> usize { 1 }
    fn pin_name(&self, index: usize) -> Option<&str> {
        match index { 0 => Some("out"), _ => None }
    }
    fn pin_mode(&self, _index: usize) -> PinMode { PinMode::Output }
    fn read_pin(&self, _index: usize) -> PinValue {
        if self.pressed {
            if self.pulled_up { PinValue::Low } else { PinValue::High }
        } else {
            if self.pulled_up { PinValue::High } else { PinValue::Low }
        }
    }
    fn write_pin(&mut self, _index: usize, _value: PinValue, _strength: DriveStrength) {}
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

#[derive(Debug)]
pub struct Potentiometer {
    name: String,
    position: f64,
    pin_a: bool,
    pin_b: bool,
}

impl Potentiometer {
    pub fn new(name: &str, position: f64) -> Self {
        Self { name: name.to_string(), position, pin_a: false, pin_b: false }
    }
    pub fn set_position(&mut self, pos: f64) { self.position = pos.clamp(0.0, 1.0); }
}

impl Component for Potentiometer {
    fn name(&self) -> &str { &self.name }
    fn kind(&self) -> &str { "potentiometer" }
    fn pin_count(&self) -> usize { 3 }
    fn pin_name(&self, index: usize) -> Option<&str> {
        match index {
            0 => Some("vcc"),
            1 => Some("gnd"),
            2 => Some("wiper"),
            _ => None,
        }
    }
    fn pin_mode(&self, _index: usize) -> PinMode { PinMode::Bidirectional }
    fn read_pin(&self, index: usize) -> PinValue {
        match index {
            2 => {
                if self.position > 0.5 { PinValue::High } else { PinValue::Low }
            }
            _ => PinValue::Floating,
        }
    }
    fn read_pin_analog(&self, index: usize) -> f64 {
        if index == 2 { self.position } else { 0.0 }
    }
    fn write_pin(&mut self, index: usize, value: PinValue, _strength: DriveStrength) {
        match index {
            0 => self.pin_a = value == PinValue::High,
            1 => self.pin_b = value == PinValue::Low,
            _ => {}
        }
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

#[derive(Debug, Clone)]
pub struct TestProbe {
    name: String,
    pub writes: Vec<(PinValue, DriveStrength)>,
    pub ticks: u64,
}

impl TestProbe {
    pub fn new(name: &str) -> Self {
        Self { name: name.to_string(), writes: Vec::new(), ticks: 0 }
    }
}

impl Component for TestProbe {
    fn name(&self) -> &str { &self.name }
    fn kind(&self) -> &str { "test_probe" }
    fn pin_count(&self) -> usize { 1 }
    fn pin_name(&self, index: usize) -> Option<&str> {
        match index { 0 => Some("in"), _ => None }
    }
    fn pin_mode(&self, _index: usize) -> PinMode { PinMode::Input }
    fn read_pin(&self, _index: usize) -> PinValue { PinValue::Floating }
    fn write_pin(&mut self, _index: usize, value: PinValue, strength: DriveStrength) {
        self.writes.push((value, strength));
    }
    fn tick(&mut self, _sys: &crate::system::System) {
        self.ticks += 1;
    }
    fn as_any(&self) -> &dyn Any { self }
    fn as_any_mut(&mut self) -> &mut dyn Any { self }
}

#[derive(Debug, Deserialize, Clone)]
#[serde(tag = "type")]
pub enum ComponentConfig {
    #[serde(rename = "led")]
    Led { name: String },
    #[serde(rename = "button")]
    Button { name: String, pulled_up: Option<bool> },
    #[serde(rename = "potentiometer")]
    Potentiometer { name: String, position: Option<f64> },
}

impl ComponentConfig {
    pub fn into_component(self) -> Box<dyn Component> {
        match self {
            ComponentConfig::Led { name } => Box::new(Led::new(&name)),
            ComponentConfig::Button { name, pulled_up } => Box::new(Button::new(&name, pulled_up.unwrap_or(true))),
            ComponentConfig::Potentiometer { name, position } => Box::new(Potentiometer::new(&name, position.unwrap_or(0.5))),
        }
    }
}
