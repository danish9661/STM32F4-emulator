use crate::system::System;
use super::Peripheral;

const DCMI_IRQ: i32 = 78;

// Pixels consumed from the JS-fed frame per model tick. The real DCMI
// consumes one pixel per PCLK; PCLK is the camera's clock, not the CPU's,
// so a ratio > 1 just speeds up frame delivery without changing semantics.
const PIXELS_PER_TICK: usize = 16;
const FIFO_DEPTH: usize = 4;

/// DCMI (Digital Camera Interface) controller. On-chip only: the pixel
/// source is an external camera sensor fed by the JS hardware layer via
/// `dcmi_feed_frame` — the model consumes that frame with VSYNC/LINE/FRAME
/// semantics and a small pixel FIFO, exactly like the real peripheral.
pub struct Dcmi {
    cr: u32, sr: u32, ris: u32, ier: u32,
    escr: u32, esur: u32, cwstrt: u32, cwsiz: u32, dr: u32,
    fifo: Vec<u8>,
    // Consumed-frame cursor + geometry
    frame: Option<(u32, u32, Vec<u8>)>,
    frame_w: u32, frame_h: u32,
    frame_x: u32, frame_y: u32,
    vsync: bool,
}

impl Default for Dcmi {
    fn default() -> Self {
        Self {
            cr: 0, sr: 0, ris: 0, ier: 0,
            escr: 0, esur: 0, cwstrt: 0, cwsiz: 0, dr: 0,
            fifo: Vec::new(),
            frame: None, frame_w: 0, frame_h: 0,
            frame_x: 0, frame_y: 0, vsync: false,
        }
    }
}

impl Dcmi {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "DCMI" { Some(Box::new(Self::default())) } else { None }
    }

    fn fire_interrupts(&mut self, sys: &System) {
        if self.ris & self.ier != 0 {
            sys.p.nvic.borrow_mut().set_intr_pending(DCMI_IRQ);
        }
    }

    fn feed_next_pixel(&mut self) {
        let Some((w, h, data)) = &self.frame else { return };
        let (w, h) = (*w, *h);
        if self.frame_y >= h { return; }
        let idx = (self.frame_y * w + self.frame_x) as usize;
        if let Some(&px) = data.get(idx) {
            if self.fifo.len() >= FIFO_DEPTH {
                // FIFO overflow: drop the oldest, flag OVR.
                self.fifo.remove(0);
                self.ris |= 1 << 3;
            }
            self.fifo.push(px);
        }
        self.frame_x += 1;
        if self.frame_x >= w {
            self.frame_x = 0;
            self.frame_y += 1;
            // LINE complete.
            self.ris |= 1 << 1;
            self.sr |= 1 << 1;
            if self.frame_y >= h {
                // FRAME complete: FRS flag, VSYNC deassert, capture done.
                self.ris |= 1 << 2;
                self.sr |= 1 << 2;
                self.vsync = false;
                self.cr &= !1; // CAPTURE auto-clears, like the real part
            }
        }
    }
}

impl Peripheral for Dcmi {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any { self }
    fn read(&mut self, sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.cr,
            0x04 => self.sr,
            0x08 => self.ris,
            0x0C => self.ier,
            0x10 => { let v = self.ris; self.ris = 0; v }
            0x14 => self.escr,
            0x18 => self.esur,
            0x1C => self.cwstrt,
            0x20 => self.cwsiz,
            0x28 => {
                let v = if let Some(px) = self.fifo.first().copied() {
                    self.fifo.remove(0);
                    px as u32
                } else {
                    0
                };
                self.dr = v;
                self.fire_interrupts(sys);
                v
            }
            _ => 0,
        }
    }

    fn write(&mut self, sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let was_capture = self.cr & 1 != 0;
                self.cr = value & 0x7FFF_3FFF;
                let now_capture = self.cr & 1 != 0;
                if now_capture && !was_capture {
                    // CAPTURE rising: start consuming the JS-fed frame.
                    if let Some((w, h, data)) = crate::system::dcmi_frame() {
                        self.frame = Some((w, h, data));
                        self.frame_w = w;
                        self.frame_h = h;
                        self.frame_x = 0;
                        self.frame_y = 0;
                        self.fifo.clear();
                        self.vsync = true;
                        self.sr |= 1;      // VSYNC
                        self.ris |= 1 << 2; // FRS pending at frame start too
                    } else {
                        self.frame = None;
                    }
                }
            }
            0x0C => {
                self.ier = value & 0x1F;
                self.fire_interrupts(sys);
            }
            0x10 => {
                self.ris &= !(value & 0x1F);
                self.sr &= !(value & 0x1F) & 0x1F;
            }
            0x14 => self.escr = value & 0x3FF,
            0x18 => self.esur = value & 0xFF_FFFF,
            0x1C => self.cwstrt = value & 0x3FFF,
            0x20 => self.cwsiz = value & 0x3FFF,
            _ => {}
        }
    }

    fn tick(&mut self, sys: &System) {
        if self.cr & 1 == 0 { return; }
        if self.frame.is_none() {
            if let Some((w, h, data)) = crate::system::dcmi_frame() {
                self.frame = Some((w, h, data));
                self.frame_w = w;
                self.frame_h = h;
                self.frame_x = 0;
                self.frame_y = 0;
                self.fifo.clear();
                self.vsync = true;
            } else {
                return;
            }
        }
        for _ in 0..PIXELS_PER_TICK {
            if self.frame_y >= self.frame_h { break; }
            self.feed_next_pixel();
        }
        if self.frame_y >= self.frame_h {
            // Frame fully consumed: drop it so the next CAPTURE start
            // reloads whatever the JS camera has fed since.
            self.frame = None;
        }
        self.fire_interrupts(sys);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // DCMI_FRAME is process-global; serialize with other dcmi tests.
    static DCMI_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn consumes_js_fed_frame_with_line_and_frame_flags() {
        let _lock = DCMI_TEST_LOCK.lock().unwrap();
        crate::system::dcmi_feed_frame(8, 4, &(0u8..32).collect::<Vec<u8>>());
        let sys = crate::system::test_dummy_system();
        let slot = sys.p.peripherals.iter().find(|s| {
            s.peripheral.borrow_mut().as_any_mut().downcast_ref::<Dcmi>().is_some()
        }).expect("dcmi slot");
        let mut d = slot.peripheral.borrow_mut();
        let dcmi = d.as_any_mut().downcast_mut::<Dcmi>().unwrap();

        dcmi.cr = 1; // CAPTURE
        let sys = sys.clone();
        for _ in 0..40 { Dcmi::tick(dcmi, &sys); }

        // Whole 8x4 frame consumed (40 ticks x 16 px = 640 >= 32 pixels).
        assert_eq!(dcmi.frame_y, 4, "frame consumed");
        assert!(dcmi.ris & (1 << 2) != 0, "FRAME flag set");
        assert!(dcmi.ris & (1 << 1) != 0, "LINE flag set");
        assert_eq!(dcmi.cr & 1, 0, "CAPTURE auto-clears at frame end");

        // The FIFO (depth 4) holds the frame's last pixels.
        crate::system::dcmi_feed_frame(8, 4, &(0xAAu8..0xAA + 32).collect::<Vec<u8>>());
        dcmi.cr = 1;
        for _ in 0..40 { Dcmi::tick(dcmi, &sys); }
        assert_eq!(dcmi.fifo.first().copied().unwrap_or(0), 0xAA + 28, "fifo tail pixel");
        dcmi.fifo.clear();
        crate::system::dcmi_clear();
    }
}
