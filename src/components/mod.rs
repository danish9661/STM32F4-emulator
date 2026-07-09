pub mod builtins;
pub mod netlist;

use serde::{Deserialize, Serialize};
use std::any::Any;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PinValue {
    Low,
    High,
    Floating,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum DriveStrength {
    None,
    Passive,
    PushPull,
    OpenDrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum PinMode {
    Input,
    Output,
    Bidirectional,
}

pub trait Component {
    fn name(&self) -> &str;
    fn kind(&self) -> &str;
    fn pin_count(&self) -> usize;
    fn pin_name(&self, index: usize) -> Option<&str>;
    fn pin_mode(&self, index: usize) -> PinMode;
    fn read_pin(&self, index: usize) -> PinValue;
    fn write_pin(&mut self, index: usize, value: PinValue, strength: DriveStrength);
    fn read_pin_analog(&self, _index: usize) -> f64 { 0.0 }
    fn write_pin_analog(&mut self, _index: usize, _value: f64) {}
    fn tick(&mut self, _sys: &crate::system::System) {}
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}
