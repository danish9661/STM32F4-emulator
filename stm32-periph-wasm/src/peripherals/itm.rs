use crate::system::{System, get_uart_output};
use super::Peripheral;

/// ITM stimulus console (0xE0000000 block): all 32 STIMn ports are live.
/// Port 0 keeps the historical behavior — writes go to the UART output when
/// TCR.ITMENA (bit 0) and TER[0] are both set (the standard ITM_SendChar
/// debug-console idiom, sunk into our terminal instead of a (nonexistent)
/// SWO trace port). Ports 1-31 queue per-port byte streams the JS driver
/// drains via `itm_take_port(n)` (`itm_port_pending(n)` reports backlog),
/// so multi-channel firmware tracing (one port per task/module) works
/// without a debugger attached. Everything else is RAZ/WI. With tracing
/// disabled (reset state) all stimulus writes are ignored, exactly like a
/// debugger-disconnected target. One byte per write is delivered (the low
/// byte; the bus layer merges sub-word accesses there), matching the
/// byte-oriented SendChar pattern. A write to a port whose TER bit is clear
/// is dropped (silicon gates per-port), and STIMn reads nonzero (FIFO
/// ready) only for enabled ports — what CMSIS ITM_SendChar polls on.
pub struct Itm {
    ter: u32,
    tpr: u32,
    tcr: u32,
    ports: [Vec<u8>; 32],
}

impl Itm {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "ITM" {
            Self::default_opt()
        } else {
            None
        }
    }

    fn default_opt() -> Option<Box<dyn Peripheral>> {
        let ports: [Vec<u8>; 32] = Default::default();
        Some(Box::new(Self { ter: 0, tpr: 0, tcr: 0, ports }))
    }

    fn tracing(&self) -> bool {
        self.tcr & 1 != 0
    }

    fn port_enabled(&self, port: usize) -> bool {
        port < 32 && self.tracing() && (self.ter >> port) & 1 != 0
    }

    fn enabled(&self) -> bool {
        self.port_enabled(0)
    }

    /// Drain a port's queued bytes (oldest first). Empty when nothing was
    /// written since the last drain.
    pub fn take_port(&mut self, port: usize) -> Vec<u8> {
        if port >= 32 {
            return Vec::new();
        }
        std::mem::take(&mut self.ports[port])
    }

    /// Queued backlog for a port (bytes waiting for a drain).
    pub fn port_pending(&self, port: usize) -> usize {
        if port >= 32 {
            return 0;
        }
        self.ports[port].len()
    }
}

impl Peripheral for Itm {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            // STIMn (n = offset/4, 0..31): nonzero (FIFO ready) only when
            // tracing is on AND that port's TER bit is set — what CMSIS
            // ITM_SendChar polls on before writing.
            0x0..=0x7C => {
                let port = (offset / 4) as usize;
                if self.port_enabled(port) {
                    1
                } else {
                    0
                }
            }
            0xE00 => self.ter,
            0xE40 => self.tpr,
            0xE80 => self.tcr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            // STIMn write: port 0 sinks into the UART console (legacy
            // behavior); ports 1-31 queue per-port bytes for itm_take_port.
            // Writes to TER-disabled ports are dropped (silicon gates them).
            0x0..=0x7C => {
                let port = (offset / 4) as usize;
                if !self.port_enabled(port) {
                    return;
                }
                let b = value as u8;
                if port == 0 {
                    get_uart_output().lock().unwrap().push(b as char);
                } else if self.ports[port].len() < 4096 {
                    self.ports[port].push(b);
                }
            }
            0xE00 => self.ter = value,
            0xE40 => self.tpr = value,
            0xE80 => self.tcr = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::peripherals::Peripheral;

    fn itm() -> Itm {
        Itm { ter: 0, tpr: 0, tcr: 0, ports: Default::default() }
    }

    #[test]
    fn port0_sinks_to_uart_and_stim_reads_ready() {
        let sys = crate::system::test_dummy_system();
        let mut it = itm();
        // Disabled: writes ignored, STIM reads 0.
        it.write(&sys, 0x0, b'A' as u32);
        assert_eq!(it.read(&sys, 0x0), 0, "STIM0 gated while disabled");
        // Enable tracing + port 0.
        it.write(&sys, 0xE80, 1); // TCR.ITMENA
        it.write(&sys, 0xE00, 1); // TER[0]
        assert_eq!(it.read(&sys, 0x0), 1, "STIM0 ready once enabled");
        it.write(&sys, 0x0, b'Z' as u32);
        let out = crate::system::get_uart_output().lock().unwrap().clone();
        assert!(out.ends_with('Z'), "port-0 byte reaches UART, tail={out:?}");
    }

    #[test]
    fn ports_1_to_31_queue_per_port_and_gate_on_ter() {
        let sys = crate::system::test_dummy_system();
        let mut it = itm();
        it.write(&sys, 0xE80, 1); // TCR.ITMENA (TER still 0)
        // TER-disabled ports drop writes.
        it.write(&sys, 0x04, b'A' as u32); // STIM1
        assert_eq!(it.port_pending(1), 0, "TER-gated port drops");
        assert_eq!(it.read(&sys, 0x04), 0, "STIM1 reads 0 while gated");
        // Enable ports 1 and 31.
        it.write(&sys, 0xE00, (1 << 1) | (1 << 31));
        assert_eq!(it.read(&sys, 0x04), 1, "STIM1 ready");
        assert_eq!(it.read(&sys, 0x7C), 1, "STIM31 ready");
        it.write(&sys, 0x04, b'H' as u32);
        it.write(&sys, 0x04, b'i' as u32);
        it.write(&sys, 0x7C, b'!' as u32);
        assert_eq!(it.port_pending(1), 2);
        assert_eq!(it.take_port(1), vec![b'H', b'i'], "port-1 stream in order");
        assert_eq!(it.port_pending(1), 0, "drain empties");
        assert_eq!(it.take_port(31), vec![b'!'], "port-31 isolated from port-1");
        assert!(it.take_port(32).is_empty(), "out-of-range port drains empty");
    }
}
