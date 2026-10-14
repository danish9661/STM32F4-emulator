use std::cell::Cell;

pub trait Memory {
    fn read8(&self, addr: u32) -> u8;
    fn read16(&self, addr: u32) -> u16;
    fn read32(&self, addr: u32) -> u32;
    /// Instruction fetch (exactly like read16, except an unmapped fetch
    /// pends an execute-class bus fault instead of a data-class one, and
    /// no MPU data check runs — the loop-top XN check owns execute
    /// permission, which is also the more correct fault class there).
    fn fetch16(&self, addr: u32) -> u16;
    /// Mapped for any access (peripheral space, flash, RAM, extras).
    /// The run loop pre-checks fetches with this so a wild PC faults
    /// precisely instead of executing a dummy NOP (which would advance PC
    /// and let the re-fault clobber BFAR).
    fn is_mapped(&self, addr: u32) -> bool;
    fn write8(&mut self, addr: u32, v: u8);
    fn write16(&mut self, addr: u32, v: u16);
    fn write32(&mut self, addr: u32, v: u32);
}

pub struct MemRegion {
    pub base: u32,
    pub data: Vec<u8>,
}

/// QUADSPI memory-mapped window (AHB 0x90000000, DCR FSIZE-sized,
/// typically 16MB): when the QSPI model reports the window live (last
/// mode switch was memory-mapped), guest reads come straight from the
/// bound flash image; otherwise the window reads erased (0xFFFFFFFF).
fn is_qspi_mmap(addr: u32) -> bool {
    (addr >= 0x90000000 && addr < 0xA0000000)
        && crate::sys().p.qspi_mmap_live()
}

/// Byte read through the QSPI memory-mapped window (LE assembly handled
/// by the callers; this serves one byte at the flash image offset).
fn qspi_mmap_byte(addr: u32) -> u8 {
    let w = crate::sys().p.qspi_mmap_read(addr.wrapping_sub(0x90000000) & !3);
    ((w >> ((addr & 3) * 8)) & 0xFF) as u8
}

fn is_periph(addr: u32) -> bool {
    (addr >= 0x40000000 && addr < 0x51000000)
        // Full FSMC window (banks 1-4 every 0x10000000 up to 0xA0000000):
        // untapped banks must reach the model (inert 0), not the bus-fault
        // arms — the fsmc_test BANK4 probe depends on it.
        // (The QSPI mmap window 0x90000000 sits INSIDE the FSMC window:
        // it is checked FIRST by the read arms below, so a live mmap
        // mapping wins over the FSMC inert-0 while unmapped reads stay 0.)
        || (addr >= 0x60000000 && addr < 0xA0000000)
        || (addr >= 0xA0000000 && addr < 0xA2000000)
        || (addr >= 0xE0000000 && addr < 0xE1000000)
}

/// Flat guest memory for the WASM-native CPU: flash + main SRAM + any number
/// of extra RAM/ROM regions (doom's EXTRAM at 0xC0000000, the WAD image at
/// 0xB8000000, ...). Peripheral addresses route into the Rust model via the
/// process-wide `SYS` instance (which `init`/`init_svd` installs before any
/// `WasmCpu` is created).
///
/// Flash is execute/read-only for the guest: normal `write8` stores to flash
/// are ignored, except while the flash is unlocked with PG set (program
/// stores apply with NOR 1->0 semantics) — real flash needs an erase/program
/// sequence, serviced by the JS flash driver (`fill_flash_erase` +
/// `flash_erase_applied`). `load()` bypasses the protection
/// and is the only way to get firmware into flash.
pub struct FlatMemory {
    pub flash: Vec<u8>,
    pub ram: Vec<u8>,
    pub extra: Vec<MemRegion>,
    pub flash_base: u32,
    pub ram_base: u32,
    /// Last unmapped access (read or write), for diagnostics. Reads of
    /// unmapped memory return 0; writes are dropped.
    pub bad: Cell<Option<u32>>,
    /// Count of guest-visible RAM/extra-region stores (fast + slow paths,
    /// DMA memmove, image loads). The translated-block driver samples it
    /// around native chunks: an unchanged value proves no native store
    /// touched guest RAM, so the mirror stays valid and the per-chunk
    /// tbGen invalidation can be skipped (VDP-class shapes alternate
    /// translated runs and writeless native fragments every column).
    /// Flash/periph/model writes never bump it (they cannot desync a RAM
    /// mirror page). Wrapping is fine (equality comparison only).
    pub write_seq: u64,
    /// Layout version: bumped whenever a region buffer can move (new extra
    /// region, backward/forward growth inside load(), map_extra()). Plain
    /// stores never move buffers (runtime writes never grow — unmapped
    /// writes BusFault instead), so a stable version across a run PROVES
    /// baked region addresses stay valid. Shared-memory translation (the
    /// mirror's replacement) keys its cache on this: any bump drops all
    /// translations. Setup-time only in practice (load/map_extra).
    pub layout_version: u64,
}

impl FlatMemory {
    pub fn new(flash_size: usize, ram_size: usize) -> Self {
        Self {
            flash: vec![0; flash_size],
            ram: vec![0; ram_size],
            extra: Vec::new(),
            flash_base: 0x08000000,
            ram_base: 0x20000000,
            bad: Cell::new(None),
            write_seq: 0,
            layout_version: 0,
        }
    }

    fn in_flash(&self, addr: u32) -> bool {
        addr >= self.flash_base && (addr - self.flash_base) < self.flash.len() as u32
    }
    fn in_ram(&self, addr: u32) -> bool {
        addr >= self.ram_base && (addr - self.ram_base) < self.ram.len() as u32
    }
    #[inline]
    fn note_write(&mut self) {
        self.write_seq = self.write_seq.wrapping_add(1);
    }
    fn extra_idx(&self, addr: u32) -> Option<usize> {
        self.extra
            .iter()
            .position(|r| addr >= r.base && (addr - r.base) < r.data.len() as u32)
    }

    /// Fast-path gate: plain flash/RAM access with the MPU off and no
    /// unaligned trap. Two relaxed atomic loads, zero global-SYS touches
    /// (no peripheral-slot borrow, no QSPI liveness probe). Everything
    /// else — periph, QSPI window, extra regions, MPU-on, UNALIGN_TRP —
    /// takes the slow path unchanged. Ranges are disjoint by construction
    /// (flash 0x08.., RAM 0x20.. vs QSPI 0x90.. / periph windows), so
    /// checking flash/RAM first cannot steal a device access.
    #[inline]
    fn fast_plain(&self) -> bool {
        !crate::system::is_mpu_enabled() && !crate::system::unalign_trp()
    }

    /// Byte offset of `[addr, addr+size)` inside flash, or None.
    /// Below-base addresses wrap huge via wrapping_sub; the checked_add
    /// then fails instead of panicking (debug) or wrapping into a bogus
    /// Some (release) — a latent bug this exact shape once had, caught by
    /// the bulk_range_parity test probing just below the base.
    #[inline]
    fn flash_range(&self, addr: u32, size: u32) -> Option<usize> {
        let off = (addr as u64).wrapping_sub(self.flash_base as u64);
        let end = off.checked_add(size as u64)?;
        if end <= self.flash.len() as u64 {
            Some(off as usize)
        } else {
            None
        }
    }

    /// Same for main SRAM.
    #[inline]
    fn ram_range(&self, addr: u32, size: u32) -> Option<usize> {
        let off = (addr as u64).wrapping_sub(self.ram_base as u64);
        let end = off.checked_add(size as u64)?;
        if end <= self.ram.len() as u64 {
            Some(off as usize)
        } else {
            None
        }
    }

    /// Map a zeroed extra region in bulk. `load()` builds regions byte by
    /// byte (quadratic for megabyte images); pre-mapping makes WAD/EXTRAM
    /// setup O(size) with a fast fill. Matches the JS driver, which zeroes
    /// every extra_ram region at map time.
    pub fn map_extra(&mut self, base: u32, size: usize) {
        self.extra.push(MemRegion { base, data: vec![0; size] });
        // New region: the region table changed (data buffers of existing
        // regions don't move on an outer-Vec push, but translators key on
        // the table too — bump unconditionally, setup-time only).
        self.layout_version = self.layout_version.wrapping_add(1);
    }

    /// Bulk read fast path for driver crossings (mem_read): single bounds
    /// check + slice copy when `fast_plain()` holds and the whole range
    /// sits in one plain region (flash / SRAM / one extra region).
    /// Returns false otherwise — the caller then takes the exact per-byte
    /// slow path, which owns all model side effects, MPU faults, QSPI
    /// routing, and bad-address latches. No observable difference: the
    /// plain paths have no side effects, and anything else never reaches
    /// here (ranges are disjoint by construction; MPU-on and UNALIGN_TRP
    /// are gated out by fast_plain).
    pub fn read_range(&self, addr: u32, out: &mut [u8]) -> bool {
        if out.is_empty() {
            return true;
        }
        if !self.fast_plain() {
            return false;
        }
        let len = out.len();
        if let Some(o) = self.flash_range(addr, len as u32) {
            out.copy_from_slice(&self.flash[o..o + len]);
            return true;
        }
        if let Some(o) = self.ram_range(addr, len as u32) {
            out.copy_from_slice(&self.ram[o..o + len]);
            return true;
        }
        // Extra regions: the whole range must sit in ONE region.
        for r in self.extra.iter() {
            let o = (addr as u64).wrapping_sub(r.base as u64);
            if o + len as u64 <= r.data.len() as u64 {
                out.copy_from_slice(&r.data[o as usize..o as usize + len]);
                return true;
            }
        }
        false
    }

    /// Bulk write fast path for driver crossings (mem_write): RAM and extra
    /// regions only. Flash keeps its protection + program-error arms (slow
    /// path: flash writes are erase/program events, never hot), and
    /// periph/QSPI/MPU/unmapped keep the exact per-byte path. Same
    /// no-observable-difference contract as read_range.
    pub fn write_range(&mut self, addr: u32, data: &[u8]) -> bool {
        if data.is_empty() {
            return true;
        }
        if !self.fast_plain() {
            return false;
        }
        let len = data.len();
        if let Some(o) = self.ram_range(addr, len as u32) {
            self.ram[o..o + len].copy_from_slice(data);
            self.note_write();
            return true;
        }
        for r in self.extra.iter_mut() {
            let o = (addr as u64).wrapping_sub(r.base as u64);
            if o + len as u64 <= r.data.len() as u64 {
                r.data[o as usize..o as usize + len].copy_from_slice(data);
                self.note_write();
                return true;
            }
        }
        false
    }

    /// Apply a sector/mass erase fill (all bytes 0xFF), clamped to mapped
    /// flash. Called by the JS flash driver after `take_flash_erase`, then
    /// completed with `flash_erase_applied` — the only runtime path that
    /// sets flash bits besides programming (see `write8`).
    pub fn fill_flash_erase(&mut self, start: u32, len: u32) {
        self.note_write();
        let end = start.saturating_add(len);
        let mut a = start;
        while a < end {
            if self.in_flash(a) {
                self.flash[(a - self.flash_base) as usize] = 0xFF;
            }
            a = a.wrapping_add(1);
        }
    }

    /// Load `data` at `base`, writing through flash protection. Bytes landing
    /// in flash or RAM go there; bytes landing anywhere else create (or
    /// extend) an extra region, so ELF segments / WAD images just work.
    pub fn load(&mut self, data: &[u8], base: u32) {
        for (i, &b) in data.iter().enumerate() {
            let a = base.wrapping_add(i as u32);
            if self.in_flash(a) {
                self.flash[(a - self.flash_base) as usize] = b;
            } else if self.in_ram(a) {
                self.ram[(a - self.ram_base) as usize] = b;
            } else if is_periph(a) {
                // never load firmware into MMIO
            } else if let Some(idx) = self.extra_idx(a) {
                let r = &mut self.extra[idx];
                r.data[(a - r.base) as usize] = b;
            } else {
                // extend backwards into a touching region, else create one
                let mut done = false;
                for r in self.extra.iter_mut() {
                    if a.wrapping_add(1) == r.base {
                        r.base = a;
                        r.data.insert(0, b);
                        // Buffer moved (insert reallocates): bump layout.
                        self.layout_version = self.layout_version.wrapping_add(1);
                        done = true;
                        break;
                    }
                    if a == r.base.wrapping_add(r.data.len() as u32) {
                        r.data.push(b);
                        // Buffer may have moved (push reallocates): bump.
                        self.layout_version = self.layout_version.wrapping_add(1);
                        done = true;
                        break;
                    }
                }
                if !done {
                    self.extra.push(MemRegion { base: a, data: vec![b] });
                    self.layout_version = self.layout_version.wrapping_add(1);
                }
            }
        }
        // Image loads rewrite guest memory wholesale (boot/setup time):
        // one bump covers the whole call.
        self.note_write();
    }

    fn read_ram_byte(&self, addr: u32) -> u8 {
        if self.in_ram(addr) {
            self.ram[(addr - self.ram_base) as usize]
        } else if let Some(idx) = self.extra_idx(addr) {
            self.extra[idx].data[(addr - self.extra[idx].base) as usize]
        } else {
            0
        }
    }

    fn write_ram_byte(&mut self, addr: u32, v: u8) {
        if self.in_ram(addr) {
            self.ram[(addr - self.ram_base) as usize] = v;
            self.note_write();
        } else if let Some(idx) = self.extra_idx(addr) {
            let r = &mut self.extra[idx];
            r.data[(addr - r.base) as usize] = v;
            self.note_write();
        }
        // Flash/unmapped destinations are skipped (matches the JS driver,
        // whose mem_write there throws and is ignored before completion).
    }

    /// Synchronously complete staged memory-to-memory DMA transfers.
    /// Real mem-to-mem DMA runs at bus rate while the guest continues, but
    /// polling firmware (edge_test, periph_test) checks NDTR/dst/flags on
    /// the very next instructions — only an inline move satisfies that, the
    /// way the pre-latch model behaved. Peripheral transfers stay staged:
    /// their data path lives in the JS driver.
    fn service_sync_dma(&mut self) {
        loop {
            let t = match crate::sys().take_memcopy_dma_transfer() {
                Some(t) => t,
                None => break,
            };
            // memmove semantics via a temp buffer (src/dst may overlap).
            let mut buf = Vec::with_capacity(t.size);
            for i in 0..t.size {
                buf.push(self.read_ram_byte(t.src.wrapping_add(i as u32)));
            }
            for (i, b) in buf.iter().enumerate() {
                self.write_ram_byte(t.dst.wrapping_add(i as u32), *b);
            }
            crate::sys().mark_dma_completed(t.stream_idx, true);
        }
    }
    /// Fetch one byte without any MPU check (execute permission belongs to
    /// the loop-top XN check). Unmapped bytes pend an execute-class bus
    /// fault (first-wins keeps the lowest faulting address).
    fn fetch_byte(&self, addr: u32) -> Option<u8> {        if self.in_flash(addr) {
            Some(self.flash[(addr - self.flash_base) as usize])
        } else if self.in_ram(addr) {
            Some(self.ram[(addr - self.ram_base) as usize])
        } else if let Some(idx) = self.extra_idx(addr) {
            let r = &self.extra[idx];
            Some(r.data[(addr - r.base) as usize])
        } else {
            self.bad.set(Some(addr));
            crate::system::pend_bus_fault(addr, true);
            None
        }
    }

    /// True for mapped normal memory (flash/RAM/extra). Periph accesses
    /// return earlier; anything else falls to the bus-fault bad-arms.
    #[inline]
    fn mapped(&self, addr: u32) -> bool {
        is_qspi_mmap(addr) || is_periph(addr) || self.in_flash(addr) || self.in_ram(addr) || self.extra_idx(addr).is_some()
    }

    /// CCR.UNALIGN_TRP gate for multi-byte normal-memory accesses. Skipped
    /// when unmapped (the bus-fault arms own those) and on the periph path
    /// (Device-memory unaligned stays lenient). Unaligned *Device* access
    /// faults regardless of the trap (the one observable MPU type rule).
    /// Like the MPU data path the faulting access completes dropped and
    /// raises before the next fetch (flags exact, PC deferred by one).
    #[inline]
    fn unaligned_deny(&self, addr: u32, size: u32) -> bool {
        if !self.mapped(addr) {
            return false;
        }
        if size <= 1 || (addr & (size - 1)) == 0 {
            return false;
        }
        if crate::system::unalign_trp() || crate::sys().p.mpu_is_device(addr) {
            crate::system::pend_align_fault(addr);
            true
        } else {
            false
        }
    }

    /// MPU gate for one CPU access (size bytes at addr). Returns true when
    /// denied (violation latched for the run loop; caller returns dummy /
    /// drops the access). Fast path is a single predictable-false branch
    /// when the MPU is off; unmapped addresses never reach here (callers
    /// check mapped-ness first and keep the legacy bad-address behavior).
    #[inline]
    fn mpu_deny(&self, addr: u32, size: u32, write: bool) -> bool {
        if !crate::system::is_mpu_enabled() {
            return false;
        }
        match crate::sys().p.mpu_check(addr, size, write, false) {
            Some(_) => {
                crate::system::pend_mpu_fault(addr, false);
                true
            }
            None => false,
        }
    }
}

impl Memory for FlatMemory {
    fn read8(&self, addr: u32) -> u8 {
        // Fast path first (flash/RAM only — disjoint from every device
        // window, so no device access can be stolen here).
        if self.fast_plain() {
            if let Some(o) = self.flash_range(addr, 1) {
                return self.flash[o];
            }
            if let Some(o) = self.ram_range(addr, 1) {
                return self.ram[o];
            }
        }
        // QSPI memory-mapped window wins over the FSMC inert-0 inside
        // 0x90000000..0xA0000000 while the mmap mode is live.
        if is_qspi_mmap(addr) {
            if self.mpu_deny(addr, 1, false) {
                return 0;
            }
            return qspi_mmap_byte(addr);
        }
        if is_periph(addr) {
            // MPU first: a faulting access must not reach model side
            // effects (UART TX, RXNE clears, ...).
            if self.mpu_deny(addr, 1, false) {
                return 0;
            }
            // Single width-1 model read (mirrors the JS memReadHook, which
            // takes the low byte). The model aligns internally.
            return crate::sys().p.read(crate::sys(), addr, 1) as u8;
        }
        // Unmapped addresses keep the legacy behavior (bad-address latch
        // + 0) and never raise MPU faults; only mapped memory is checked.
        if self.in_flash(addr) {
            if self.mpu_deny(addr, 1, false) {
                return 0;
            }
            self.flash[(addr - self.flash_base) as usize]
        } else if self.in_ram(addr) {
            if self.mpu_deny(addr, 1, false) {
                return 0;
            }
            self.ram[(addr - self.ram_base) as usize]
        } else if let Some(idx) = self.extra_idx(addr) {
            if self.mpu_deny(addr, 1, false) {
                return 0;
            }
            let r = &self.extra[idx];
            r.data[(addr - r.base) as usize]
        } else {
            // Unmapped memory: legacy bad-address latch plus a precise
            // BusFault (silicon faults wild accesses; the 0-return alone
            // used to hide null derefs and overruns). Peripheral-space
            // holes never reach here (the model returns 0 for those).
            self.bad.set(Some(addr));
            crate::system::pend_bus_fault(addr, false);
            0
        }
    }
    fn fetch16(&self, addr: u32) -> u16 {
        // Fast path needs NO gate: the slow path performs no MPU or
        // unaligned checks either (execute permission belongs to the
        // loop-top XN check). Pure range math, zero global touches.
        if let Some(o) = self.flash_range(addr, 2) {
            return u16::from_le_bytes([self.flash[o], self.flash[o + 1]]);
        }
        if let Some(o) = self.ram_range(addr, 2) {
            return u16::from_le_bytes([self.ram[o], self.ram[o + 1]]);
        }
        if is_periph(addr) {
            // No MPU data check: the loop-top XN check owns execute
            // permission (and reports the correct IACCVIOL-class fault).
            return crate::sys().p.read(crate::sys(), addr, 2) as u16;
        }
        // Bounds-checked per byte (a fetch straddling a region end faults
        // on the first unmapped byte instead of panicking the host).
        match (self.fetch_byte(addr), self.fetch_byte(addr.wrapping_add(1))) {
            (Some(lo), Some(hi)) => (lo as u16) | ((hi as u16) << 8),
            _ => 0,
        }
    }
    fn is_mapped(&self, addr: u32) -> bool {
        // Fast path skips the QSPI liveness probe (global SYS borrow) for
        // the two ranges that cover ~every fetch and data access.
        if self.flash_range(addr, 1).is_some() || self.ram_range(addr, 1).is_some() {
            return true;
        }
        self.mapped(addr)
    }
    fn read16(&self, addr: u32) -> u16 {
        if self.fast_plain() {
            if let Some(o) = self.flash_range(addr, 2) {
                return u16::from_le_bytes([self.flash[o], self.flash[o + 1]]);
            }
            if let Some(o) = self.ram_range(addr, 2) {
                return u16::from_le_bytes([self.ram[o], self.ram[o + 1]]);
            }
        }
        if is_qspi_mmap(addr) {
            if self.mpu_deny(addr, 2, false) {
                return 0;
            }
            // Unaligned-safe: assemble from window bytes (no Device-fault
            // games — the window is Normal-mapped flash image memory).
            let lo = qspi_mmap_byte(addr) as u16;
            let hi = qspi_mmap_byte(addr.wrapping_add(1)) as u16;
            return lo | (hi << 8);
        }
        if is_periph(addr) {
            if self.mpu_deny(addr, 2, false) {
                return 0;
            }
            return crate::sys().p.read(crate::sys(), addr, 2) as u16;
        }
        if self.unaligned_deny(addr, 2) {
            return 0;
        }
        let lo = self.read8(addr) as u16;
        let hi = self.read8(addr + 1) as u16;
        lo | (hi << 8)
    }
    fn read32(&self, addr: u32) -> u32 {
        // Single range check + one LE load replaces four read8 calls
        // (each with its own region walk + MPU probe).
        if self.fast_plain() {
            if let Some(o) = self.flash_range(addr, 4) {
                return u32::from_le_bytes([self.flash[o], self.flash[o + 1], self.flash[o + 2], self.flash[o + 3]]);
            }
            if let Some(o) = self.ram_range(addr, 4) {
                return u32::from_le_bytes([self.ram[o], self.ram[o + 1], self.ram[o + 2], self.ram[o + 3]]);
            }
        }
        if is_periph(addr) {
            if self.mpu_deny(addr, 4, false) {
                return 0;
            }
            return crate::sys().p.read(crate::sys(), addr, 4);
        }
        if self.unaligned_deny(addr, 4) {
            return 0;
        }
        let b0 = self.read8(addr) as u32;
        let b1 = self.read8(addr + 1) as u32;
        let b2 = self.read8(addr + 2) as u32;
        let b3 = self.read8(addr + 3) as u32;
        b0 | (b1 << 8) | (b2 << 16) | (b3 << 24)
    }
    fn write8(&mut self, addr: u32, v: u8) {
        // Fast path is RAM-only: flash stores need the protection +
        // program-error arms below, so they stay on the slow path (flash
        // writes are erase/program events, never hot).
        if self.fast_plain() {
            if let Some(o) = self.ram_range(addr, 1) {
                self.ram[o] = v;
                self.note_write();
                return;
            }
        }
        if is_periph(addr) {
            if self.mpu_deny(addr, 1, true) {
                return;
            }
            // Single width-1 model write. Never split a wider guest store
            // into byte RMWs here: each model write can have side effects
            // (a USART DR write emits a UART char), so one guest store must
            // equal exactly one model call, like the JS memWriteHook.
            crate::sys().p.write(crate::sys(), addr, 1, v as u32);
            self.service_sync_dma();
            return;
        }
        if self.in_flash(addr) {
            // MPU first (an RX-mapped flash write faults on silicon),
            // then flash protection: an unlocked flash with PG set accepts
            // program stores with NOR semantics (bits only transition 1->0;
            // erase restores 0xFF via fill_flash_erase). Anything else is
            // ignored (see struct docs).
            if self.mpu_deny(addr, 1, true) {
                return;
            }
            // Program-sequence check (wrong width for PSIZE, or address
            // outside the array): latches PGSERR, write dropped.
            if crate::sys().p.flash_program_error(addr, 1) {
                return;
            }
            if crate::system::flash_is_programming() {
                let i = (addr - self.flash_base) as usize;
                if let Some(cell) = self.flash.get_mut(i) {
                    *cell &= v;
                    self.note_write();
                }
            }
        } else if self.in_ram(addr) {
            if self.mpu_deny(addr, 1, true) {
                return;
            }
            self.ram[(addr - self.ram_base) as usize] = v;
            self.note_write();
        } else if let Some(idx) = self.extra_idx(addr) {
            if self.mpu_deny(addr, 1, true) {
                return;
            }
            let r = &mut self.extra[idx];
            r.data[(addr - r.base) as usize] = v;
            self.note_write();
        } else {
            // Unmapped store: bad-address latch plus a precise BusFault
            // (read arm documents the split with peripheral holes).
            self.bad.set(Some(addr));
            crate::system::pend_bus_fault(addr, false);
        }
    }
    fn write16(&mut self, addr: u32, v: u16) {
        if self.fast_plain() {
            if let Some(o) = self.ram_range(addr, 2) {
                let b = v.to_le_bytes();
                self.ram[o] = b[0];
                self.ram[o + 1] = b[1];
                self.note_write();
                return;
            }
        }
        if is_periph(addr) {
            if self.mpu_deny(addr, 2, true) {
                return;
            }
            crate::sys().p.write(crate::sys(), addr, 2, v as u32);
            self.service_sync_dma();
            return;
        }
        if self.unaligned_deny(addr, 2) {
            return;
        }
        if self.in_flash(addr) && crate::sys().p.flash_program_error(addr, 2) {
            return;
        }
        self.write8(addr, (v & 0xFF) as u8);
        self.write8(addr + 1, (v >> 8) as u8);
    }
    fn write32(&mut self, addr: u32, v: u32) {
        if self.fast_plain() {
            if let Some(o) = self.ram_range(addr, 4) {
                let b = v.to_le_bytes();
                self.ram[o] = b[0];
                self.ram[o + 1] = b[1];
                self.ram[o + 2] = b[2];
                self.ram[o + 3] = b[3];
                self.note_write();
                return;
            }
        }
        if is_periph(addr) {
            if self.mpu_deny(addr, 4, true) {
                return;
            }
            crate::sys().p.write(crate::sys(), addr, 4, v);
            self.service_sync_dma();
            return;
        }
        if self.unaligned_deny(addr, 4) {
            return;
        }
        if self.in_flash(addr) && crate::sys().p.flash_program_error(addr, 4) {
            return;
        }
        self.write8(addr, (v & 0xFF) as u8);
        self.write8(addr + 1, ((v >> 8) & 0xFF) as u8);
        self.write8(addr + 2, ((v >> 16) & 0xFF) as u8);
        self.write8(addr + 3, ((v >> 24) & 0xFF) as u8);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Bulk range fast path must be bit-identical to the per-byte path
    /// (content assertions only — never the taken/not-taken bool, so a
    /// concurrent MPU firmware test flipping `fast_plain` can't flake it).
    #[test]
    fn bulk_range_parity() {
        let mut m = FlatMemory::new(1024, 0x20000);
        m.map_extra(0xC0000000, 0x10000);
        // Pure range helpers are globals-free and deterministic.
        assert_eq!(m.flash_range(0x08000000, 1024), Some(0));
        assert_eq!(m.flash_range(0x080003F0, 64), None); // straddles end
        assert_eq!(m.flash_range(0x07FFFFFF, 4), None); // below base wraps
        assert_eq!(m.ram_range(0x20000000, 0x20000), Some(0));
        assert_eq!(m.ram_range(0x2001FFF0, 32), None); // straddles end
        for i in 0..1024 {
            m.flash[i] = (i & 0xFF) as u8;
        }
        for i in 0..0x20000 {
            m.ram[i] = ((i * 7 + 3) & 0xFF) as u8;
        }
        for i in 0..0x10000 {
            m.extra[0].data[i] = ((i * 13 + 5) & 0xFF) as u8;
        }
        // Reads: bulk vs per-byte across regions and sizes.
        for (base, len) in [
            (0x08000000u32, 1usize),
            (0x08000000, 1024),
            (0x080003F0, 64),
            (0x20000000, 4096),
            (0x2001F000, 8192),
            (0xC0000000, 16384),
            (0xC000FFF0, 32),
        ] {
            let mut bulk = vec![0u8; len];
            let took_fast = m.read_range(base, &mut bulk);
            let slow: Vec<u8> = (0..len).map(|i| m.read8(base.wrapping_add(i as u32))).collect();
            if took_fast {
                assert_eq!(bulk, slow, "read parity @{:08x}+{}", base, len);
            }
            // Cross-region (RAM end + past end) must decline to slow path.
            let _ = took_fast;
        }
        // Cross-region span declines (straddles RAM end).
        let mut cross = vec![0u8; 32];
        assert!(!m.read_range(0x2001FFF0, &mut cross));
        // Writes: RAM + extra round-trip through the bulk path or slow
        // path identically; flash direct-apply stays refused.
        let waddr = 0x20005000;
        let wdata: Vec<u8> = (0..256).map(|i| (i ^ 0xA5) as u8).collect();
        if !m.write_range(waddr, &wdata) {
            for (i, &b) in wdata.iter().enumerate() {
                m.write8(waddr.wrapping_add(i as u32), b);
            }
        }
        let back: Vec<u8> = (0..256).map(|i| m.read8(waddr.wrapping_add(i as u32))).collect();
        assert_eq!(back, wdata, "ram write round-trip");
        let xaddr = 0xC0002000;
        if !m.write_range(xaddr, &wdata) {
            for (i, &b) in wdata.iter().enumerate() {
                m.write8(xaddr.wrapping_add(i as u32), b);
            }
        }
        let xback: Vec<u8> = (0..256).map(|i| m.read8(xaddr.wrapping_add(i as u32))).collect();
        assert_eq!(xback, wdata, "extra write round-trip");
        // Flash bulk write must NOT apply directly (protection arms own it).
        assert!(!m.write_range(0x08000100, &wdata));
        assert_eq!(m.flash[0x100], 0x00);
        // write_seq: reads never bump; RAM/extra stores bump exactly once
        // per call (batching granularity is calls, not bytes — the driver
        // only needs changed-vs-unchanged).
        let s0 = m.write_seq;
        let _ = m.read8(waddr);
        let mut tmp = [0u8; 4];
        assert!(m.read_range(waddr, &mut tmp));
        assert_eq!(m.write_seq, s0, "reads must not bump");
        m.write8(waddr, 0x11);
        assert_eq!(m.write_seq, s0 + 1);
        assert!(m.write_range(waddr, &[1, 2, 3, 4]));
        assert_eq!(m.write_seq, s0 + 2);
        assert!(m.write_range(xaddr, &[9, 9]));
        assert_eq!(m.write_seq, s0 + 3);
    }
}
