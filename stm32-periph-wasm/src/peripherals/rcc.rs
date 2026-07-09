use crate::system::{System, instruction_count};
use super::Peripheral;

const HSI_FREQ: u64 = 16_000_000;
const HSE_FREQ: u64 = 8_000_000;
const LSI_FREQ: u64 = 32_000;

pub struct Rcc {
    cr: u32,
    pllcfgr: u32,
    cfgr: u32,
    cir: u32,
    ahb1rstr: u32, ahb2rstr: u32, ahb3rstr: u32,
    apb1rstr: u32, apb2rstr: u32,
    ahb1enr: u32, ahb2enr: u32, ahb3enr: u32,
    apb1enr: u32, apb2enr: u32,
    ahb1lpenr: u32, ahb2lpenr: u32, ahb3lpenr: u32,
    apb1lpenr: u32, apb2lpenr: u32,
    bdcr: u32, csr: u32,
    sscg: u32, plli2scfgr: u32, pllsai: u32,
    dckcfgr: u32, ckgatenr: u32, dckcfgr2: u32,

    hse_on_inst: u64,
    pll_on_inst: u64,
    lsi_on_inst: u64,
    lse_on_inst: u64,
}

impl Default for Rcc {
    fn default() -> Self {
        Self {
            cr: 0x0000_0003,
            cfgr: 0x0000_0000,
            ahb1enr: 0x0010_0000,
            bdcr: 0x0000_0000,
            csr: 0x0C00_0000,
            hse_on_inst: u64::MAX,
            pll_on_inst: u64::MAX,
            lsi_on_inst: u64::MAX,
            lse_on_inst: u64::MAX,
            pllcfgr: 0, cir: 0,
            ahb1rstr: 0, ahb2rstr: 0, ahb3rstr: 0,
            apb1rstr: 0, apb2rstr: 0,
            ahb2enr: 0, ahb3enr: 0,
            apb1enr: 0, apb2enr: 0,
            ahb1lpenr: 0, ahb2lpenr: 0, ahb3lpenr: 0,
            apb1lpenr: 0, apb2lpenr: 0,
            sscg: 0, plli2scfgr: 0, pllsai: 0,
            dckcfgr: 0, ckgatenr: 0, dckcfgr2: 0,
        }
    }
}

impl Rcc {
    pub fn new(name: &str) -> Option<Box<dyn Peripheral>> {
        if name == "RCC" { Some(Box::new(Self::default())) } else { None }
    }

    fn now(&self) -> u64 { instruction_count() }

    fn hse_rdy(&self) -> bool {
        self.cr & (1 << 16) != 0 && self.now().wrapping_sub(self.hse_on_inst) > 200
    }

    fn pll_rdy(&self) -> bool {
        self.cr & (1 << 24) != 0 && self.now().wrapping_sub(self.pll_on_inst) > 400
    }

    fn build_cr(&mut self) -> u32 {
        let mut cr = self.cr;
        cr |= 1 << 1; // HSIRDY always ready
        if self.hse_rdy() { cr |= 1 << 17; }
        if self.pll_rdy() { cr |= 1 << 25; }
        cr
    }

    fn cfgr_with_sws(&self) -> u32 {
        let sw = self.cfgr & 0x3;
        // SWS immediately mirrors SW (per reference implementation)
        (self.cfgr & !0xC) | (sw << 2)
    }

    pub fn system_clock_hz(&self) -> u64 {
        match (self.cfgr >> 2) & 0x3 {
            1 => HSE_FREQ,
            2 => self.pll_freq(),
            _ => HSI_FREQ,
        }
    }

    fn pll_freq(&self) -> u64 {
        let pllm = ((self.pllcfgr) & 0x3F).max(2) as u64;
        let plln = ((self.pllcfgr >> 6) & 0x1FF) as u64;
        let pllp = match (self.pllcfgr >> 16) & 0x3 {
            0 => 2, 1 => 4, 2 => 6, 3 => 8, _ => 2,
        };
        let src = match (self.pllcfgr >> 22) & 0x1 {
            0 => HSI_FREQ / 2,
            _ => HSE_FREQ,
        };
        (src / pllm * plln) / pllp
    }

    pub fn ahb_freq(&self) -> u64 {
        let hpre = (self.cfgr >> 4) & 0x0F;
        let div = match hpre {
            0..=7 => 1, 8 => 2, 9 => 4, 10 => 8, 11 => 16,
            12 => 64, 13 => 128, 14 => 256, 15 => 512,
            _ => 1,
        };
        self.system_clock_hz() / div
    }

    pub fn apb1_freq(&self) -> u64 {
        self.ahb_freq() / apb_div((self.cfgr >> 10) & 0x7)
    }

    pub fn apb2_freq(&self) -> u64 {
        self.ahb_freq() / apb_div((self.cfgr >> 13) & 0x7)
    }

    pub fn is_peripheral_enabled(&self, periph_base: u32) -> bool {
        let (reg, mask) = peripheral_enable_bit(periph_base);
        match reg {
            0x30 => self.ahb1enr & mask != 0,
            0x34 => self.ahb2enr & mask != 0,
            0x38 => self.ahb3enr & mask != 0,
            0x40 => self.apb1enr & mask != 0,
            0x44 => self.apb2enr & mask != 0,
            _ => true,
        }
    }
}

fn apb_div(ppre: u32) -> u64 {
    match ppre { 0..=3 => 1, 4 => 2, 5 => 4, 6 => 8, 7 => 16, _ => 1 }
}

fn peripheral_enable_bit(base: u32) -> (u32, u32) {
    match base {
        0x40020000..=0x40022FFF => (0x30, 1 << (base >> 10) & 0x1F),
        0x40026000 => (0x30, 1 << 12),
        0x40026400 => (0x30, 1 << 13),
        0x40023800 => (0x30, 1 << 14),
        0x40006000 => (0x30, 1 << 21),
        0x40020000..=0x40023FFF => (0x30, 1 << (base >> 10) & 0x1F),
        0x40000000..=0x40007FFF => (0x40, 1 << ((base >> 10) & 0x1F)),
        0x40010000..=0x4001FFFF => (0x44, 1 << ((base >> 10) & 0x1F)),
        _ => (0, 0),
    }
}

impl Peripheral for Rcc {
    fn read(&mut self, _sys: &System, offset: u32) -> u32 {
        match offset {
            0x00 => self.build_cr(),
            0x04 => self.pllcfgr,
            0x08 => self.cfgr_with_sws(),
            0x0C => self.cir,
            0x10 => self.ahb1rstr, 0x14 => self.ahb2rstr, 0x18 => self.ahb3rstr,
            0x20 => self.apb1rstr, 0x24 => self.apb2rstr,
            0x30 => self.ahb1enr, 0x34 => self.ahb2enr, 0x38 => self.ahb3enr,
            0x40 => self.apb1enr, 0x44 => self.apb2enr,
            0x50 => self.ahb1lpenr, 0x54 => self.ahb2lpenr, 0x58 => self.ahb3lpenr,
            0x60 => self.apb1lpenr, 0x64 => self.apb2lpenr,
            0x70 => self.bdcr,
            0x74 => self.csr,
            0x80 => self.sscg, 0x84 => self.plli2scfgr, 0x88 => self.pllsai,
            0x8C => self.dckcfgr, 0x90 => self.ckgatenr, 0x94 => self.dckcfgr2,
            _ => 0,
        }
    }

    fn write(&mut self, _sys: &System, offset: u32, value: u32) {
        match offset {
            0x00 => {
                let old_hseon = self.cr & (1 << 16);
                let new_hseon = value & (1 << 16);
                if old_hseon == 0 && new_hseon != 0 { self.hse_on_inst = instruction_count(); }
                if old_hseon != 0 && new_hseon == 0 { self.hse_on_inst = u64::MAX; }
                let old_pllon = self.cr & (1 << 24);
                let new_pllon = value & (1 << 24);
                if old_pllon == 0 && new_pllon != 0 { self.pll_on_inst = instruction_count(); }
                if old_pllon != 0 && new_pllon == 0 { self.pll_on_inst = u64::MAX; }
                self.cr = value;
            }
            0x04 => self.pllcfgr = value & 0x7F7F_FFFF,
            0x08 => self.cfgr = (value & 0xFD7F_FFFC) | (value & 0x3),
            0x0C => self.cir = value,
            0x10 => self.ahb1rstr = value, 0x14 => self.ahb2rstr = value, 0x18 => self.ahb3rstr = value,
            0x20 => self.apb1rstr = value, 0x24 => self.apb2rstr = value,
            0x30 => self.ahb1enr = value, 0x34 => self.ahb2enr = value, 0x38 => self.ahb3enr = value,
            0x40 => self.apb1enr = value, 0x44 => self.apb2enr = value,
            0x50 => self.ahb1lpenr = value, 0x54 => self.ahb2lpenr = value, 0x58 => self.ahb3lpenr = value,
            0x60 => self.apb1lpenr = value, 0x64 => self.apb2lpenr = value,
            0x70 => {
                self.bdcr = value & 0x0001_FF1F;
                if value & 1 != 0 { self.lse_on_inst = self.now(); }
                else { self.lse_on_inst = u64::MAX; }
                if self.lse_on_inst != u64::MAX && self.now().wrapping_sub(self.lse_on_inst) > 500 {
                    self.bdcr |= 1 << 1;
                } else { self.bdcr &= !(1 << 1); }
            }
            0x74 => {
                self.csr = value & 0x0C00_003F;
                if value & 1 != 0 { self.lsi_on_inst = self.now(); }
                else { self.lsi_on_inst = u64::MAX; }
                if self.lsi_on_inst != u64::MAX && self.now().wrapping_sub(self.lsi_on_inst) > 200 {
                    self.csr |= 1 << 1;
                } else { self.csr &= !(1 << 1); }
            }
            0x80 => self.sscg = value,
            0x84 => self.plli2scfgr = value,
            0x88 => self.pllsai = value,
            0x8C => self.dckcfgr = value & 0x001F_FF3F,
            0x90 => self.ckgatenr = value,
            0x94 => self.dckcfgr2 = value,
            _ => {}
        }
    }
}
