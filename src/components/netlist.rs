use std::collections::HashMap;
use serde::{Deserialize, Serialize};

use super::{Component, PinValue, PinMode, DriveStrength};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Endpoint {
    McuPin { port: u8, pin: u8 },
    ComponentPin { component: String, pin_index: usize },
    TimerAnalog { timer_idx: u8, channel: u8 },
    AdcChannel { adc_idx: u8, channel: u8 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Net {
    pub name: String,
    pub endpoints: Vec<Endpoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WireConfig {
    pub from: String,
    pub to: String,
}

pub struct Netlist {
    nets: Vec<Net>,
    pub components: Vec<Box<dyn Component>>,
    component_map: HashMap<String, usize>,
    net_for_mcu_pin: HashMap<(u8, u8), usize>,
    net_for_comp_pin: HashMap<(usize, usize), usize>,
    net_for_timer: HashMap<(u8, u8), usize>,
    net_for_adc: HashMap<(u8, u8), usize>,
    pub analog_values: HashMap<(u8, u8), f64>,
}

impl Netlist {
    pub fn new() -> Self {
        Self {
            nets: Vec::new(),
            components: Vec::new(),
            component_map: HashMap::new(),
            net_for_mcu_pin: HashMap::new(),
            net_for_comp_pin: HashMap::new(),
            net_for_timer: HashMap::new(),
            net_for_adc: HashMap::new(),
            analog_values: HashMap::new(),
        }
    }

    pub fn add_component(&mut self, component: Box<dyn Component>) {
        let name = component.name().to_string();
        self.component_map.insert(name.clone(), self.components.len());
        self.components.push(component);
    }

    pub fn get_component(&self, name: &str) -> Option<&dyn Component> {
        self.component_map.get(name).map(|&i| self.components[i].as_ref())
    }

    pub fn get_component_mut(&mut self, name: &str) -> Option<&mut dyn Component> {
        let i = *self.component_map.get(name)?;
        Some(self.components[i].as_mut())
    }

    fn parse_mcu_pin_id(s: &str) -> Option<(u8, u8)> {
        let s = s.to_uppercase();
        let s = s.trim_start_matches('P');
        if s.is_empty() {
            return None;
        }
        let port_char = s.chars().next()?;
        if !('A'..='K').contains(&port_char) {
            return None;
        }
        let port = port_char as u8 - b'A';
        let pin: u8 = s[1..].parse().ok()?;
        if pin > 15 {
            return None;
        }
        Some((port, pin))
    }

    pub fn build_wires(&mut self, wires: &[WireConfig]) {
        let mut net_bindings: Vec<(String, Vec<Endpoint>)> = Vec::new();

        fn parse_endpoint(nl: &Netlist, s: &str) -> Option<Endpoint> {
            if let Some((port, pin)) = Netlist::parse_mcu_pin_id(s) {
                return Some(Endpoint::McuPin { port, pin });
            }
            let upp = s.to_uppercase();
            if upp.starts_with("TIM") {
                let rest = &upp[3..];
                let parts: Vec<&str> = rest.splitn(2, '.').collect();
                if parts.len() == 2 {
                    let tim_idx: u8 = parts[0].parse().ok()?;
                    if let Some(ch) = parts[1].strip_prefix("CH") {
                        let channel: u8 = ch.parse().ok()?;
                        if channel >= 1 && channel <= 4 {
                            return Some(Endpoint::TimerAnalog { timer_idx: tim_idx, channel: channel - 1 });
                        }
                    }
                }
                return None;
            }
            if upp.starts_with("ADC") {
                let rest = &upp[3..];
                let parts: Vec<&str> = rest.splitn(2, '.').collect();
                if parts.len() == 2 {
                    let adc_idx: u8 = parts[0].parse().ok()?;
                    if let Some(ch) = parts[1].strip_prefix("IN") {
                        let channel: u8 = ch.parse().ok()?;
                        if channel <= 15 {
                            return Some(Endpoint::AdcChannel { adc_idx: adc_idx - 1, channel });
                        }
                    }
                }
                return None;
            }
            let parts: Vec<&str> = s.splitn(2, '.').collect();
            if parts.len() != 2 {
                warn!("Invalid endpoint: {}", s);
                return None;
            }
            let comp_idx = nl.component_map.get(parts[0])?;
            let pin_idx: usize = match parts[1].parse() {
                Ok(v) => v,
                Err(_) => {
                    let comp = &nl.components[*comp_idx];
                    (0..comp.pin_count()).find(|&i| comp.pin_name(i) == Some(parts[1]))?
                }
            };
            Some(Endpoint::ComponentPin { component: parts[0].to_string(), pin_index: pin_idx })
        }

        for wire in wires {
            let from_endpoint = match parse_endpoint(self, &wire.from) {
                Some(ep) => ep,
                None => continue,
            };
            let to_endpoint = match parse_endpoint(self, &wire.to) {
                Some(ep) => ep,
                None => continue,
            };

            let existing = net_bindings.iter_mut().find(|(_, eps)| {
                eps.iter().any(|e| e == &from_endpoint || e == &to_endpoint)
            });

            if let Some((_, eps)) = existing {
                if !eps.contains(&from_endpoint) {
                    eps.push(from_endpoint);
                }
                if !eps.contains(&to_endpoint) {
                    eps.push(to_endpoint);
                }
            } else {
                net_bindings.push((wire.from.clone(), vec![from_endpoint, to_endpoint]));
            }
        }

        for (name, endpoints) in net_bindings {
            let net_idx = self.nets.len();
            self.nets.push(Net { name, endpoints: endpoints.clone() });

            for ep in &endpoints {
                match ep {
                    Endpoint::McuPin { port, pin } => {
                        self.net_for_mcu_pin.insert((*port, *pin), net_idx);
                    }
                    Endpoint::ComponentPin { component, pin_index } => {
                        if let Some(&ci) = self.component_map.get(component) {
                            self.net_for_comp_pin.insert((ci, *pin_index), net_idx);
                        }
                    }
                    Endpoint::TimerAnalog { timer_idx, channel } => {
                        self.net_for_timer.insert((*timer_idx, *channel), net_idx);
                    }
                    Endpoint::AdcChannel { adc_idx, channel } => {
                        self.net_for_adc.insert((*adc_idx, *channel), net_idx);
                    }
                }
            }
        }
    }

    pub fn propagate_mcu_write(&mut self, port: u8, pin: u8, value: bool) {
        let net_idx = match self.net_for_mcu_pin.get(&(port, pin)) {
            Some(&i) => i,
            None => return,
        };

        let pin_val = if value { PinValue::High } else { PinValue::Low };
        let strength = DriveStrength::PushPull;

        let endpoints = self.nets[net_idx].endpoints.clone();
        for ep in &endpoints {
            match ep {
                Endpoint::ComponentPin { component, pin_index } => {
                    if let Some(&ci) = self.component_map.get(component) {
                        self.components[ci].write_pin(*pin_index, pin_val, strength);
                    }
                }
                Endpoint::McuPin { port: p, pin: pi } => {
                    if *p == port && *pi == pin {
                        continue;
                    }
                }
                Endpoint::TimerAnalog { .. } | Endpoint::AdcChannel { .. } => {}
            }
        }
    }

    pub fn read_mcu_pin(&self, port: u8, pin: u8) -> Option<bool> {
        let net_idx = self.net_for_mcu_pin.get(&(port, pin))?;
        self.resolve_net_value(*net_idx)
    }

    fn resolve_net_value(&self, net_idx: usize) -> Option<bool> {
        let net = &self.nets[net_idx];

        let mut best_value = None;

        for ep in &net.endpoints {
            let (val, strength) = match ep {
                Endpoint::ComponentPin { component, pin_index } => {
                    let ci = self.component_map.get(component)?;
                    let comp = &self.components[*ci];
                    let val = comp.read_pin(*pin_index);
                    let mode = comp.pin_mode(*pin_index);
                    let strength = match (val, mode) {
                        (PinValue::High, PinMode::Output) => DriveStrength::PushPull,
                        (PinValue::Low, PinMode::Output) => DriveStrength::PushPull,
                        (PinValue::High, PinMode::Bidirectional) => DriveStrength::PushPull,
                        (PinValue::Low, PinMode::Bidirectional) => DriveStrength::PushPull,
                        _ => DriveStrength::None,
                    };
                    (val, strength)
                }
                Endpoint::McuPin { port: _, pin: _ } => {
                    (PinValue::Floating, DriveStrength::None)
                }
                Endpoint::TimerAnalog { timer_idx: _, channel: _ } => {
                    (PinValue::Floating, DriveStrength::None)
                }
                Endpoint::AdcChannel { adc_idx: _, channel: _ } => {
                    (PinValue::Floating, DriveStrength::None)
                }
            };

            if strength == DriveStrength::PushPull {
                match val {
                    PinValue::High => return Some(true),
                    PinValue::Low => return Some(false),
                    _ => {}
                }
            }

            if best_value.is_none() && val != PinValue::Floating {
                best_value = match val {
                    PinValue::High => Some(true),
                    PinValue::Low => Some(false),
                    _ => best_value,
                };
            }
        }

        best_value
    }

    pub fn write_timer_analog(&mut self, timer_idx: u8, channel: u8, value: f64) {
        if let Some(&net_idx) = self.net_for_timer.get(&(timer_idx, channel)) {
            self.analog_values.insert((timer_idx, channel), value);
            let endpoints = self.nets[net_idx].endpoints.clone();
            for ep in &endpoints {
                if let Endpoint::ComponentPin { component, pin_index } = ep {
                    if let Some(&ci) = self.component_map.get(component) {
                        self.components[ci].write_pin_analog(*pin_index, value);
                    }
                }
            }
        }
    }

    pub fn read_analog(&self, adc_idx: u8, channel: u8) -> f64 {
        if let Some(&net_idx) = self.net_for_adc.get(&(adc_idx, channel)) {
            for ep in &self.nets[net_idx].endpoints {
                if let Endpoint::ComponentPin { component, pin_index } = ep {
                    if let Some(&ci) = self.component_map.get(component) {
                        let v = self.components[ci].read_pin_analog(*pin_index);
                        if v > 0.0 { return v; }
                    }
                }
                if let Endpoint::TimerAnalog { timer_idx, channel } = ep {
                    if let Some(v) = self.analog_values.get(&(*timer_idx, *channel)) {
                        return *v;
                    }
                }
            }
        }
        0.0
    }

    pub fn tick(&mut self, sys: &crate::system::System) {
        for comp in &mut self.components {
            comp.tick(sys);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::builtins::{Led, Button, TestProbe};

    #[test]
    fn test_parse_mcu_pin() {
        assert_eq!(Netlist::parse_mcu_pin_id("PA0"), Some((0, 0)));
        assert_eq!(Netlist::parse_mcu_pin_id("PB15"), Some((1, 15)));
        assert_eq!(Netlist::parse_mcu_pin_id("PC5"), Some((2, 5)));
        assert_eq!(Netlist::parse_mcu_pin_id("A0"), Some((0, 0)));
    }

    #[test]
    fn test_parse_mcu_pin_invalid() {
        assert_eq!(Netlist::parse_mcu_pin_id(""), None);
        assert_eq!(Netlist::parse_mcu_pin_id("PX0"), None);
        assert_eq!(Netlist::parse_mcu_pin_id("PA16"), None);
    }

    #[test]
    fn test_single_wire_led() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(Led::new("led1")));

        nl.build_wires(&[
            WireConfig { from: "PA0".into(), to: "led1.anode".into() },
        ]);

        assert_eq!(nl.nets.len(), 1);
        assert_eq!(nl.nets[0].endpoints.len(), 2);
        assert!(nl.net_for_mcu_pin.contains_key(&(0, 0)));
        assert!(nl.net_for_comp_pin.contains_key(&(0, 0)));
    }

    #[test]
    fn test_propagate_mcu_to_led() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(Led::new("led1")));
        nl.build_wires(&[
            WireConfig { from: "PA0".into(), to: "led1.anode".into() },
        ]);

        // Propagate high
        nl.propagate_mcu_write(0, 0, true);
        let led = nl.get_component("led1").unwrap();
        assert_eq!(led.read_pin(0), PinValue::High);

        // Propagate low
        nl.propagate_mcu_write(0, 0, false);
        let led = nl.get_component("led1").unwrap();
        assert_eq!(led.read_pin(0), PinValue::Low);
    }

    #[test]
    fn test_read_mcu_pin_from_button() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(Button::new("btn1", true))); // pulled-up
        nl.build_wires(&[
            WireConfig { from: "btn1.out".into(), to: "PA0".into() },
        ]);

        // Button not pressed -> pulled-up -> High
        assert_eq!(nl.read_mcu_pin(0, 0), Some(true));

        // Press button
        nl.get_component_mut("btn1").unwrap()
            .as_any_mut().downcast_mut::<Button>().unwrap().press();
        assert_eq!(nl.read_mcu_pin(0, 0), Some(false));
    }

    #[test]
    fn test_multi_endpoint_net() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(Led::new("led1")));
        nl.add_component(Box::new(Led::new("led2")));
        nl.add_component(Box::new(Button::new("btn1", true)));

        // btn1.out -> PA0, PA0 -> led1.anode, PA0 -> led2.anode
        nl.build_wires(&[
            WireConfig { from: "btn1.out".into(), to: "PA0".into() },
            WireConfig { from: "PA0".into(), to: "led1.anode".into() },
            WireConfig { from: "PA0".into(), to: "led2.anode".into() },
        ]);

        // All on one net
        assert_eq!(nl.nets.len(), 1);

        // Both LEDs get the button value
        assert_eq!(nl.read_mcu_pin(0, 0), Some(true));

        nl.get_component_mut("btn1").unwrap()
            .as_any_mut().downcast_mut::<Button>().unwrap().press();
        assert_eq!(nl.read_mcu_pin(0, 0), Some(false));
    }

    #[test]
    fn test_probe_records_mcu_writes() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(TestProbe::new("probe1")));
        nl.build_wires(&[WireConfig { from: "PA5".into(), to: "probe1.in".into() }]);

        nl.propagate_mcu_write(0, 5, true);
        nl.propagate_mcu_write(0, 5, false);
        nl.propagate_mcu_write(0, 5, true);

        let probe = nl.get_component("probe1").unwrap()
            .as_any().downcast_ref::<TestProbe>().unwrap();
        assert_eq!(probe.writes.len(), 3);
        assert_eq!(probe.writes[0], (PinValue::High, DriveStrength::PushPull));
        assert_eq!(probe.writes[1], (PinValue::Low, DriveStrength::PushPull));
        assert_eq!(probe.writes[2], (PinValue::High, DriveStrength::PushPull));
    }

    #[test]
    fn test_probe_not_wired_no_record() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(TestProbe::new("probe1")));
        // No wires — probe is disconnected
        nl.build_wires(&[]);

        nl.propagate_mcu_write(0, 5, true);
        let probe = nl.get_component("probe1").unwrap()
            .as_any().downcast_ref::<TestProbe>().unwrap();
        assert_eq!(probe.writes.len(), 0);
    }

    #[test]
    fn test_probe_tick_increment() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(TestProbe::new("probe1")));

        // Simulate multiple ticks
        nl.tick(&crate::system::System {
            uc: std::cell::RefCell::new(Box::leak(Box::new(
                unicorn_engine::Unicorn::new(
                    unicorn_engine::unicorn_const::Arch::ARM,
                    unicorn_engine::unicorn_const::Mode::MCLASS | unicorn_engine::unicorn_const::Mode::LITTLE_ENDIAN,
                ).unwrap()
            ))),
            p: std::rc::Rc::new(crate::peripherals::Peripherals::default()),
            d: std::rc::Rc::new(crate::ext_devices::ExtDevices {
                spi_flashes: vec![], usart_probes: vec![], displays: vec![],
                lcds: vec![], touchscreens: vec![],
            }),
            n: Some(std::rc::Rc::new(std::cell::RefCell::new(Netlist::new()))),
        });
        nl.tick(&crate::system::System {
            uc: std::cell::RefCell::new(Box::leak(Box::new(
                unicorn_engine::Unicorn::new(
                    unicorn_engine::unicorn_const::Arch::ARM,
                    unicorn_engine::unicorn_const::Mode::MCLASS | unicorn_engine::unicorn_const::Mode::LITTLE_ENDIAN,
                ).unwrap()
            ))),
            p: std::rc::Rc::new(crate::peripherals::Peripherals::default()),
            d: std::rc::Rc::new(crate::ext_devices::ExtDevices {
                spi_flashes: vec![], usart_probes: vec![], displays: vec![],
                lcds: vec![], touchscreens: vec![],
            }),
            n: Some(std::rc::Rc::new(std::cell::RefCell::new(Netlist::new()))),
        });

        let probe = nl.get_component("probe1").unwrap()
            .as_any().downcast_ref::<TestProbe>().unwrap();
        assert_eq!(probe.ticks, 2);
    }

    #[test]
    fn test_propagate_mcu_write_unwired_pin_noop() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(TestProbe::new("probe1")));
        nl.build_wires(&[WireConfig { from: "PA5".into(), to: "probe1.in".into() }]);

        // Write to an unwired pin — should not affect probe
        nl.propagate_mcu_write(1, 0, true);
        let writes = nl.get_component("probe1").unwrap()
            .as_any().downcast_ref::<TestProbe>().unwrap().writes.len();
        assert_eq!(writes, 0);

        // Write to wired pin — should affect probe
        nl.propagate_mcu_write(0, 5, true);
        let writes = nl.get_component("probe1").unwrap()
            .as_any().downcast_ref::<TestProbe>().unwrap().writes.len();
        assert_eq!(writes, 1);
    }

    #[test]
    fn test_config_deserialize_led() {
        let yaml = r#"
cpu:
  svd: test.svd
  vector_table: 0x08000000
regions: []
components:
  - type: led
    name: my_led
wires:
  - from: PA1
    to: my_led.anode
"#;
        let cfg: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.components.is_some());
        assert!(cfg.wires.is_some());
        let comps = cfg.components.unwrap();
        assert_eq!(comps.len(), 1);
        let led = comps[0].clone().into_component();
        assert_eq!(led.name(), "my_led");
        assert_eq!(led.kind(), "led");
    }

    #[test]
    fn test_config_deserialize_button() {
        let yaml = r#"
cpu:
  svd: test.svd
  vector_table: 0x08000000
regions: []
components:
  - type: button
    name: btn1
    pulled_up: false
wires:
  - from: btn1.out
    to: PB3
"#;
        let cfg: crate::config::Config = serde_yaml::from_str(yaml).unwrap();
        let comps = cfg.components.unwrap();
        let btn = comps[0].clone().into_component();
        assert_eq!(btn.name(), "btn1");
        assert_eq!(btn.kind(), "button");
        // pulled_up=false means it outputs Low when not pressed
        assert_eq!(btn.read_pin(0), PinValue::Low);
    }

    #[test]
    fn test_three_component_bus() {
        let mut nl = Netlist::new();
        nl.add_component(Box::new(Led::new("led_a")));
        nl.add_component(Box::new(Led::new("led_b")));
        nl.add_component(Box::new(Button::new("btn", true)));
        nl.build_wires(&[
            WireConfig { from: "btn.out".into(), to: "PE0".into() },
            WireConfig { from: "PE0".into(), to: "led_a.anode".into() },
            WireConfig { from: "PE0".into(), to: "led_b.anode".into() },
        ]);

        // All 3 peripherals + MCU pin share one net
        assert_eq!(nl.nets.len(), 1);
        assert_eq!(nl.nets[0].endpoints.len(), 4);

        // Button not pressed -> High on bus
        assert_eq!(nl.read_mcu_pin(4, 0), Some(true));
        let led_a = nl.get_component("led_a").unwrap();
        assert_eq!(led_a.read_pin(0), PinValue::Low);

        // Propagate from MCU pin instead
        nl.propagate_mcu_write(4, 0, true);
        let led_a = nl.get_component("led_a").unwrap();
        assert_eq!(led_a.read_pin(0), PinValue::High);
        let led_b = nl.get_component("led_b").unwrap();
        assert_eq!(led_b.read_pin(0), PinValue::High);
    }
}
