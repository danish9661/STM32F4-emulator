pub mod rcc;
pub mod spi;
pub mod usart;
pub mod systick;
pub mod gpio;
pub mod dma;
pub mod i2c;
pub mod nvic;
pub mod scb;
pub mod tim;
pub mod adc;
pub mod flash;
pub mod pwr;
pub mod wwdg;
pub mod iwdg;
pub mod rtc;
pub mod crc;
pub mod rng;
pub mod dac;
pub mod can;
pub mod sdio;
pub mod dcmi;
pub mod fsmc;
pub mod i2s;
pub mod sai;
pub mod sw_spi;
pub mod ltdc;
pub mod exti;
pub mod syscfg;
pub mod dbgmcu;
pub mod cryp;
pub mod hash;
pub mod eth;
pub mod qspi;
pub mod fpu;
pub mod mpu;
pub mod dwt;
pub mod itm;
pub mod stir;
pub mod usb;

use std::cell::RefCell;
use std::collections::HashMap;
use crate::system::System;
use crate::ext_devices::ExtDevices;
use fsmc::Fsmc;
use i2s::I2s;
use sai::Sai;
use sw_spi::{SoftwareSpi, SoftwareSpiConfig};
use ltdc::Ltdc;
use exti::Exti;
use syscfg::Syscfg;
use dbgmcu::Dbgmcu;
use cryp::Cryp;
use hash::Hash;
use eth::EthernetMac;
use qspi::Qspi;
use fpu::Fpu;
use mpu::Mpu;
use dwt::{Dwt, Demcr};
use itm::Itm;
use stir::Stir;
use usb::UsbFs;
use gpio::GpioPorts;
use svd_parser::svd::{MaybeArray, PeripheralInfo};

pub trait Peripheral: std::any::Any {
    fn read(&mut self, sys: &System, offset: u32) -> u32;
    fn write(&mut self, sys: &System, offset: u32, value: u32);
    fn tick(&mut self, _sys: &System) {}
    fn rx_byte(&mut self, _sys: &System, _byte: u8) {}
    /// JS driver called this after applying the queued FLASH erase to guest
    /// memory; default no-op, overridden by FLASH.
    fn flash_erase_applied(&mut self) {}
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

pub struct PeripheralSlot<T> {
    pub start: u32,
    pub end: u32,
    pub peripheral: T,
}

pub struct Peripherals {
pub(crate) peripherals: Vec<PeripheralSlot<RefCell<Box<dyn Peripheral>>>>,
    pub nvic: RefCell<nvic::Nvic>,
    pub gpio: RefCell<GpioPorts>,
    pub syscfg: RefCell<syscfg::Syscfg>,
}

pub const SYSCFG_BASE: u32 = 0x4001_3800;
pub const SYSCFG_END: u32 = 0x4001_3900;
pub const FLASH_REGS_BASE: u32 = 0x4002_3C00;

impl Peripherals {
    /// The JS driver applied the queued FLASH erase to guest memory; tell the
    /// FLASH model so it can clear BSY and let the firmware's busy-wait exit.
    pub fn flash_erase_applied(&self) {
        for slot in &self.peripherals {
            if slot.start == FLASH_REGS_BASE {
                slot.peripheral.borrow_mut().flash_erase_applied();
                break;
            }
        }
    }

    /// Mark the PWR peripheral as having woken from low-power (sets CSR WUF).
    /// Called by the emulator when the core resumes after a WFI/WFE halt.
    /// Run a closure on the USB OTG FS device model (host-side test API).
    fn with_usb<R>(&self, f: impl FnOnce(&mut UsbFs) -> R) -> Option<R> {
        for slot in &self.peripherals {
            if slot.start == 0x5000_0000 {
                use crate::peripherals::usb::UsbFs;
                if let Some(u) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<UsbFs>() {
                    return Some(f(u));
                }
                break;
            }
        }
        None
    }

    pub fn usb_reset(&self, sys: &System) {
        if self.with_usb(|u| u.host_reset(sys)).is_some() {
            sys.p.nvic.borrow_mut().clear_pending(crate::peripherals::usb::USB_IRQ);
        }
    }

    pub fn usb_enumerated(&self, sys: &System) {
        self.with_usb(|u| u.host_enumerated(sys));
    }

    pub fn usb_inject_setup(&self, sys: &System, data: &[u8]) {
        self.with_usb(|u| u.inject_setup(sys, data));
    }

    pub fn usb_inject_out(&self, sys: &System, ep: u32, data: &[u8]) {
        self.with_usb(|u| u.inject_out(sys, ep as usize, data));
    }

    pub fn usb_take_in(&self, ep: u32) -> Vec<u8> {
        self.with_usb(|u| u.take_in(ep as usize)).unwrap_or_default()
    }

    pub fn usb_in_status(&self, ep: u32) -> u32 {
        self.with_usb(|u| u.in_status(ep as usize)).unwrap_or(0)
    }

    /// DWT EXCCNT tick: one exception entry (called from every take path
    /// in the CPU). Gated on DEMCR.TRCENA like the rest of the unit.
    pub fn dwt_count_exc(&self, sys: &System) {
        if sys.p.read(sys, 0xE000EDFC, 4) & (1 << 24) == 0 {
            return;
        }
        for slot in &self.peripherals {
            if slot.start == 0xE000_1000 {
                use crate::peripherals::dwt::Dwt;
                if let Some(dwt) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Dwt>() {
                    dwt.count_exc();
                }
                break;
            }
        }
    }

    /// DWT FOLDCNT tick: one predicated-skipped instruction (called from
    /// the decoder's IT machinery). Same TRCENA gate.
    pub fn dwt_count_fold(&self, sys: &System) {
        if sys.p.read(sys, 0xE000EDFC, 4) & (1 << 24) == 0 {
            return;
        }
        for slot in &self.peripherals {
            if slot.start == 0xE000_1000 {
                use crate::peripherals::dwt::Dwt;
                if let Some(dwt) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Dwt>() {
                    dwt.count_fold();
                }
                break;
            }
        }
    }

    pub fn pwr_wakeup(&self) {
        for slot in &self.peripherals {
            if slot.start == 0x4000_7000 {
                use crate::peripherals::pwr::Pwr;
                if let Some(pwr) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Pwr>() {
                    pwr.wakeup();
                }
                break;
            }
        }
    }

    /// FPEXC.EN shadow for the VMRS/VMSR path (see cpu/thumb.rs). Effective
    /// FPU enable is CPACR-full && this bit; reset set (CPACR gates alone
    /// out of reset, matching every M4 startup sequence).
    pub fn fpu_fpexc_en(&self) -> bool {
        for slot in &self.peripherals {
            if slot.start == 0xE000_EF34 {
                if let Some(fpu) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Fpu>() {
                    return fpu.fpexc_en();
                }
                break;
            }
        }
        true
    }

    pub fn set_fpu_fpexc_en(&self, v: bool) {
        for slot in &self.peripherals {
            if slot.start == 0xE000_EF34 {
                if let Some(fpu) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Fpu>() {
                    fpu.set_fpexc_en(v);
                }
                break;
            }
        }
    }

    /// MPU access check for a CPU access range. Returns Some(true) on an
    /// execute violation, Some(false) on data, None when allowed (or when
    /// the MPU is disabled / has no slot — fail-open, the pre-MPU path).
    /// Fast gate first: one predictable-false branch when disabled.
    pub fn mpu_check(&self, addr: u32, size: u32, write: bool, exec: bool) -> Option<bool> {
        if !crate::system::is_mpu_enabled() {
            return None;
        }
        let priv_ = crate::system::current_privileged();
        let hfnmi = crate::system::current_hfnmi();
        // LDRT/STRT probe as-unprivileged even in handler mode (the decoder
        // holds the override for exactly one access).
        let priv_ = priv_ && !crate::system::mpu_force_unpriv();
        // The MPU slot is found by its fixed base, NOT by the access
        // address (which usually lives in RAM/FLASH, not in any slot).
        for slot in &self.peripherals {
            if slot.start == 0xE000_ED90 {
                if let Some(mpu) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Mpu>() {
                    return mpu.check_range(addr, size, write, exec, priv_, hfnmi);
                }
                break;
            }
        }
        // No MPU slot (minimal map): fail open.
        None
    }

    /// Device-memory query for the unaligned-Device rule (see Mpu): false
    /// with the MPU off, unmatched, or slotless (background Normal for
    /// the memory paths that consult this).
    pub fn mpu_is_device(&self, addr: u32) -> bool {
        if !crate::system::is_mpu_enabled() {
            return false;
        }
        for slot in &self.peripherals {
            if slot.start == 0xE000_ED90 {
                if let Some(mpu) = slot.peripheral.borrow_mut().as_any_mut().downcast_mut::<Mpu>() {
                    return mpu.is_device(addr);
                }
                break;
            }
        }
        false
    }
}

fn extract_svd_max_offset(p: &PeripheralInfo) -> u32 {
    let mut max_off = 0u32;

    use svd_parser::svd::register::{address_offsets as reg_offsets};
    use svd_parser::svd::array::{names as arr_names};
    use svd_parser::svd::cluster::{address_offsets as clus_offsets};

    for reg in p.registers() {
        match reg {
            MaybeArray::Single(r) => max_off = max_off.max(r.address_offset + 4),
            MaybeArray::Array(r, dim) => {
                for (off, _) in reg_offsets(r, dim).zip(arr_names(r, dim)) {
                    max_off = max_off.max(off + 4);
                }
            }
        }
    }

    for cluster in p.clusters() {
        match cluster {
            MaybeArray::Single(c) => {
                let base = c.address_offset;
                for reg in c.registers() {
                    match reg {
                        MaybeArray::Single(r) => max_off = max_off.max(base + r.address_offset + 4),
                        MaybeArray::Array(r, dim) => {
                            for (off, _) in reg_offsets(r, dim).zip(arr_names(r, dim)) {
                                max_off = max_off.max(base + off + 4);
                            }
                        }
                    }
                }
            }
            MaybeArray::Array(c, dim) => {
                for (clus_off, _) in clus_offsets(c, dim).zip(dim.indexes()) {
                    let base = c.address_offset + clus_off as u32;
                    for reg in c.registers() {
                        match reg {
                            MaybeArray::Single(r) => max_off = max_off.max(base + r.address_offset + 4),
                            MaybeArray::Array(r, d) => {
                                for (off, _) in reg_offsets(r, d).zip(arr_names(r, d)) {
                                    max_off = max_off.max(base + off + 4);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    max_off
}

impl Peripherals {
    pub const NVIC_REGS_BASE: u32 = 0xE000_E100;
    pub const NVIC_REGS_END: u32 = 0xE000_E500;

    pub const MEMORY_MAPS: [(u32, u32); 2] = [
        (0x4000_0000, 0xB000_0000),
        (0xE000_0000, 0xE100_0000),
    ];

    pub fn from_svd(svd_xml: &str, gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let mut device: svd_parser::svd::Device = svd_parser::parse(svd_xml)
            .expect("Failed to parse SVD XML");

        device.peripherals.sort_by_key(|p| p.base_address);

        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
            syscfg: RefCell::new(syscfg::Syscfg::default()),
        };

        let svd_map: HashMap<&str, &PeripheralInfo> = device.peripherals.iter()
            .filter_map(|p| match p {
                MaybeArray::Single(p) => Some((p.name.as_str(), p)),
                MaybeArray::Array(_, _) => None,
            })
            .collect();

        for p in &device.peripherals {
            let p = match p {
                MaybeArray::Single(p) => p,
                MaybeArray::Array(_, _) => continue,
            };

            let resolved = p.derived_from.as_ref()
                .and_then(|d| svd_map.get(d.as_str()).copied())
                .unwrap_or(p);

            let name = &p.name;
            let size = extract_svd_max_offset(resolved).max(0x10).min(0x400);
            let (start, end) = if name.as_str() == "FSMC" {
                (0x6000_0000, 0xA000_1000)
            } else if name.as_str() == "FPU" {
                // The SVD only describes FPCCR/FPCAR/FPDSCR (0x10 bytes) but
                // the MVFR0-2 ID regs follow at +0xC/+0x10/+0x14: claim them.
                (0xE000_EF34, 0xE000_EF34 + 0x18)
            } else if name.as_str() == "FPU_CPACR" {
                // Single register (CPACR at +0x0); the SVD-derived 0x10
                // would overlap the MPU slot at 0xE000ED90 (hard fault).
                (0xE000_ED88, 0xE000_ED8C)
            } else {
                (p.base_address as u32, p.base_address as u32 + size)
            };
            let peri: Option<Box<dyn Peripheral>> = None
                .or_else(|| nvic::NvicWrapper::new(name))
                .or_else(|| SysTick::new(name))
                .or_else(|| Scb::new(name))
                .or_else(|| Gpio::new(name))
                .or_else(|| Usart::new(name, ext_devices))
                .or_else(|| Rcc::new(name))
                .or_else(|| Flash::new(name))
                .or_else(|| Pwr::new(name))
                .or_else(|| Wwdg::new(name))
                .or_else(|| Iwdg::new(name))
                .or_else(|| Rtc::new(name))
                .or_else(|| Crc::new(name))
                .or_else(|| Rng::new(name))
                .or_else(|| Dac::new(name))
                .or_else(|| I2c::new(name, ext_devices))
                .or_else(|| Dma::new(name))
                .or_else(|| Spi::new(name, ext_devices))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Sdio::new(name))
                .or_else(|| Dcmi::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
                .or_else(|| I2s::new(name))
                .or_else(|| Sai::new(name))
                .or_else(|| Ltdc::new(name))
                .or_else(|| Exti::new(name))
                .or_else(|| Syscfg::new(name))
                .or_else(|| Cryp::new(name))
                .or_else(|| Hash::new(name))
                .or_else(|| Dbgmcu::new(name))
                .or_else(|| EthernetMac::new(name))
                .or_else(|| Qspi::new(name))
                .or_else(|| Mpu::new(name))
                .or_else(|| Fpu::new(name))
                .or_else(|| Dwt::new(name))
                .or_else(|| Demcr::new(name))
                .or_else(|| Stir::new(name))
            ;

            if let Some(peri) = peri {
                peripherals.peripherals.push(PeripheralSlot {
                    start, end,
                    peripheral: RefCell::new(peri),
                });
            }
        }

        // QUADSPI is absent from the F407 SVD; register it explicitly at its
        // conventional base so SVD-based init also models it.
        if let Some(p) = Qspi::new("QUADSPI") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0xA000_1000,
                end: 0xA000_1400,
                peripheral: RefCell::new(p),
            });
        }
        // Same for the DWT cycle counter + DEMCR TRCENA gate (both absent
        // from the SVD; tight slots — the map asserts on overlaps).
        if let Some(p) = Dwt::new("DWT") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0xE000_1000,
                end: 0xE000_1020,
                peripheral: RefCell::new(p),
            });
        }
        if let Some(p) = Demcr::new("DEMCR") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0xE000_EDFC,
                end: 0xE000_EE00,
                peripheral: RefCell::new(p),
            });
        }
        // ITM stimulus console (absent from the SVD too): one slot covering
        // STIM0-31 plus TER/TPR/TCR. No neighbor anywhere near it.
        if let Some(p) = Itm::new("ITM") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0xE000_0000,
                end: 0xE000_0F00,
                peripheral: RefCell::new(p),
            });
        }
        // STIR software trigger (absent from the SVD too): one word at
        // 0xE000EF00, clear of DEMCR (..EE00) and FPU (EF34..).
        if let Some(p) = Stir::new("STIR") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0xE000_EF00,
                end: 0xE000_EF04,
                peripheral: RefCell::new(p),
            });
        }
        // Core ARM system block: some SVDs (Keil DFP family files) omit
        // every system peripheral (no SCB/MPU/SysTick/FPU). Register whichever
        // bases came up empty, with the same windows new_wasm uses — without
        // this, VTOR reads 0 and every interrupt vectors into the void.
        // (SVDs that describe them, like monox, hit the skip rule instead.)
        peripherals.ensure_core_system_slots();
        // USB OTG FS device block (regs + EP0-3 FIFO strides to 0x50005000).
        // The four SVD OTG_FS_* entries stay dropped (UsbFs::new only
        // matches the combined name); HS is out of scope (stays dropped).
        if let Some(p) = UsbFs::new("USB_OTG_FS") {
            peripherals.peripherals.push(PeripheralSlot {
                start: 0x5000_0000,
                end: 0x5000_5000,
                peripheral: RefCell::new(p),
            });
        }
        // FSMC full-range fallback (new_wasm registers it unconditionally;
        // the SVD loop above only covers maps with an "FSMC" entry — Keil
        // calls it "FMC" (F429) or omits it (F401/F411), though the F429
        // silicon has the controller. Without this, BCR1 reads 0).
        if !peripherals.peripherals.iter().any(|p| p.start == 0x6000_0000) {
            if let Some(p) = Fsmc::new("FSMC", ext_devices) {
                peripherals.peripherals.push(PeripheralSlot {
                    start: 0x6000_0000,
                    end: 0xA000_1000,
                    peripheral: RefCell::new(p),
                });
            }
        }

        peripherals.wire_spi_flash_cs_callbacks();
        peripherals.finish_registration();
        peripherals
    }

    fn wire_spi_flash_cs_callbacks(&mut self) {
        // Wire SPI-flash CS pins to GPIO write callbacks so deassert edges
        // (which commit flash transactions) are observed immediately.
        let mut gpio = self.gpio.borrow_mut();
        for slot in &mut self.peripherals {
            let mut p = slot.peripheral.borrow_mut();
            let any = p.as_any_mut();
            if let Some(spi) = any.downcast_mut::<spi::Spi>() {
                spi.register_cs_callbacks(&mut *gpio);
            }
        }
        drop(gpio);
    }

    pub fn new_wasm(gpio: GpioPorts, ext_devices: &ExtDevices) -> Self {
        let mut peripherals = Peripherals {
            peripherals: Vec::new(),
            nvic: RefCell::new(nvic::Nvic::default()),
            gpio: RefCell::new(gpio),
            syscfg: RefCell::new(syscfg::Syscfg::default()),
        };

        // STM32F407 peripheral base addresses (sorted)
        let regs: Vec<(u32, &str)> = vec![
            (0x4000_0000, "TIM2"),  (0x4000_0400, "TIM3"),  (0x4000_0800, "TIM4"),
            (0x4000_0C00, "TIM5"),  (0x4000_1000, "TIM6"),  (0x4000_1400, "TIM7"),
            (0x4000_1800, "TIM12"), (0x4000_1C00, "TIM13"), (0x4000_2000, "TIM14"),
            (0x4000_2800, "RTC"),  (0x4000_2C00, "WWDG"),  (0x4000_3000, "IWDG"),
            (0x4000_3400, "I2S2ext"), (0x4000_3800, "SPI2"),  (0x4000_3C00, "SPI3"),
            (0x4000_4000, "I2S3ext"),
            (0x4000_4400, "USART2"), (0x4000_4800, "USART3"),
            (0x4000_4C00, "UART4"),  (0x4000_5000, "UART5"),
            (0x4000_5400, "I2C1"), (0x4000_5800, "I2C2"), (0x4000_5C00, "I2C3"),
            (0x4000_6000, "DMA1"),
            (0x4000_6400, "CAN1"), (0x4000_6800, "CAN2"),
            (0x4000_7000, "PWR"),  (0x4000_7400, "DAC"),  (0x4000_7800, "UART7"),
            (0x4000_7C00, "UART8"),
            (0x4001_1000, "USART1"), (0x4001_1400, "USART6"),
            (0x4001_2000, "ADC1"), (0x4001_2100, "ADC2"), (0x4001_2200, "ADC3"),
            (0x4001_2C00, "TIM8"),  (0x4001_3000, "SPI1"),
            (0x4001_3400, "SPI4"),  (0x4001_4000, "TIM9"),  (0x4001_4400, "TIM10"),
            (0x4001_4800, "TIM11"), (0x4001_5000, "SPI5"), (0x4001_5400, "SPI6"),
            (0x4001_5800, "SAI1"),  (0x4001_6800, "LTDC"),
            (0x4002_0000, "GPIOA"), (0x4002_0400, "GPIOB"), (0x4002_0800, "GPIOC"),
            (0x4002_0C00, "GPIOD"), (0x4002_1000, "GPIOE"), (0x4002_1400, "GPIOF"),
            (0x4002_1800, "GPIOG"), (0x4002_1C00, "GPIOH"), (0x4002_2000, "GPIOI"),
            (0x4002_3000, "CRC"),  (0x4002_3800, "RCC"),  (0x4002_3C00, "FLASH"),
            (0x4002_5800, "RNG"),  (0x4002_6400, "DMA2"),
            (0x4002_8000, "Ethernet_MAC"),
            (0x4002_8100, "Ethernet_MMC"),
            (0x4002_8700, "Ethernet_PTP"),
            (0x4002_9000, "Ethernet_DMA"),
            (0x5005_0000, "DCMI"),
            // QUADSPI is not in the F407 SVD (the F4 family lacks it), but we
            // model it anyway at its conventional base for completeness.
            (0xA000_1000, "QUADSPI"),
            // DWT cycle counter (not in the SVD either): explicit slot sized
            // to end before SysTick (the map asserts on overlaps).
            (0xE000_1000, "DWT"),
            (0xE000_E000, "NVIC"), (0xE000_E010, "SysTick"), (0xE000_ED00, "SCB"),
            (0xE000_ED90, "MPU"),
            // DEMCR lives in its own 4-byte slot: the SCB slots end before
            // EDFC and a wider claim would overlap MPU/FPU (assert-guarded).
            // STIR follows for the same reason (it sizes DEMCR to 4 bytes;
            // second explicit push below would double-cover it).
            (0xE000_EDFC, "DEMCR"),
            (0xE000_EF00, "STIR"),
            (0xE000_EF34, "FPU"),
            (0xE004_2000, "DBGMCU"),
        ];

        for (i, &(base, name)) in regs.iter().enumerate() {
            let size = regs.get(i + 1)
                .map(|&(next, _)| (next - base).min(0x400))
                .unwrap_or(0x100);
            let (start, end) = (base, base + size);

            let p: Option<Box<dyn Peripheral>> = None
                .or_else(|| nvic::NvicWrapper::new(name))
                .or_else(|| SysTick::new(name))
                .or_else(|| Scb::new(name))
                .or_else(|| Gpio::new(name))
                .or_else(|| Usart::new(name, ext_devices))
                .or_else(|| Rcc::new(name))
                .or_else(|| Flash::new(name))
                .or_else(|| Pwr::new(name))
                .or_else(|| Wwdg::new(name))
                .or_else(|| Iwdg::new(name))
                .or_else(|| Rtc::new(name))
                .or_else(|| Crc::new(name))
                .or_else(|| Rng::new(name))
                .or_else(|| Dac::new(name))
                .or_else(|| I2c::new(name, ext_devices))
                .or_else(|| Dma::new(name))
                .or_else(|| Spi::new(name, ext_devices))
                .or_else(|| Timer::new(name))
                .or_else(|| Adc::new(name))
                .or_else(|| Can::new(name))
                .or_else(|| Sdio::new(name))
                .or_else(|| Dcmi::new(name))
                .or_else(|| Fsmc::new(name, ext_devices))
                .or_else(|| I2s::new(name))
                .or_else(|| Sai::new(name))
                .or_else(|| Ltdc::new(name))
                .or_else(|| Exti::new(name))
                .or_else(|| Syscfg::new(name))
                .or_else(|| Cryp::new(name))
                .or_else(|| Hash::new(name))
                .or_else(|| Dbgmcu::new(name))
                .or_else(|| EthernetMac::new(name))
                .or_else(|| Qspi::new(name))
                .or_else(|| Mpu::new(name))
                .or_else(|| Fpu::new(name))
                .or_else(|| Dwt::new(name))
                .or_else(|| Demcr::new(name))
                .or_else(|| Stir::new(name))
            ;

            if let Some(p) = p {
                peripherals.peripherals.push(PeripheralSlot { start, end, peripheral: RefCell::new(p) });
            }
        }

        // FSMC has a huge range (0x6000_0000-0xA000_1000), register separately
        if let Some(p) = Fsmc::new("FSMC", ext_devices) {
            peripherals.peripherals.push(PeripheralSlot { start: 0x6000_0000, end: 0xA000_1000, peripheral: RefCell::new(p) });
        }

        // ITM stimulus console: STIM block + TER/TPR/TCR in one slot
        // (nothing else lives in 0xE0000000-0xE0000F00 on either map).
        // (STIR needs no explicit push here: the regs table above already
        // sizes it via the DEMCR->STIR->FPU adjacency.)
        if let Some(p) = Itm::new("ITM") {
            peripherals.peripherals.push(PeripheralSlot { start: 0xE000_0000, end: 0xE000_0F00, peripheral: RefCell::new(p) });
        }
        // USB OTG FS device block (regs + EP0-3 FIFO strides to 0x50005000;
        // no neighbor anywhere near it on either map).
        if let Some(p) = UsbFs::new("USB_OTG_FS") {
            peripherals.peripherals.push(PeripheralSlot { start: 0x5000_0000, end: 0x5000_5000, peripheral: RefCell::new(p) });
        }

        // Wire SPI-flash CS pins to GPIO write callbacks so deassert edges
        // (which commit flash transactions) are observed immediately.
        peripherals.wire_spi_flash_cs_callbacks();

        peripherals.finish_registration();
        peripherals
    }

    pub fn register_software_spi(&self, name: &str, cs: Option<String>, clk: &str, miso: &str, mosi: &str, ext_devices: &ExtDevices) {
        let config = SoftwareSpiConfig {
            name: name.to_string(),
            cs,
            clk: clk.to_string(),
            miso: miso.to_string(),
            mosi: mosi.to_string(),
        };
        SoftwareSpi::register(config, &mut self.gpio.borrow_mut(), ext_devices);
    }

    /// Register core ARM system slots (SysTick/SCB/MPU/FPU) whose bases
    /// came up empty — for SVDs that omit the system block entirely (Keil
    /// DFP family files). Skips any base an SVD entry already claims, so
    /// fully-described maps (monox) are untouched. Windows mirror new_wasm.
    fn ensure_core_system_slots(&mut self) {
        let want: [(u32, u32, &str); 4] = [
            (0xE000_E010, 0xE000_E020, "SysTick"),
            (0xE000_ED00, 0xE000_ED90, "SCB"),
            (0xE000_ED90, 0xE000_EDFC, "MPU"),
            (0xE000_EF34, 0xE000_EF50, "FPU"),
        ];
        for (start, end, name) in want {
            if self.peripherals.iter().any(|p| p.start == start) {
                continue;
            }
            let peri: Option<Box<dyn Peripheral>> = match name {
                "SysTick" => SysTick::new(name),
                "SCB" => Scb::new(name),
                "MPU" => Mpu::new(name),
                "FPU" => Fpu::new(name),
                _ => None,
            };
            if let Some(p) = peri {
                self.peripherals.push(PeripheralSlot {
                    start,
                    end,
                    peripheral: RefCell::new(p),
                });
            }
        }
    }

    fn finish_registration(&mut self) {
        self.peripherals.sort_by_key(|p| p.start);
        let a = self.peripherals.iter();
        let mut b = self.peripherals.iter();
        b.next();
        for (p1, p2) in a.zip(b) {
            assert!(p1.end <= p2.start, "Overlap: 0x{:08x}-0x{:08x} vs 0x{:08x}-0x{:08x}",
                p1.start, p1.end, p2.start, p2.end);
        }
    }

    fn get_peripheral<T>(slots: &[PeripheralSlot<T>], addr: u32) -> Option<&PeripheralSlot<T>> {
        let index = slots.binary_search_by_key(&addr, |p| p.start)
            .map_or_else(|e| e.checked_sub(1), |v| Some(v));
        index.map(|i| slots.get(i).filter(|p| addr <= p.end)).flatten()
    }

    fn bitbanding(addr: u32) -> Option<(u32, u8)> {
        if (0x4200_0000..0x4400_0000).contains(&addr) {
            let bit_number = (addr % 32) / 4;
            let mapped = 0x4000_0000 + (addr - 0x4200_0000) / 32;
            Some((mapped, bit_number as u8))
        } else { None }
    }

    fn is_register(addr: u32) -> bool { !(0x6000_0000..0xA000_0000).contains(&addr) }

    fn align_addr_4(addr: u32) -> (u32, u8) {
        let byte_offset = (addr % 4) as u8;
        (addr - byte_offset as u32, byte_offset)
    }

    pub fn read(&self, sys: &System, addr: u32, size: u8) -> u32 {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            return (self.read(sys, addr, 1) >> bit_number) & 1;
        }
        let is_reg = Self::is_register(addr);
        let (addr, byte_offset) = if is_reg {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        let value = if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().read(sys, addr - Self::NVIC_REGS_BASE)
        } else if SYSCFG_BASE <= addr && addr < SYSCFG_END {
            self.syscfg.borrow_mut().read(sys, addr - SYSCFG_BASE)
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().read(sys, addr - p.start)
        } else { 0 };
        if is_reg { value << (8 * byte_offset) } else { value }
    }

    pub fn write(&self, sys: &System, addr: u32, size: u8, mut value: u32) {
        if let Some((addr, bit_number)) = Self::bitbanding(addr) {
            let mut v = self.read(sys, addr, 1);
            v &= !(1 << bit_number);
            v |= (value & 1) << bit_number;
            return self.write(sys, addr, 1, v);
        }
        let (addr, byte_offset) = if Self::is_register(addr) {
            Self::align_addr_4(addr)
        } else { (addr, 0) };
        if byte_offset != 0 && Self::is_register(addr) {
            let v = self.read(sys, addr, 4);
            value = (value << 8 * byte_offset) | (v & (0xFFFF_FFFF >> (32 - 8 * byte_offset)));
        }
        if Self::NVIC_REGS_BASE <= addr && addr < Self::NVIC_REGS_END {
            self.nvic.borrow_mut().write(sys, addr - Self::NVIC_REGS_BASE, value);
        } else if SYSCFG_BASE <= addr && addr < SYSCFG_END {
            self.syscfg.borrow_mut().write(sys, addr - SYSCFG_BASE, value);
        } else if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().write(sys, addr - p.start, value);
        }
    }

    pub fn rx_byte(&self, sys: &System, addr: u32, byte: u8) -> bool {
        if let Some(p) = Self::get_peripheral(&self.peripherals, addr) {
            p.peripheral.borrow_mut().rx_byte(sys, byte);
            true
        } else { false }
    }

    pub fn addr_desc(&self, addr: u32) -> String {
        format!("addr=0x{:08x}", addr)
    }
}

use spi::Spi;
use usart::Usart;
use systick::SysTick;
use gpio::Gpio;
use dma::Dma;
use i2c::I2c;
use scb::Scb;
use tim::Timer;
use adc::Adc;
use flash::Flash;
use pwr::Pwr;
use wwdg::Wwdg;
use iwdg::Iwdg;
use rtc::Rtc;
use crc::Crc;
use rng::Rng;
use dac::Dac;
use rcc::Rcc;
use can::Can;
use sdio::Sdio;
use dcmi::Dcmi;
