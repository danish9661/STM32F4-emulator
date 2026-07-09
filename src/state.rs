use serde::Serialize;
use std::collections::HashMap;

use crate::peripherals::gpio::GpioPorts;

#[derive(Serialize)]
pub struct EmulatorState {
    pub cpu: CpuState,
    pub gpio: HashMap<String, GpioPortState>,
    pub components: HashMap<String, ComponentState>,
}

#[derive(Serialize)]
pub struct CpuState {
    pub pc: u32,
    pub sp: u32,
    pub num_instructions: u64,
}

#[derive(Serialize)]
pub struct GpioPortState {
    pub port: String,
    pub output: u16,
    pub input: u16,
}

#[derive(Serialize)]
pub struct ComponentState {
    pub kind: String,
    pub pins: HashMap<String, String>,
}

pub fn collect_state(
    sys: &crate::system::System,
    gpio: &GpioPorts,
    num_instructions: u64,
) -> EmulatorState {
    use unicorn_engine::RegisterARM;
    let uc = sys.uc.borrow();
    let pc = uc.reg_read(RegisterARM::PC).unwrap_or(0) as u32;
    let sp = uc.reg_read(RegisterARM::SP).unwrap_or(0) as u32;

    let mut gpio_state = HashMap::new();
    for (port_idx, _read_cbs) in gpio.read_callbacks.iter().enumerate() {
        let port_letter = (b'A' + port_idx as u8) as char;
        gpio_state.insert(
            format!("GPIO{}", port_letter),
            GpioPortState {
                port: format!("GPIO{}", port_letter),
                output: 0,
                input: 0,
            },
        );
    }

    let mut components = HashMap::new();
    if let Some(ref n) = sys.n {
        let n = n.borrow();
        for comp in &n.components {
            let mut pins = HashMap::new();
            for i in 0..comp.pin_count() {
                let pname = comp.pin_name(i).unwrap_or("").to_string();
                let pval = format!("{:?}", comp.read_pin(i));
                pins.insert(pname, pval);
            }
            components.insert(
                comp.name().to_string(),
                ComponentState {
                    kind: comp.kind().to_string(),
                    pins,
                },
            );
        }
    }

    EmulatorState {
        cpu: CpuState { pc, sp, num_instructions },
        gpio: gpio_state,
        components,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_serialize_empty() {
        let state = EmulatorState {
            cpu: CpuState { pc: 0, sp: 0, num_instructions: 0 },
            gpio: HashMap::new(),
            components: HashMap::new(),
        };
        let json = serde_json::to_string(&state).unwrap();
        assert!(json.contains("\"cpu\""));
        assert!(json.contains("\"gpio\""));
        assert!(json.contains("\"components\""));
    }

    #[test]
    fn test_state_serialize_with_component() {
        let mut comps = HashMap::new();
        let mut pins = HashMap::new();
        pins.insert("anode".to_string(), "High".to_string());
        pins.insert("cathode".to_string(), "Low".to_string());
        comps.insert("led1".to_string(), ComponentState {
            kind: "led".to_string(),
            pins,
        });
        let state = EmulatorState {
            cpu: CpuState { pc: 0x08000000, sp: 0x20010000, num_instructions: 42 },
            gpio: HashMap::new(),
            components: comps,
        };
        let json = serde_json::to_string_pretty(&state).unwrap();
        assert!(json.contains("\"pc\": 134217728"));
        assert!(json.contains("\"kind\": \"led\""));
        assert!(json.contains("\"anode\": \"High\""));
    }
}
