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

/// 256-entry color lookup table (one per layer): CLUTWR writes load
/// address + RGB here (silicon auto-increments CLUTADD after each write
/// when live; the model tracks the address explicitly). Indexed pixel
/// formats (L8 pf=5, AL44 pf=6, AL88 pf=7) resolve each framebuffer byte
/// through this table — without it LUT layers render black.
#[derive(Clone, Copy)]
struct Clut {
    addr: u8,
    table: [u32; 256],
}

impl Default for Clut {
    fn default() -> Self {
        Self { addr: 0, table: [0; 256] }
    }
}

pub struct Ltdc {
    sscr: u32, bpcr: u32, awcr: u32, twcr: u32,
    gcr: u32, srcr: u32, bccr: u32,
    ier: u32, isr: u32, lipcr: u32,
    layers: [Lx; NUM_LAYERS as usize],
    cluts: [Clut; NUM_LAYERS as usize],
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
            cluts: [Clut::default(); NUM_LAYERS as usize],
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

    /// CLUT entry (scope probe): the 24-bit RGB the indexed formats
    /// resolve a framebuffer byte through on layer `li`.
    pub fn clut_entry(&self, li: usize, idx: u8) -> u32 {
        self.cluts.get(li).map(|c| c.table[idx as usize]).unwrap_or(0)
    }

    /// Resolve one indexed framebuffer byte to ARGB8888 through the
    /// layer's CLUT (L8: byte = index; AL44: low nibble = index, high =
    /// alpha; AL88: byte = index with full alpha). Scope probe for the
    /// LUT-indexed render path (what the JS sink paints per pixel).
    pub fn lut_pixel(&self, li: usize, pf: u32, byte: u8) -> u32 {
        let entry = self.clut_entry(li, match pf {
            6 => byte & 0x0F, // AL44: low nibble indexes
            _ => byte,        // L8 (5) / AL88 (7): full byte indexes
        });
        let rgb = entry & 0xFFFFFF;
        match pf {
            6 => {
                let a = (byte >> 4) as u32; // high nibble = alpha
                (a * 0x11 << 24) | rgb // expand 4-bit alpha to 8
            }
            7 => 0xFF00_0000 | rgb, // AL88: full alpha
            _ => 0xFF00_0000 | rgb, // L8: full alpha
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
                // CLUTWR readback: current CLUT address in bits 31:24
                // (silicon reads back the load pointer; the RGB field
                // is write-only).
                0x40 => (self.cluts[_li as usize].addr as u32) << 24,
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
                // CLUTWR: CLUTADD (bits 31:24) selects the entry, RGB
                // (bits 23:0) loads it; the address auto-increments
                // (silicon loads the LUT sequentially without re-addressing).
                0x40 => {
                    self.layers[_li as usize].clutwr = value & 0xFF_FFFF_FF;
                    let clut = &mut self.cluts[_li as usize];
                    clut.addr = ((value >> 24) & 0xFF) as u8;
                    clut.table[clut.addr as usize] = value & 0xFFFFFF;
                    clut.addr = clut.addr.wrapping_add(1);
                }
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
#[cfg(test)]
mod gap10_tests {
    use super::*;
    use crate::system::test_dummy_system;

    // CLUTWR loads entries with auto-increment; readback shows the load
    // pointer; lut_pixel resolves L8/AL44/AL88 bytes through the table.
    #[test]
    fn clut_loads_and_lut_resolves() {
        let sys = test_dummy_system();
        let mut l = Ltdc::default();
        let _ = &sys;
        // Layer 0 base is 0x84: CLUTWR at 0x84+0x40 = 0xC4.
        l.write(&sys, 0xC4, (0 << 24) | 0xFF0000); // idx0 = red
        // Second write carries its own address field (silicon: CLUTADD is
        // a register field, auto-increment sets it for the NEXT write —
        // firmware may still address explicitly; both land correctly).
        l.write(&sys, 0xC4, (1 << 24) | 0x00FF00); // idx1 = green
        assert_eq!(l.read(&sys, 0xC4) >> 24, 2, "CLUTADD auto-increments");
        assert_eq!(l.clut_entry(0, 0), 0xFF0000);
        assert_eq!(l.clut_entry(0, 1), 0x00FF00);
        assert_eq!(l.lut_pixel(0, 5, 0), 0xFFFF0000, "L8 idx0 = red");
        assert_eq!(l.lut_pixel(0, 5, 1), 0xFF00FF00, "L8 idx1 = green");
        // AL44: low nibble indexes, high nibble = alpha*0x11.
        assert_eq!(l.lut_pixel(0, 6, 0x81), 0x8800FF00, "AL44 a=8 idx1");
        // AL88: full alpha + table RGB.
        assert_eq!(l.lut_pixel(0, 7, 0), 0xFFFF0000, "AL88 idx0");
    }
}
