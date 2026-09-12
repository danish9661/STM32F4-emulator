//! DMA2D (Chrom-ART accelerator, F429-only: 0x4002B000).
//!
//! The register file, job staging, and completion flags live here; pixel
//! movement runs in the JS driver (guest RAM is only reachable from there,
//! same split as the DMA engine). On a CR START write the job is validated
//! and staged for `dma2d_take_job()`; the driver gathers source lines,
//! runs them through the pure `convert_px`/`blend_px` helpers below via
//! `dma2d_convert`/`dma2d_blend`, scatters the output lines (honoring the
//! OR line offsets), then calls `dma2d_job_done()`, which sets TCIF and
//! pends IRQ56 when TCIE is set. START clears on completion.
//!
//! Supported color modes: 0 ARGB8888, 1 RGB888, 2 RGB565 (FG/BG/out).
//! Anything else raises CEIF (configuration error) at START. Blend alpha
//! comes from the FG pixel for ARGB8888, else FGPFCCR.ALPHA[31:24].

use std::sync::Mutex;

use crate::system::System;
use super::Peripheral;

const DMA2D_IRQ: i32 = 56;

// ISR/IFCR bit 1 = transfer-complete.
const TCIF: u32 = 1 << 1;
// CEIF bit 5 = configuration error.
const CEIF: u32 = 1 << 5;

#[derive(Clone, Copy, Default)]
struct Dma2dJob {
    mode: u32,
    w: u32,
    h: u32,
    fg_addr: u32,
    fg_cm: u32,
    fg_off: u32,
    bg_addr: u32,
    bg_cm: u32,
    bg_off: u32,
    out_addr: u32,
    out_cm: u32,
    out_off: u32,
    ocolr: u32,
    fgpfccr_alpha: u8,
}

// One staged job max (silicon runs one transfer at a time); guarded because
// cargo runs lib tests in parallel threads.
static DMA2D_JOB: Mutex<Option<Dma2dJob>> = Mutex::new(None);

/// Take the staged transfer for the JS driver (16 words, or empty).
pub fn take_job() -> Option<[u32; 16]> {
    DMA2D_JOB.lock().unwrap().take().map(|j| {
        [
            j.mode, j.w, j.h, j.fg_addr, j.fg_cm, j.fg_off, j.bg_addr, j.bg_cm,
            j.bg_off, j.out_addr, j.out_cm, j.out_off, j.ocolr,
            (j.fgpfccr_alpha) as u32, 0, 0,
        ]
    })
}

pub struct Dma2d {
    cr: u32,
    isr: u32,
    fgmar: u32,
    fgor: u32,
    bgmar: u32,
    bgor: u32,
    fgpfccr: u32,
    fgcolr: u32,
    bgpfccr: u32,
    bgcolr: u32,
    opfccr: u32,
    ocolr: u32,
    omar: u32,
    oor: u32,
    nlr: u32,
}

impl Default for Dma2d {
    fn default() -> Self {
        Self {
            cr: 0, isr: 0, fgmar: 0, fgor: 0, bgmar: 0, bgor: 0, fgpfccr: 0,
            fgcolr: 0, bgpfccr: 0, bgcolr: 0, opfccr: 0, ocolr: 0, omar: 0,
            oor: 0, nlr: 0,
        }
    }
}

impl Dma2d {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DMA2D" { Some(Box::new(Self::default())) } else { None }
    }

    fn out_cm(&self) -> u32 {
        self.opfccr & 7
    }

    /// Validate the programmed transfer and stage it for the JS driver.
    /// Bad geometry or color modes raise CEIF instead (silicon behavior).
    fn start(&mut self) {
        let mode = (self.cr >> 16) & 3;
        let w = (self.nlr >> 16) & 0x3FFF;
        let h = self.nlr & 0xFFFF;
        let fg_cm = self.fgpfccr & 0xF;
        let bg_cm = self.bgpfccr & 0xF;
        let out_cm = self.out_cm();
        let ok_formats = match mode {
            0 => true, // M2M: raw word copy, formats ignored
            3 => out_cm <= 2, // R2M: only the output format matters
            1 => fg_cm <= 2 && out_cm <= 2, // M2M+PFC
            _ => fg_cm <= 2 && bg_cm <= 2 && out_cm <= 2, // blend
        };
        if w == 0 || h == 0 || !ok_formats {
            self.isr |= CEIF;
            return;
        }
        *DMA2D_JOB.lock().unwrap() = Some(Dma2dJob {
            mode,
            w,
            h,
            fg_addr: self.fgmar,
            fg_cm,
            fg_off: self.fgor & 0x3FFF,
            bg_addr: self.bgmar,
            bg_cm,
            bg_off: self.bgor & 0x3FFF,
            out_addr: self.omar,
            out_cm,
            out_off: self.oor & 0x3FFF,
            ocolr: self.ocolr,
            fgpfccr_alpha: (self.fgpfccr >> 24) as u8,
        });
    }

    /// Complete the staged transfer (called by the JS driver after it moved
    /// the pixels): TCIF + IRQ56 when TCIE is set, START clears.
    pub fn job_done(&mut self, sys: &System) {
        self.isr |= TCIF;
        self.cr &= !1;
        if self.cr & (1 << 9) != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(DMA2D_IRQ);
        }
    }
}

/// Decode one pixel of the given color mode into (a, r, g, b).
fn decode_px(cm: u32, p: &[u8]) -> (u8, u8, u8, u8) {
    match cm {
        0 => {
            let v = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            ((v >> 24) as u8, (v >> 16) as u8, (v >> 8) as u8, v as u8)
        }
        1 => (255, p[0], p[1], p[2]),
        _ => {
            let v = u16::from_le_bytes([p[0], p[1]]) as u32;
            let r = ((v >> 11) & 31) * 255 / 31;
            let g = ((v >> 5) & 63) * 255 / 63;
            let b = (v & 31) * 255 / 31;
            (255, r as u8, g as u8, b as u8)
        }
    }
}

/// Encode one (a, r, g, b) pixel into the given color mode.
fn encode_px(cm: u32, a: u8, r: u8, g: u8, b: u8, out: &mut Vec<u8>) {
    match cm {
        0 => out.extend_from_slice(&((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32).to_le_bytes()),
        1 => out.extend_from_slice(&[r, g, b]),
        _ => {
            let v = ((r as u16 >> 3) << 11) | ((g as u16 >> 2) << 5) | (b as u16 >> 3);
            out.extend_from_slice(&v.to_le_bytes())
        }
    }
}

fn bpp(cm: u32) -> usize {
    match cm {
        0 => 4,
        1 => 3,
        _ => 2,
    }
}

/// Convert a line-packed (w*h, no gaps) pixel buffer between color modes.
pub fn convert_px(fg_cm: u32, out_cm: u32, px: &[u8]) -> Vec<u8> {
    let (fb, ob) = (bpp(fg_cm), bpp(out_cm));
    let n = px.len() / fb;
    let mut out = Vec::with_capacity(n * ob);
    for i in 0..n {
        let (a, r, g, b) = decode_px(fg_cm, &px[i * fb..]);
        encode_px(out_cm, a, r, g, b, &mut out);
    }
    out
}

/// Blend FG over BG ("over" operator) into the output mode. The FG alpha
/// comes from the pixel for ARGB8888, else from `fg_alpha` (FGPFCCR.ALPHA).
pub fn blend_px(fg_cm: u32, bg_cm: u32, out_cm: u32, fg_alpha: u8, fg: &[u8], bg: &[u8]) -> Vec<u8> {
    let (fb, bb, ob) = (bpp(fg_cm), bpp(bg_cm), bpp(out_cm));
    let n = (fg.len() / fb).min(bg.len() / bb);
    let mut out = Vec::with_capacity(n * ob);
    for i in 0..n {
        let (mut fa, fr, fg_, fb_) = decode_px(fg_cm, &fg[i * fb..]);
        if fg_cm != 0 {
            fa = fg_alpha;
        }
        let (ba, br, bg_, bb_) = decode_px(bg_cm, &bg[i * bb..]);
        let t = fa as u32;
        let u = 255 - t;
        let r = (fr as u32 * t + br as u32 * u + 127) / 255;
        let g = (fg_ as u32 * t + bg_ as u32 * u + 127) / 255;
        let b = (fb_ as u32 * t + bb_ as u32 * u + 127) / 255;
        let a = (t * 255 + ba as u32 * u + 127) / 255;
        encode_px(out_cm, a as u8, r as u8, g as u8, b as u8, &mut out);
    }
    out
}

impl Peripheral for Dma2d {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.isr,
            0x0C => self.fgmar,
            0x10 => self.fgor,
            0x14 => self.bgmar,
            0x18 => self.bgor,
            0x1C => self.fgpfccr,
            0x20 => self.fgcolr,
            0x24 => self.bgpfccr,
            0x28 => self.bgcolr,
            0x34 => self.opfccr,
            0x38 => self.ocolr,
            0x3C => self.omar,
            0x40 => self.oor,
            0x44 => self.nlr,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            // CR: MODE[17:16] + IE[13:8] + ABORT[2] + SUSP[1] + START[0].
            0x00 => {
                self.cr = value & 0x0003_3F07;
                if value & 4 != 0 {
                    // ABORT: drop any staged job, no flags.
                    *DMA2D_JOB.lock().unwrap() = None;
                } else if value & 1 != 0 {
                    self.start();
                }
            }
            // IFCR: write-1-clear.
            0x08 => self.isr &= !value,
            0x0C => self.fgmar = value,
            0x10 => self.fgor = value & 0x3FFF,
            0x14 => self.bgmar = value,
            0x18 => self.bgor = value & 0x3FFF,
            0x1C => self.fgpfccr = value,
            0x20 => self.fgcolr = value,
            0x24 => self.bgpfccr = value,
            0x28 => self.bgcolr = value,
            0x34 => self.opfccr = value & 7,
            0x38 => self.ocolr = value,
            0x3C => self.omar = value,
            0x40 => self.oor = value & 0x3FFF,
            0x44 => self.nlr = value,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::test_dummy_system;

    static DMA2D_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn argb(a: u8, r: u8, g: u8, b: u8) -> [u8; 4] {
        ((a as u32) << 24 | (r as u32) << 16 | (g as u32) << 8 | b as u32).to_le_bytes()
    }

    #[test]
    fn convert_565_roundtrip() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        // Pure red/green/blue/white/black through RGB565 -> ARGB8888.
        for (rgb565, expect) in [
            (0xF800u16, (255, 255, 0, 0)),
            (0x07E0, (255, 0, 255, 0)),
            (0x001F, (255, 0, 0, 255)),
            (0xFFFF, (255, 255, 255, 255)),
            (0x0000, (255, 0, 0, 0)),
        ] {
            let out = convert_px(2, 0, &rgb565.to_le_bytes());
            let v = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
            assert_eq!((v >> 24) as u8, expect.0, "alpha {rgb565:04X}");
            assert_eq!(((v >> 16) & 255) as u8, expect.1, "red {rgb565:04X}");
            assert_eq!(((v >> 8) & 255) as u8, expect.2, "green {rgb565:04X}");
            assert_eq!((v & 255) as u8, expect.3, "blue {rgb565:04X}");
        }
    }

    #[test]
    fn convert_argb8888_identity_sizes() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        let px = [argb(0x12, 0x34, 0x56, 0x78), argb(0xFF, 0, 0, 0xFF)];
        let flat: Vec<u8> = px.concat();
        assert_eq!(convert_px(0, 0, &flat), flat);
        // ... to RGB888 (3 bytes/px) and RGB565 (2 bytes/px).
        assert_eq!(convert_px(0, 1, &flat).len(), 6);
        assert_eq!(convert_px(0, 2, &flat).len(), 4);
        let back = convert_px(1, 0, &convert_px(0, 1, &flat));
        assert_eq!(back.len(), 8);
    }

    #[test]
    fn blend_semi_red_over_blue() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        // FG a=128 red over opaque blue (the firmware T4 vector).
        let out = blend_px(0, 0, 0, 0, &argb(128, 255, 0, 0), &argb(255, 0, 0, 255));
        assert_eq!(out, argb(255, 128, 0, 127));
    }

    #[test]
    fn blend_opaque_fg_wins() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        let out = blend_px(0, 0, 0, 0, &argb(255, 10, 20, 30), &argb(255, 200, 210, 220));
        assert_eq!(out, argb(255, 10, 20, 30));
    }

    #[test]
    fn start_rejects_bad_config_with_ceif() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        let mut d = Dma2d::default();
        // Zero-size transfer.
        d.write(&sys, 0x44, 0);
        d.write(&sys, 0x00, 1);
        assert_eq!(d.read(&sys, 0x04) & CEIF, CEIF, "empty NLR raises CEIF");
        assert!(take_job().is_none(), "nothing staged");
        // Unsupported FG format (L8 = 5) in M2M+PFC mode.
        d.write(&sys, 0x44, (4 << 16) | 4);
        d.write(&sys, 0x1C, 5);
        d.write(&sys, 0x00, (1 << 16) | 1);
        assert_eq!(d.read(&sys, 0x04) & CEIF, CEIF, "bad CM raises CEIF");
        assert!(take_job().is_none(), "nothing staged");
    }

    #[test]
    fn start_stages_r2m_job() {
        let _g = DMA2D_TEST_LOCK.lock().unwrap();
        let sys = test_dummy_system();
        let mut d = Dma2d::default();
        d.write(&sys, 0x3C, 0x20001000); // OMAR
        d.write(&sys, 0x44, (32 << 16) | 16); // 32x16
        d.write(&sys, 0x34, 0); // ARGB8888 out
        d.write(&sys, 0x38, 0xFFFF0000); // red
        d.write(&sys, 0x00, (3 << 16) | 1); // R2M + START
        let job = take_job().expect("job staged");
        assert_eq!(&job[0..3], &[3, 32, 16]);
        assert_eq!(job[9], 0x20001000);
        assert_eq!(job[12], 0xFFFF0000);
    }
}
