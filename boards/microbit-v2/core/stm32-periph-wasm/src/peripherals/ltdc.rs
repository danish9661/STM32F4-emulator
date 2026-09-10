use crate::system::System;
use super::Peripheral;

const LAYER_BASE: u32 = 0x84;
const LAYER_STRIDE: u32 = 0x80;
const NUM_LAYERS: u32 = 2;

const LTDC_IRQ: i32 = 88;

fn lx_bis(offset: u32) -> Option<(u32, u32)> {
    if offset >= LAYER_BASE {
        let layer_rel = offset - LAYER_BASE;
        let layer_idx = layer_rel / LAYER_STRIDE;
        if layer_idx < NUM_LAYERS {
            Some((layer_idx, layer_rel % LAYER_STRIDE))
        } else { None }
    } else { None }
}

#[derive(Clone, Copy, Default)]
struct Lx {
    cr: u32, whpcr: u32, wvpcr: u32, ckcr: u32,
    pfcr: u32, cacr: u32, dccr: u32, bfcr: u32,
    cfbar: u32, cfblr: u32, cfblnr: u32, clutwr: u32,
}

pub struct Ltdc {
    sscr: u32, bpcr: u32, awcr: u32, twcr: u32,
    gcr: u32, srcr: u32, bccr: u32,
    ier: u32, isr: u32, lipcr: u32,
    layers: [Lx; NUM_LAYERS as usize],
    // Scanout pacing: pixel/line counters advanced per tick while LTDCEN
    // is set. Drives line (LIPCR) and frame-end (F) interrupt flags so the
    // JS display sink can render each frame.
    scan_px: u32,
    scan_line: u32,
    scan_frame: u32,
    /// Last INSTRUCTION_COUNT seen. Scanout is instruction-count driven
    /// (like the timers): each tick() advances by 2px per elapsed
    /// instruction, so batched tick_n(delta) advances correctly. A fixed
    /// 2px-per-tick() stalls under batching (wasm steps tick_n(100k) once).
    last_tick: u64,
}

impl Default for Ltdc {
    fn default() -> Self {
        Self {
            sscr: 0, bpcr: 0, awcr: 0, twcr: 0,
            gcr: 0x2220, srcr: 0, bccr: 0,
            ier: 0, isr: 0, lipcr: 0,
            layers: {
                let mut lx = [Lx::default(); NUM_LAYERS as usize];
                lx[0].bfcr = 0x0607;
                lx[1].bfcr = 0x0607;
                lx
            },
            scan_px: 0, scan_line: 0, scan_frame: 0,
            last_tick: crate::system::instruction_count(),
        }
    }
}

impl Ltdc {
    /// Current scanline within the frame (0 = just after sync). 0xFFFF when
    /// the controller is disabled.
    pub fn scanline(&self) -> u32 {
        if self.gcr & 1 == 0 { 0xFFFF } else { self.scan_line }
    }

    /// Completed frames since enable.
    pub fn frame_count(&self) -> u32 {
        self.scan_frame
    }

    /// Advance the scanout: a few pixels per tick keeps this cheap; line
    /// geometry follows the real SSCR/BPCR/AWCR values. The line interrupt
    /// fires when the scanline crosses LIPCR; the frame-end flag (ISR bit 1)
    /// fires after the active + vertical-blanking lines.
    fn scan_tick(&mut self, sys: &System) {
        if self.gcr & 1 == 0 {
            return;
        }
        let now = crate::system::instruction_count();
        let delta = now.wrapping_sub(self.last_tick);
        self.last_tick = now;
        if delta == 0 {
            return;
        }
        let active_w = ((self.awcr & 0xFFF) + 1) as u32;
        let hbp = ((self.bpcr & 0xFFF) + 1) as u32;
        let hspw = ((self.sscr & 0xFFF) + 1) as u32;
        let line_px = active_w + hbp + hspw + 1;
        let active_h = ((self.awcr >> 16) & 0xFFF) as u32 + 1;
        let vbp = ((self.bpcr >> 16) & 0xFFF) as u32 + 1;
        let vspw = ((self.sscr >> 16) & 0xFFF) as u32 + 1;
        let frame_lines = active_h + vbp + vspw;
        // Advance 2px per elapsed instruction, carrying whole lines/frames
        // (a 100k-instruction batch crosses many lines; the old fixed
        // 2px-per-tick stalled under it).
        let mut px = self.scan_px as u64 + 2 * delta.min(u64::from(u32::MAX));
        let lip = self.lipcr & 0x7FF;
        loop {
            if px < u64::from(line_px) {
                break;
            }
            px -= u64::from(line_px);
            self.scan_line += 1;
            if self.scan_line == lip {
                self.isr |= 1 << 0; // LIF
                self.fire_interrupts(sys);
            }
            if self.scan_line >= frame_lines {
                self.scan_line = 0;
                self.scan_frame = self.scan_frame.wrapping_add(1);
                self.isr |= 1 << 1; // F flag
                self.fire_interrupts(sys);
            }
        }
        self.scan_px = px.min(u64::from(u32::MAX)) as u32;
    }
}

impl Ltdc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "LTDC" { Some(Box::new(Self::default())) } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.isr & self.ier & 0x0F != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(LTDC_IRQ);
        }
    }
}

impl Peripheral for Ltdc {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn tick(&mut self, sys: &System) {
        self.scan_tick(sys);
    }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        if let Some((_li, lr)) = lx_bis(offset) {
            return match lr {
                0x00 => self.layers[_li as usize].cr,
                0x04 => self.layers[_li as usize].whpcr,
                0x08 => self.layers[_li as usize].wvpcr,
                0x0C => self.layers[_li as usize].ckcr,
                0x10 => self.layers[_li as usize].pfcr,
                0x14 => self.layers[_li as usize].cacr,
                0x18 => self.layers[_li as usize].dccr,
                0x1C => self.layers[_li as usize].bfcr,
                0x28 => self.layers[_li as usize].cfbar,
                0x2C => self.layers[_li as usize].cfblr,
                0x30 => self.layers[_li as usize].cfblnr,
                0x40 => 0,
                _ => 0,
            };
        }
        match offset {
            0x08 => self.sscr,
            0x0C => self.bpcr,
            0x10 => self.awcr,
            0x14 => self.twcr,
            0x18 => self.gcr,
            0x24 => self.srcr,
            0x2C => self.bccr,
            0x34 => self.ier,
            0x38 => self.isr,
            0x40 => self.lipcr,
            0x44 => 0,
            0x48 => 0x0F,
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        if let Some((_li, lr)) = lx_bis(offset) {
            match lr {
                0x00 => self.layers[_li as usize].cr = value & 0x13,
                0x04 => self.layers[_li as usize].whpcr = value & 0x0FFF_0FFF,
                0x08 => self.layers[_li as usize].wvpcr = value & 0x07FF_07FF,
                0x0C => self.layers[_li as usize].ckcr = value & 0x00FF_FFFF,
                0x10 => self.layers[_li as usize].pfcr = value & 0x07,
                0x14 => self.layers[_li as usize].cacr = value & 0xFF,
                0x18 => self.layers[_li as usize].dccr = value,
                0x1C => self.layers[_li as usize].bfcr = value & 0x0707,
                0x28 => self.layers[_li as usize].cfbar = value & 0xFFFF_FFFF,
                0x2C => self.layers[_li as usize].cfblr = value & 0x1FFF_1FFF,
                0x30 => self.layers[_li as usize].cfblnr = value & 0x07FF,
                0x40 => self.layers[_li as usize].clutwr = value & 0xFF_FFFF_FF,
                _ => {}
            }
            return;
        }
        match offset {
            0x08 => self.sscr = value & 0x0FFF_0FFF,
            0x0C => self.bpcr = value & 0x0FFF_0FFF,
            0x10 => self.awcr = value & 0x0FFF_0FFF,
            0x14 => self.twcr = value & 0x0FFF_0FFF,
            0x18 => self.gcr = value & 0xF331_1111,
            0x24 => {
                self.srcr = value & 0x03;
                if value & 0x01 != 0 {
                    self.isr |= 1 << 3;
                    self.fire_interrupts(sys);
                }
                if value & 0x02 != 0 {
                    self.isr |= 1 << 2;
                    self.fire_interrupts(sys);
                }
            }
            0x2C => self.bccr = value & 0x00FF_FFFF,
            0x34 => {
                self.ier = value & 0x0F;
                self.fire_interrupts(sys);
            }
            0x3C => {
                self.isr &= !(value & 0x0F);
            }
            0x40 => self.lipcr = value & 0x07FF,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scanout_advances_line_and_frame_flags() {
        let mut l = Ltdc::default();
        // active 40x20; HSPW=4,HBP=4 -> 5 each with the +1 -> line span
        // 40 + 5 + 5 + 1 = 51 px; at 2 px/tick a line takes 26 ticks.
        l.sscr = 0x0003_0004;
        l.bpcr = 0x0003_0004;
        l.awcr = (20 - 1) << 16 | (40 - 1);
        l.lipcr = 10;
        l.ier = 0x0F;
        let sys = crate::system::test_dummy_system();
        l.gcr |= 1; // LTDCEN
        for _ in 0..26 {
            crate::system::INSTRUCTION_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            l.scan_tick(&sys);
        }
        assert_eq!(l.scanline(), 1);
        for _ in 0..(26 * 9) {
            crate::system::INSTRUCTION_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            l.scan_tick(&sys);
        }
        assert_eq!(l.scanline(), 10);
        assert_ne!(l.isr & 1, 0, "LIF set");
        // frame end after active 20 + VBP 4 + VSPW 4 = 28 lines
        for _ in 0..(26 * 18) {
            crate::system::INSTRUCTION_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            l.scan_tick(&sys);
        }
        assert_eq!(l.scanline(), 0, "scanline wraps");
        assert_ne!(l.isr & 2, 0, "F flag set");
        assert_eq!(l.frame_count(), 1);
    }

    #[test]
    fn scanout_idle_when_disabled() {
        let mut l = Ltdc::default();
        l.awcr = (20 - 1) << 16 | (40 - 1);
        let sys = crate::system::test_dummy_system();
        for _ in 0..1000 { l.scan_tick(&sys); }
        assert_eq!(l.scanline(), 0xFFFF);
        assert_eq!(l.frame_count(), 0);
        assert_eq!(l.isr & 3, 0);
    }

    #[test]
    fn fire_interrupts_on_line_flag() {
        let mut l = Ltdc::default();
        l.sscr = 0x0003_0004;
        l.bpcr = 0x0003_0004;
        l.awcr = (20 - 1) << 16 | (40 - 1);
        l.lipcr = 2;
        l.ier = 0x0F;
        let sys = crate::system::test_dummy_system();
        assert!(!sys.p.nvic.borrow().irq_pending(LTDC_IRQ));
        l.gcr |= 1;
        for _ in 0..(26 * 3) {
            crate::system::INSTRUCTION_COUNT
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            l.scan_tick(&sys);
        }
        assert_ne!(l.isr & 1, 0);
        // Pending is set even without ISER (delivery is what needs enable).
        assert!(sys.p.nvic.borrow().irq_pending(LTDC_IRQ));
    }
}