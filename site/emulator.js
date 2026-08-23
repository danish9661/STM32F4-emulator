// Universal STM32F407 emulator factory — runs in Node and the browser.
// No imports: the caller supplies the bindings (wasm-bindgen module),
// the Unicorn factory (MUnicorn), the SVD XML, and the firmware bytes.
//
// Default eth_* constants match eth_http/eth_http.elf (nm-verified):
//   irq_flag 0x20000620, rx_frame_idx 0x20000628, rx_frame_len 0x2000062c,
//   tx_desc 0x20000610, rx_desc 0x20000630, tx_pkt 0x20000008, rx_buf 0x20000660

export async function createEmulator(opts) {
    const {
        firmware,
        bindings,             // wasm-bindgen module (web or nodejs build)
        unicorn,              // async factory -> Unicorn module
        svdXml,               // SVD XML string for init_svd
        wasmInit,             // optional: wasm bytes for bindings.default() (Node)
        flash_size = 0x100000,
        ram_size = 0x20000,
        vector_table = 0x08000000,
        tickEvery = 5000,     // tick_n batching cadence
        pollEvery = 1000,     // DMA/TX-poll check cadence
        // Skip the per-instruction poll/tick wasm calls entirely (no ETH/DMA/
        // timers/watchdog needed — the doom firmware).  InstCount++ alone keeps
        // the instruction accounting; the guest's own clock paces the loop.
        minimalPolls = false,
        // doom fast path: HOOK_BLOCK instead of HOOK_CODE (size/2 inst per
        // block). Only valid with minimalPolls (no tick/poll logic to run).
        blockCounting = false,
        // Fastest path: register NO counting hook at all. Every basic block
        // otherwise crosses the WASM->JS boundary just to bump a counter,
        // which is pure overhead for a firmware (like doom) that paces off
        // its own frame counter rather than instCount. `instCount` then
        // tracks the emu_start budget issued per step() — which is actually
        // MORE accurate than block counting (that over-reports ~1.39x, see
        // AGENTS §16). Requires minimalPolls; overrides blockCounting.
        noCountHook = false,
        // Escape hatch: force the old per-INSTRUCTION HOOK_CODE path. The
        // default is now a per-BLOCK hook doing the same accounting (~16%
        // faster — the crossing, not the work, is what costs). Set this if a
        // firmware genuinely needs instruction-exact instCount or stops.
        perInstHook = false,
        maxBatch = 100000,    // emu_start instruction budget per step()
        onTx = null,          // (frame: Uint8Array, meta) called per TX capture
        eth = {},             // firmware-specific SRAM addresses (defaults above)
        ext_devices = {},
        extra_ram = [],       // [{addr, size}] plain RAM regions mapped before
                              // extra_mem preload (e.g. FSMC SDRAM, WAD ROM at
                              // 0xB8000000 for the doom firmware)
        extra_mem = [],       // [{addr, data}] preloaded into mapped memory (ELF RAM segments)
        uart_addr = 0x40011000, // USART base for uart_rx_byte injection (UART4 = 0x40004C00)
        // Run guest IRQ handlers (USART RXNE etc.) between batches. OFF by
        // default: the ETH firmware is driven by writing irq_flag in SRAM and
        // is corrupted if the guest ETH_IRQHandler also runs (it re-reads
        // DMASR and re-scans rx_desc, stomping rx_frame_idx/len). Enable only
        // for interrupt-driven firmware (rx_interrupt_test, rx_crypto_test).
        enable_irqs = false,
        // Interrupt-driven ETH firmware: the guest ETH_IRQHandler (run by the
        // pump) reads DMASR and scans rx_desc itself, so the driver must NOT
        // write the SRAM irq_flag / rx_frame_idx / rx_frame_len globals that
        // the polling ETH firmware (eth_http) expects. Requires enable_irqs.
        irq_eth = false,
        // FreeRTOS port: deliver the `svc` instruction (used by
        // xPortStartScheduler to start the first task) via the model/pump
        // instead of letting Unicorn take it natively (this WASM build cannot
        // perform the Cortex-M exception return, so a native `svc`/bx-lr
        // crashes). Also switches the interrupt pump to EXC_RETURN-based
        // context restore so FreeRTOS task switches actually take effect.
        // GATED: off by default, so existing firmwares are untouched.
        freertos = false,
        // Low-power: trap WFI/WFE and halt the core until a wakeup source
        // (e.g. RTC alarm) fires. Opt-in so default runs keep their speed.
        lowpower = false,
        // Debug: trace every peripheral MMIO read/write to stderr (used by the
        // `stm32f4-emu --verbose` CLI flag). Capped so a chatty firmware can't
        // flood the terminal.
        verbose = false,
    } = opts;

    // ── firmware image validation (actionable errors on load failure) ──
    if (!firmware || !(firmware instanceof Uint8Array) || firmware.length < 8) {
        const len = firmware && firmware.length !== undefined ? firmware.length : typeof firmware;
        throw new Error(
            `firmware image is invalid (length=${len}): expected a Uint8Array of at least ` +
            `8 bytes containing the Cortex-M vector table (initial SP + reset PC). ` +
            `Load a compiled STM32F4 firmware image (.bin / .elf / .hex).`
        );
    }

    const {
        periph_read, periph_write, tick, tick_n, get_uart_output,
        dma_get_pending_count, dma_get_pending, dma_set_completed,
        dma_periph_read, dma_periph_write,
        is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, init_svd,
        eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll,
        eth_is_rx_poll, eth_clear_rx_poll, eth_tx_done, eth_rx_done,
        get_next_pending_interrupt, set_intr_pending, has_pending_interrupt, pwr_wakeup, uart_rx_byte,
        flash_is_programming, flash_take_erase, flash_erase_applied,
        spi_tap, spi_take_events, spi_push_miso,
        fsmc_tap, fsmc_take_events, fsmc_push_data,
        dcmi_feed_frame, dcmi_clear,
        i2c_register_slave, i2c_take_events, i2c_push_rx,
        i2c_register_regfile, i2c_regfile_get, i2c_regfile_set,
         audio_take_capture, can_inject, tim_inject_capture,
        gpio_read_output, gpio_read_input, gpio_set_input,
        adc_set_channel_value, adc_clear_channel_value,
    } = bindings;

    const E = {
        txDesc: 0x20000610, rxDesc: 0x20000630, txPkt: 0x20000008,
        rxBuf: 0x20000660, rxStride: 1536, irqFlag: 0x20000620,
        rxFrameIdx: 0x20000628, rxFrameLen: 0x2000062c, // SRAM addrs of globals
        rxInjectIdx: 0,                                  // desc index to inject at
        rxDescs: 4,                                      // number of RX descriptors
        ...eth,
    };

    const Module = await unicorn();
    if (typeof bindings.default === 'function') {
        if (wasmInit) await bindings.default({ module_or_path: wasmInit });
        else await bindings.default();
    }
    // Clear process-lifetime wasm globals before registering THIS instance's
    // devices.  ExtDevices is a module-level list and the peripheral
    // constructors bind by first-match, so without this a second
    // createEmulator() in the same process silently attaches to the FIRST
    // instance's devices (measured: a regfile seeded 0x22 read back 0x11).
    // Older wasm bundles may not export it, hence the guard.
    if (typeof bindings.reset_state === 'function') bindings.reset_state();
    for (const cfg of (ext_devices.spi_flash || [])) {
        add_spi_flash(cfg.peripheral, cfg.jedec_id, cfg.data, cfg.cs ?? null);
    }
    for (const cfg of (ext_devices.i2c_eeprom || [])) {
        add_i2c_eeprom(cfg.peripheral, cfg.address, cfg.data);
    }
    // Register-file I2C devices (DS3231 RTC). init is a &[u8] snapshot of the
    // register file: BCD time regs 0x00-0x06, temp MSB/LSB 0x11/0x12. The
    // `rtc` device config is shorthand for a regfile at 0x68 that also
    // enables the emu.rtc time/temp decoder.
    const regfileCfg = [
        ...(ext_devices.regfile || []),
        ...(ext_devices.rtc ? [{
            peripheral: ext_devices.rtc.i2c || 'I2C1',
            address: ext_devices.rtc.addr || 0x68,
            size: ext_devices.rtc.size || 20,
            init: ext_devices.rtc.init || [],
        }] : []),
    ];
    for (const cfg of regfileCfg) {
        i2c_register_regfile(
            cfg.peripheral || 'I2C1', cfg.address || 0x68, cfg.size || 20,
            new Uint8Array(cfg.init || []));
    }
    // Virtual peripheral devices (JS hardware layer). Each is a real device
    // protocol implemented in JS on top of the bus taps, driven by the
    // firmware through the modeled I2C/SPI/TIM/I2S registers.
    if (ext_devices.oled) {
        i2c_register_slave(ext_devices.oled.i2c || 'I2C1', ext_devices.oled.addr || 0x3C);
    }
    if (ext_devices.tft) {
        spi_tap(ext_devices.tft.spi || 'SPI2', ext_devices.tft.cs || null, ext_devices.tft.dc || null);
    }
    // Custom SPI/I2C devices (site/components.js-style embedder devices):
    // { peripheral, cs?, dc?, handler(events, push) } / { peripheral, address, handler }.
    // spi_tap/i2c_register_slave must run before init() below — the Spi/I2c
    // peripheral objects snapshot their attached-device list once at
    // construction and never rescan it (see spi.rs Spi::new).
    const spiDevices = [];
    for (const cfg of (ext_devices.spiDevices || [])) {
        spi_tap(cfg.peripheral, cfg.cs ?? null, cfg.dc ?? null);
        spiDevices.push({ peripheral: cfg.peripheral, handler: cfg.handler });
    }
    const i2cDevices = [];
    for (const cfg of (ext_devices.i2cDevices || [])) {
        i2c_register_slave(cfg.peripheral, cfg.address);
        i2cDevices.push({ peripheral: cfg.peripheral, handler: cfg.handler });
    }
    // Memory-mapped FSMC devices: { bank, handler(events, push) }. `bank` is
    // 0-based (0 = BANK1 @ 0x60000000). Events arrive as PAIRS of words —
    // header then value — because an FSMC access carries an address as well
    // as a value; see the fsmc_tap doc comment in lib.rs. Same before-init()
    // rule as spi_tap: Fsmc binds its banks' devices once at construction.
    const fsmcDevices = [];
    for (const cfg of (ext_devices.fsmcDevices || [])) {
        const bank = cfg.bank ?? 0;
        fsmc_tap(bank);
        fsmcDevices.push({ bank, handler: cfg.handler });
    }
    // Exactly ONE of these. They both install a fresh WasmSystem and the last
    // one wins, so calling init() after init_svd() would replace the
    // SVD-derived peripheral map with the hardcoded one. (It used to be a
    // harmless no-op only because SYS was a OnceLock that ignored the second
    // call — see the SYS comment in stm32-periph-wasm/src/lib.rs.)
    if (svdXml) init_svd(svdXml);
    else bindings.init();

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        // MODE_MCLASS enables Cortex-M banked registers (PSP/MSP) and exception
        // semantics that FreeRTOS relies on (its context switch does `msr psp`).
        // Gated to the FreeRTOS path so other firmwares are untouched.
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN | (freertos ? Module.MODE_MCLASS : 0)
    );

    const mmap = (addr, size, perms, label) => {
        try {
            uc.mem_map(BigInt(addr), size, perms);
        } catch (e) {
            let regions = '?';
            try {
                regions = JSON.stringify(uc.mem_regions());
            } catch (e2) {
                regions = 'regions-err: ' + e2;
            }
            throw new Error(`mem_map ${label} 0x${addr.toString(16)}+0x${size.toString(16)}: ${e} | regions=${regions}`);
        }
    };
    mmap(vector_table & ~0x1FFFF, flash_size, Module.PROT_ALL, 'flash');
    uc.mem_write(BigInt(vector_table), firmware);
    mmap(0x20000000, ram_size, Module.PROT_ALL, 'ram');
    // NOTE: this Unicorn WASM build's mem_write transports data via a
    // stackAlloc copy; subsequent mem_map mallocs can reuse stack-touched
    // heap pages, so "fresh" guest RAM can be seeded with leftovers from the
    // firmware transfer (the 288KB doom.bin is big enough to trigger it).
    // Zero SRAM after mapping so the guest never reads phantom pre-boot data.
    uc.mem_write(BigInt(0x20000000), new Uint8Array(ram_size));
    for (const r of extra_ram) {
        mmap(r.addr, r.size, Module.PROT_ALL, 'extra_ram');
        // same leftover-seeding hazard as SRAM: extra_ram can host .data/.bss
        // (doom's 0xC0000000) — zero it too so bss globals start clean.
        uc.mem_write(BigInt(r.addr), new Uint8Array(r.size));
    }
    for (const seg of extra_mem) {
        uc.mem_write(BigInt(seg.addr), seg.data);
    }

    const periphRanges = [
        [0x40000000, 0xB0000000],
        [0xE0000000, 0xE1000000],
    ];
    for (const [start, end] of periphRanges) {
        mmap(start, end - start, Module.PROT_READ | Module.PROT_WRITE, 'periph');
    }

    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        const v = new DataView(b.buffer, b.byteOffset, b.byteLength);
        return v.getUint32(0, true);
    };
    const write32 = (addr, val) => {
        const b = new Uint8Array(4);
        new DataView(b.buffer).setUint32(0, val >>> 0, true);
        uc.mem_write(BigInt(addr), b);
    };

    // Scratch buffers reused across MMIO reads.  This hook fires on EVERY
    // peripheral read, and polling firmware (ETH spinning on DMASR, USART on
    // TXE) hits it constantly — allocating a fresh Uint8Array each time was
    // pure GC churn.  Safe to reuse: uc.mem_write copies the bytes into guest
    // memory synchronously before returning, so nothing retains the buffer.
    const rdScratch = [];
    // ── verbose peripheral register trace (opts.verbose / --verbose) ──
    const VERBOSE_CAP = 5000;
    let verboseCount = 0;
    const vtrace = (dir, addr, size, val) => {
        if (!verbose || verboseCount >= VERBOSE_CAP) return;
        verboseCount++;
        if (verboseCount === VERBOSE_CAP) {
            process.stderr.write(`[verbose] register trace capped at ${VERBOSE_CAP} accesses\n`);
            return;
        }
        const a = Number(addr);
        process.stderr.write(
            `[reg ${dir}] 0x${a.toString(16).padStart(8, '0')}[${size}] = 0x${val.toString(16)}\n`
        );
    };

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const val = periph_read(Number(address), size) >>> 0;
        vtrace('R', address, size, val);
        let bytes = rdScratch[size];
        if (bytes === undefined) bytes = rdScratch[size] = new Uint8Array(size);
        for (let i = 0; i < size; i++) bytes[i] = (val >> (i * 8)) & 0xFF;
        uc.mem_write(address, bytes);
    };
    const memWriteHook = (handle, type, address, size, value, user_data) => {
        const a = Number(address);
        periph_write(a, size, Number(value));
        vtrace('W', a, size, Number(value));
        if (a >= 0x40023C00 && a <= 0x40023C18) syncFlashProtection(); // FLASH CR writes flip pg
        // FreeRTOS task-context yield: portYIELD() pends PendSV via an SCB ICSR
        // write (bit 28 = PENDSVSET).  Unlike the SVC-based first switch, the
        // Unicorn code hook does not stop on this plain memory write, so the
        // yielding task would keep running (re-looping) until the step budget,
        // and the context switch (delivered once per step, after emu_start) would
        // land while the scheduler is suspended -> vTaskSwitchContext skips it and
        // the task spins forever.  Stop emu_start here so processInterrupts()
        // runs the PendSV handler immediately, after xTaskResumeAll has resumed
        // the scheduler (matching real Cortex-M, where PendSV is lowest priority
        // and fires only after the yielding task yields cleanly).  Only for
        // task-context yields: while delivering a guest ISR (e.g. SysTick) the
        // flag is set and we let the ISR complete first.
        if (freertos && !deliveringIsr && a === 0xE000ED04 && (Number(value) & 0x10000000) !== 0) {
            // The `str PENDSVSET` is still mid-execution when this hook fires, so
            // PC is the store's own address (e.g. 0x8001924), not the next
            // instruction.  Real Cortex-M takes the exception AFTER the
            // instruction completes, so the saved return address must be the
            // following instruction.  Record the frozen PC here and let
            // processInterrupts() advance the resume PC past the store; otherwise
            // the yielding task re-pends PendSV on every resume and spins forever
            // (which deadlocks the scheduler when the highest-prio task yields).
            icsrYieldPc = uc.reg_read_i32(Module.ARM_REG_PC) >>> 0;
            uc.emu_stop();
        }
    };
    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    // FLASH regions: guest stores are hardware-gated by programming mode.
    // While the model says programming is active (unlocked + PG + !BSY) the
    // region is writable so stores land directly; otherwise it is read/exec
    // only — the guest's useless stores fault with UC_ERR_WRITE_PROT and the
    // driver skips the instruction (Unicorn 2.1.4 WASM fires HOOK_MEM_WRITE
    // before the store and cannot block it; WRITE_PROT is the only gate).
    const FLASH_GUEST_START = 0x08000000n;
    const FLASH_GUEST_LEN = 0x100000;
    let flashWritable = false;
    const syncFlashProtection = () => {
        const pg = flash_is_programming();
        if (pg && !flashWritable) {
            uc.mem_protect(FLASH_GUEST_START, FLASH_GUEST_LEN, Module.PROT_ALL);
            flashWritable = true;
        } else if (!pg && flashWritable) {
            uc.mem_protect(FLASH_GUEST_START, FLASH_GUEST_LEN, Module.PROT_READ | Module.PROT_EXEC);
            flashWritable = false;
        }
    };
    uc.mem_protect(FLASH_GUEST_START, FLASH_GUEST_LEN, Module.PROT_READ | Module.PROT_EXEC);

    const stepThroughFlashFault = () => {
        const pc = uc.reg_read_i32(Module.ARM_REG_PC) >>> 0;
        const b = uc.mem_read(BigInt(pc & ~1), 2);
        const h = new DataView(b.buffer, b.byteOffset, b.byteLength).getUint16(0, true);
        const size = (h & 0xE000) === 0xE000 ? 4 : 2; // Thumb-2 wide vs halfword
        uc.reg_write_i32(Module.ARM_REG_PC, (pc + size) | 1);
    };

    // Length in bytes of the Thumb-2 instruction at pc (2 or 4). Used to compute
    // the exception-return address for a mid-instruction yield.
    const thumbInstLen = (pc) => {
        const b = uc.mem_read(BigInt(pc & ~1), 2);
        const hw1 = (b[1] << 8) | b[0];
        if ((hw1 & 0xE000) === 0xE000 && (hw1 & 0x1800) !== 0x0800) return 4;
        return 2;
    };

    const applyFlashErase = () => {
        const er = flash_take_erase();
        if (er.length === 2) {
            const [start, len] = er;
            const ff = new Uint8Array(4096).fill(0xFF);
            for (let off = 0; off < len; off += 4096) {
                const chunk = Math.min(4096, len - off);
                uc.mem_write(BigInt(start + off), ff.subarray(0, chunk));
            }
            flash_erase_applied();
        }
    };

    const sp_init = read32(vector_table);
    const pc_init = read32(vector_table + 4);
    if (sp_init === 0 || pc_init === 0) {
        throw new Error(
            `firmware reset vector is zero (SP=0x${sp_init.toString(16)}, PC=0x${pc_init.toString(16)}). ` +
            `The loaded image does not contain a valid Cortex-M vector table at ` +
            `0x${vector_table.toString(16)} — ensure you loaded a compiled STM32F4 firmware, ` +
            `not raw or empty data.`
        );
    }
    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    let stopRequested = false;
    let instCount = 0;
    let tickAcc = 0;
    let pollAcc = 0;
    const rxQueue = [];
    // FreeRTOS needs the banked PSP, but this Unicorn WASM build only honors
    // PSP register writes made *inside* emu_start (the codeHook context).
    // Writes performed after emu_start returns (e.g. at our synthetic
    // exception return) are silently dropped. So we keep the authoritative
    // PSP in JS (curPSP) and flush it to the real CPU exactly once, from the
    // next hook invocation via pendingPsp — which sticks because it runs
    // inside emu_start, before the resumed thread executes its first insn.
    let curPSP = 0;
    let pendingPsp = null;
    // Captured thread context at exception entry. The handler may clobber the
    // PSP frame in RAM (e.g. SysTick -> vTaskIncrementTick), so for the
    // no-context-switch case we restore from this JS copy instead of memory.
    let lastEntryRegs = null;
    let lastEntryPsp = 0;
    // Frozen PC of a task-context `str PENDSVSET` that was stopped mid-instruction
    // (see the memWriteHook above). 0 when not applicable. Consumed by
    // processInterrupts() to compute the correct exception-return address.
    let icsrYieldPc = 0;
    // True while we are running a guest ISR inside processInterrupts().  Used so
    // the portYIELD() trap below does NOT fire for the SysTick ISR's own
    // PendSV pend (which must run to completion first); it only fires for
    // task-context yields, where we must stop emu_start so the context switch
    // is delivered before the yielding task re-loops with the scheduler
    // suspended (which would otherwise skip the switch and leave it spinning).
    let deliveringIsr = false;

    // ── low-power (WFI/WFE) state ──
    // 0 = awake, 1 = sleep (WFI with SLEEPDEEP clear), 2 = stop (deep sleep).
    let sleeping = 0;
    const RTC_BASE = 0x40002800;       // RTC TR at +0x00 drives time advance
    const WAKE_STEP = 120000;          // virtual instructions advanced per step while asleep

    const codeHook = (handle, address, size, user_data) => {
        if (lowpower) {
            // Decode WFI / WFE and halt the core until a wakeup source fires.
            // WFI is a no-op if an enabled interrupt is already pending, so we
            // only sleep when quiescent. Handle both the 16-bit Thumb encoding
            // (0xBF30 / 0xBF20, what GCC emits for -mthumb) and the 32-bit
            // Thumb-2 hint (0xF3BF 0x8F4F / 0xF3BF 0x8F5F).
            const b2 = uc.mem_read(BigInt(address), 2);
            const hw = (b2[1] << 8) | b2[0];
            let isWfi = false;
            if (hw === 0xBF30 || hw === 0xBF20) isWfi = true;
            else if (hw === 0xF3BF) {
                const b4 = uc.mem_read(BigInt(address), 4);
                const hw2 = (b4[3] << 8) | b4[2];
                if (hw2 === 0x8F4F || hw2 === 0x8F5F) isWfi = true;
            }
            if (isWfi) {
                const primask = uc.reg_read_i32(Module.ARM_REG_PRIMASK) & 1;
                if (primask === 0 && !has_pending_interrupt()) {
                    const scr = periph_read(0xE000ED10, 4);
                    sleeping = ((scr >> 2) & 1) ? 2 : 1; // 2 = STOP (deep sleep)
                    uc.emu_stop();
                    return;
                }
            }
        }
        if (freertos) {
            // Flush the authoritative PSP to the real CPU now (this runs inside
            // emu_start, so the write sticks, unlike one done after emu_start).
            if (pendingPsp !== null) {
                uc.reg_write_i32(Module.ARM_REG_PSP, pendingPsp);
                pendingPsp = null;
            }
                const pc = uc.reg_read_i32(Module.ARM_REG_PC);
                const b = uc.mem_read(BigInt(pc), 2);
            const hw1 = (b[1] << 8) | b[0];
            // Emulate the banked `msr PSP, Rn` / `mrs Rn, PSP` instructions,
            // which this Unicorn WASM build cannot decode (UC_ERR_INSN_INVALID).
            // We perform the register transfer against the JS-shadowed curPSP and
            // advance the PC past the instruction WITHOUT stopping emulation: the
            // handler must keep running (e.g. PendSV's first instruction is
            // `mrs r0, PSP`, and it still needs to switch tasks afterwards).
            if ((hw1 & 0xFFF0) === 0xF380) {
                const b4 = uc.mem_read(BigInt(pc), 4);
                const hw2 = (b4[3] << 8) | b4[2];
                if ((hw2 & 0xFFF0) === 0x8800) { // msr PSP, Rn (Rn = hw1 & 0xF)
                    const rn = hw1 & 0xF;
                    curPSP = uc.reg_read_i32(Module.ARM_REG_R0 + rn);
                    uc.reg_write_i32(Module.ARM_REG_PSP, curPSP); // sticks (in-hook)
                    uc.reg_write_i32(Module.ARM_REG_PC, ((pc & ~1) + 4) | 1);
                    uc.emu_stop();
                    return;
                }
            } else if (hw1 === 0xF3EF) {
                const b4 = uc.mem_read(BigInt(pc), 4);
                const hw2 = (b4[3] << 8) | b4[2];
                if ((hw2 & 0xF0FF) === 0x8009) { // mrs Rn, PSP (Rn = (hw2>>8) & 0xF)
                    const rn = (hw2 >> 8) & 0xF;
                    uc.reg_write_i32(Module.ARM_REG_R0 + rn, curPSP);
                    uc.reg_write_i32(Module.ARM_REG_PC, ((pc & ~1) + 4) | 1);
                    uc.emu_stop();
                    return;
                }
            }
            // Intercept `svc #imm` (Thumb: 0xDF00 | imm) BEFORE Unicorn
            // executes it. Delivering the SVC exception via Unicorn natively is
            // fatal here (it then crashes on the bx lr EXC_RETURN). Instead we
            // synthesize the pending SVC in the model and stop; the pump below
            // delivers vPortSVCHandler with a proper context switch.
            if ((hw1 & 0xFF00) === 0xDF00) {
                set_intr_pending(-5); // IRQ::SVC
                uc.emu_stop();
                return;
            }
        }
        instCount++;
        if (rxQueue.length > 0 && eth_is_rx_poll()) {
            uc.emu_stop();
            return;
        }
        if (minimalPolls) return;
        // The FreeRTOS context-switch path feeds tick_n deterministically from
        // step() (the per-instruction hook under-counts in this Unicorn build),
        // so skip the hook's own tick/poll feeding for FreeRTOS.
        if (freertos) return;
        tickAcc++;
        if (tickAcc >= tickEvery) {
            tickAcc = 0;
            tick_n(tickEvery);
            if (is_watchdog_reset_requested()) {
                // Watchdog expiry → emulate an MCU reset: jump to the reset
                // vector so the firmware's startup re-runs (re-zeros .bss,
                // re-copies .data). The model already latched the cause in
                // RCC->CSR for the firmware to detect. Don't stop — the next
                // batch resumes at the reset handler.
                uc.reg_write_i32(Module.ARM_REG_SP, read32(0x08000000));
                uc.reg_write_i32(Module.ARM_REG_PC, (read32(0x08000004) | 1) >>> 0);
                tickAcc = 0; pollAcc = 0;
                uc.emu_stop();
                return;
            }
        }
        pollAcc++;
        if (pollAcc >= pollEvery) {
            pollAcc = 0;
            applyFlashErase(); // model holds BSY until the erase is applied
            if (dma_get_pending_count() > 0 || eth_is_tx_poll()) {
                uc.emu_stop();
                return;
            }
        }
    };
    // Count-only block hook (doom's old fast path): no tick/poll logic, so it
    // is ONLY valid alongside minimalPolls.
    const blockHook = (handle, address, size, user_data) => {
        instCount += Number(size) / 2; // Thumb-2: block size in bytes
    };
    // Full block hook — same work as codeHook, but once per basic block
    // instead of once per instruction.
    //
    // Measured: the WASM->JS crossing itself costs ~16% of throughput, while
    // the callback BODY costs ~0 (a hook whose body returns immediately is
    // just as slow as one that does the full accounting). Blocks average
    // roughly 5-15 Thumb-2 instructions, so this removes ~80-90% of the
    // crossings and hands that ~16% to EVERY firmware, not just doom.
    //
    // Why this is not a behaviour change: the tick/poll thresholds are 5000
    // and 1000 instructions, so accumulating per block still trips them
    // within one block (~10 inst) of where per-instruction counting would —
    // far finer than the thresholds themselves. The rx-poll stop likewise
    // lands at most a block late, and the driver just services RX and
    // resumes. The one real cost is that instCount becomes approximate (a
    // block that exits early still counts its full size), which affects the
    // MIPS readout but nothing that gates on emu_start's own budget.
    const blockHookFull = (handle, address, size, user_data) => {
        const n = Number(size) / 2;
        instCount += n;
        if (rxQueue.length > 0 && eth_is_rx_poll()) {
            uc.emu_stop();
            return;
        }
        tickAcc += n;
        if (tickAcc >= tickEvery) {
            tick_n(tickAcc);
            tickAcc = 0;
            if (is_watchdog_reset_requested()) {
                // Watchdog expiry → emulate an MCU reset (jump to the reset
                // vector). Model latched the cause in RCC->CSR. Resume at the
                // reset handler rather than stopping.
                uc.reg_write_i32(Module.ARM_REG_SP, read32(0x08000000));
                uc.reg_write_i32(Module.ARM_REG_PC, (read32(0x08000004) | 1) >>> 0);
                tickAcc = 0; pollAcc = 0;
                uc.emu_stop();
                return;
            }
        }
        pollAcc += n;
        if (pollAcc >= pollEvery) {
            pollAcc = 0;
            applyFlashErase(); // model holds BSY until the erase is applied
            if (dma_get_pending_count() > 0 || eth_is_tx_poll()) {
                uc.emu_stop();
                return;
            }
        }
    };
    if (noCountHook) { /* no per-block/per-inst hook: step() accounts the budget */ }
    else if (minimalPolls || lowpower) uc.hook_add(Module.HOOK_CODE, codeHook, null);
    else if (blockCounting) uc.hook_add(Module.HOOK_BLOCK, blockHook, null);
    else if (perInstHook || freertos) uc.hook_add(Module.HOOK_CODE, codeHook, null);
    else uc.hook_add(Module.HOOK_BLOCK, blockHookFull, null);

    // ── virtual peripheral devices (JS hardware layer) ────────────────────
    // Device protocols implemented in JS on top of the bus taps:
    //  - oled:   SSD1306 128x64 over I2C (page-addressed framebuffer)
    //  - tft:    ILI9341 240x320 RGB565 over SPI with a DC line
    //  - buzzer: TIM PWM frequency/duty read from the modeled timer regs
    //  - speaker:I2S TX samples drained from the model's capture FIFO
    // Enabled per-firmware via ext_devices.{oled,tft,buzzer,speaker}.

    const oled = ext_devices.oled ? {
        w: 128, h: 64, col: 0, page: 0, cmdArg: 0, cmdLeft: 0,
        inData: false, needControl: false,
        fb: new Uint8Array(128 * 64), frame: 0,
    } : null;
    const oledI2C = ext_devices.oled ? (ext_devices.oled.i2c || 'I2C1') : null;
    const OLED_ARG_CMDS = { 0x20: 1, 0x21: 1, 0x22: 1, 0x81: 1, 0x8D: 1, 0xA8: 1, 0xD3: 1, 0xD5: 1, 0xD9: 1, 0xDA: 1, 0xDB: 1 };
    const processOled = () => {
        if (!oled) return;
        const events = i2c_take_events(oledI2C);
        for (const ev of events) {
            if (ev & 0x80000000) {
                // START: the next byte is a control byte (0x00 = command
                // group, 0x40 = data group). STOP: group ends here.
                oled.inData = false;
                if (ev & 0x40000000) oled.needControl = true;
                continue;
            }
            const b = ev & 0xFF;
            if (oled.needControl) {
                oled.needControl = false;
                if (b === 0x40) oled.inData = true;
                continue;
            }
            if (oled.inData) {
                for (let bit = 0; bit < 8; bit++) {
                    oled.fb[(oled.page * 8 + bit) * oled.w + oled.col] = (b >> bit) & 1;
                }
                oled.col = (oled.col + 1) % oled.w;
                continue;
            }
            if (oled.cmdLeft > 0) { oled.cmdLeft--; continue; }
            if (b >= 0xB0 && b <= 0xB7) { oled.page = b & 0x07; continue; }
            if (b >= 0x00 && b <= 0x0F) { oled.col = (oled.col & 0xF0) | b; continue; }
            if (b >= 0x10 && b <= 0x1F) { oled.col = ((b & 0x0F) << 4) | (oled.col & 0x0F); continue; }
            const n = OLED_ARG_CMDS[b];
            if (n !== undefined) { oled.cmdLeft = n; continue; }
            // single-byte commands (0xAE/0xAF/0x40/0xA4/0xA6/...) — no action
        }
        if (events.length) oled.frame++;
    };

    const tft = ext_devices.tft ? {
        w: 240, h: 320, x0: 0, y0: 0, x1: 239, y1: 319, x: 0, y: 0,
        mode: 'idle', cmdArg: 0, cmdLeft: 0, argBuf: [],
        fb: new Uint8Array(240 * 320 * 2), frame: 0, pixels: 0,
    } : null;
    const tftSpi = ext_devices.tft ? (ext_devices.tft.spi || 'SPI2') : null;
    const TFT_ARG_CMDS = { 0x2A: 4, 0x2B: 4, 0x36: 1, 0x3A: 1, 0xC0: 2, 0xC1: 1, 0xC5: 2, 0xC7: 1, 0xE0: 15, 0xE1: 15, 0xF6: 3, 0x35: 1, 0x53: 1 };
    const processTft = () => {
        if (!tft) return;
        const events = spi_take_events(tftSpi);
        for (const ev of events) {
            if (ev & 0x80000000) {
                if (ev & 0x40000000) {          // CS asserted: fresh transaction
                    tft.mode = 'idle';
                    tft.cmdLeft = 0;
                }
                continue;
            }
            const dc = (ev >> 29) & 1;
            const b = ev & 0xFF;
            if (tft.mode === 'write') {
                tft.argBuf.push(b);
                if (tft.argBuf.length === 2) {
                    const px = (tft.argBuf[0] << 8) | tft.argBuf[1];
                    const off = (tft.y * tft.w + tft.x) * 2;
                    tft.fb[off] = tft.argBuf[0];
                    tft.fb[off + 1] = tft.argBuf[1];
                    tft.x++;
                    tft.pixels++;
                    if (tft.x > tft.x1) {
                        tft.x = tft.x0;
                        tft.y++;
                        tft.frame++;
                    }
                    if (tft.y > tft.y1) { tft.y = tft.y0; tft.mode = 'idle'; }
                    tft.argBuf.length = 0;
                }
                continue;
            }
            if (!dc) {                          // command byte
                if (b === 0x2C) {
                    tft.mode = 'write';
                    tft.x = tft.x0;
                    tft.y = tft.y0;
                    tft.argBuf.length = 0;
                    continue;
                }
                const n = TFT_ARG_CMDS[b];
                if (n !== undefined) { tft.mode = 'args'; tft.cmdArg = b; tft.cmdLeft = n; tft.argBuf.length = 0; }
                continue;
            }
            // data byte in args mode
            if (tft.mode === 'args') {
                tft.argBuf.push(b);
                tft.cmdLeft--;
                if (tft.cmdLeft === 0) {
                    if (tft.cmdArg === 0x2A && tft.argBuf.length === 4) {
                        tft.x0 = (tft.argBuf[0] << 8) | tft.argBuf[1];
                        tft.x1 = (tft.argBuf[2] << 8) | tft.argBuf[3];
                    } else if (tft.cmdArg === 0x2B && tft.argBuf.length === 4) {
                        tft.y0 = (tft.argBuf[0] << 8) | tft.argBuf[1];
                        tft.y1 = (tft.argBuf[2] << 8) | tft.argBuf[3];
                    }
                    tft.mode = 'idle';
                }
            }
        }
    };

    const buzzer = ext_devices.buzzer ? {
        base: ext_devices.buzzer.tim === 'TIM3' ? 0x40000400
            : ext_devices.buzzer.tim === 'TIM4' ? 0x40000800
            : ext_devices.buzzer.tim === 'TIM5' ? 0x40000C00
            : 0x40000000,                        // TIM2 default
        fclk: 84e6, freq: 0, duty: 0, change: 0,
    } : null;
    const processBuzzer = () => {
        if (!buzzer) return;
        const cr1 = read32(buzzer.base + 0x00);
        const ccer = read32(buzzer.base + 0x20);
        const psc = read32(buzzer.base + 0x28);
        const arr = read32(buzzer.base + 0x2C);
        const ccr = read32(buzzer.base + 0x34);
        let freq = 0, duty = 0;
        if ((cr1 & 1) && (ccer & 1) && ccr > 0 && arr > 0 && arr < 0xFFFFFF) {
            const div = (psc + 1) * (arr + 1);
            if (div > 0) { freq = buzzer.fclk / div; duty = ccr / (arr + 1); }
        }
        if (freq !== buzzer.freq || duty !== buzzer.duty) {
            buzzer.freq = freq; buzzer.duty = duty; buzzer.change++;
        }
    };

    const speaker = ext_devices.speaker ? { ring: [], total: 0, samples: 0 } : null;
    const processSpeaker = () => {
        if (!speaker) return;
        const s = audio_take_capture();
        if (!s || s.length === 0) return;
        const f = new Float32Array(s.length);
        for (let i = 0; i < s.length; i++) f[i] = (s[i] > 0x7FFF ? s[i] - 0x10000 : s[i]) / 32768;
        speaker.ring.push(f);
        speaker.total += f.length;
        if (speaker.ring.length > 64) speaker.ring.shift();
    };
    const takeSpeakerSamples = () => {
        if (!speaker) return new Float32Array(0);
        let n = 0;
        for (const f of speaker.ring) n += f.length;
        const out = new Float32Array(n);
        let off = 0;
        for (const f of speaker.ring) { out.set(f, off); off += f.length; }
        speaker.ring.length = 0;
        return out;
    };

    // DS3231 RTC: register file behind the tap (BCD time regs 0x00-0x06,
    // temp MSB/LSB 0x11/0x12). Read live from the modeled registers; the
    // JS driver seeds the register file (init) and the guest writes it.
    const rtc = ext_devices.rtc ? {
        peri: ext_devices.rtc.i2c || 'I2C1', change: 0, time: null, temp: null,
        lastKey: '',
    } : null;
    const bcd2n = (v) => ((v >> 4) * 10) + (v & 0x0F);
    const processRtc = () => {
        if (!rtc) return;
        const r = (o) => i2c_regfile_get(rtc.peri, o);
        const time = {
            sec: bcd2n(r(0x00)), min: bcd2n(r(0x01)), hour: bcd2n(r(0x02)),
            dow: bcd2n(r(0x03)), day: bcd2n(r(0x04)), mon: bcd2n(r(0x05)),
            year: bcd2n(r(0x06)),
        };
        const tmsb = r(0x11), tlsb = r(0x12);
        const temp = (tmsb & 0x80 ? tmsb - 0x100 : tmsb) + (tlsb >> 6) * 0.25;
        rtc.time = time;
        rtc.temp = temp;
        const key = `${time.sec}:${time.min}:${time.hour}:${time.day}:${time.mon}:${time.year}:${temp}`;
        if (key !== rtc.lastKey) { rtc.lastKey = key; rtc.change++; }
    };

    // ── Public component-attachment API (LED/Button/custom SPI/I2C
    // devices) — built on the same GPIO shims and bus taps the oled/tft
    // blocks above use internally, but reusable by embedder code without
    // editing this file. See site/components.js and docs/components.md.
    const portIndex = (port) => typeof port === 'string' ? port.toUpperCase().charCodeAt(0) - 65 : port;

    const pin = (port, num) => {
        const p = portIndex(port);
        return {
            read: () => gpio_read_output(p, num),
            readInput: () => gpio_read_input(p, num),
            write: (level) => gpio_set_input(p, num, !!level),
        };
    };

    const i2cRegfile = (peripheral) => ({
        get: (offset) => i2c_regfile_get(peripheral, offset),
        set: (offset, value) => i2c_regfile_set(peripheral, offset, value & 0xFF),
    });

    const setAdcChannel = (peripheral, channel, value) => adc_set_channel_value(peripheral, channel, value);
    const clearAdcChannel = (peripheral, channel) => adc_clear_channel_value(peripheral, channel);

    const gpioWatchers = [];
    const watchPin = (port, num, callback) => {
        const p = portIndex(port);
        const w = { p, num, last: gpio_read_output(p, num), callback };
        gpioWatchers.push(w);
        return () => {
            const i = gpioWatchers.indexOf(w);
            if (i >= 0) gpioWatchers.splice(i, 1);
        };
    };
    const processGpioWatchers = () => {
        for (const w of gpioWatchers) {
            const v = gpio_read_output(w.p, w.num);
            if (v !== w.last) { w.last = v; w.callback(v); }
        }
    };

    const processSpiDevices = () => {
        for (const d of spiDevices) {
            const events = spi_take_events(d.peripheral);
            if (events.length) d.handler(events, (bytes) => spi_push_miso(d.peripheral, bytes));
        }
    };

    const processI2cDevices = () => {
        for (const d of i2cDevices) {
            const events = i2c_take_events(d.peripheral);
            if (events.length) d.handler(events, (bytes) => i2c_push_rx(d.peripheral, bytes));
        }
    };

    // ── DCMI camera sensor (JS pixel source) ──
    // The DCMI model consumes one fed frame with real VSYNC/LINE/FRAME/OVR
    // semantics and drops it when fully read, so a live camera just has to
    // keep supplying frames. Unlike the bus taps this needs no registration
    // before init() — the frame slot is a global the model polls.
    const camera = ext_devices.camera || null;
    let cameraFrames = 0;
    let cameraRunning = true;
    const processCamera = () => {
        if (!camera || !cameraRunning) return;
        // `frame(n)` returns the next frame's pixels, or null/undefined to
        // leave the current one in place (a sensor running slower than the
        // step loop, which is the normal case).
        const px = camera.frame ? camera.frame(cameraFrames) : camera.pixels;
        if (!px) return;
        dcmi_feed_frame(camera.width, camera.height,
            px instanceof Uint8Array ? px : Uint8Array.from(px));
        cameraFrames++;
    };

    const processFsmcDevices = () => {
        for (const d of fsmcDevices) {
            const events = fsmc_take_events(d.bank);
            if (events.length) d.handler(events, (values) => fsmc_push_data(d.bank, Uint32Array.from(values)));
        }
    };

    const processDevices = () => {
        processOled();
        processTft();
        processBuzzer();
        processSpeaker();
        processRtc();
        processGpioWatchers();
        processSpiDevices();
        processI2cDevices();
        processFsmcDevices();
        processCamera();
    };

    // ── ETH TX capture + RX injection ──
    const processEth = () => {
        if (eth_is_tx_poll()) {
            const descAddr = eth_get_tx_desc_addr();
            if (descAddr !== 0) {
                const desc = uc.mem_read(BigInt(descAddr), 8);
                const dv = new DataView(desc.buffer, desc.byteOffset, desc.byteLength);
                const tdes0 = dv.getUint32(0, true);
                const tdes1 = dv.getUint32(4, true);
                if (tdes0 & 0x80000000) {
                    const bufAddr = tdes1 & 0xFFFFFFFC;
                    const bufSize = tdes0 & 0x3FFF;
                    if (bufAddr !== 0 && bufSize > 0 && bufSize <= 2000) {
                        const pkt = new Uint8Array(uc.mem_read(BigInt(bufAddr), bufSize));
                        if (onTx) onTx(pkt, { bufAddr, len: bufSize });
                    }
                    const wb = new Uint8Array(4);
                    new DataView(wb.buffer).setUint32(0, (tdes0 & ~0x80000000) | 0x20000000, true);
                    uc.mem_write(BigInt(descAddr), wb);
                }
            }
            eth_clear_tx_poll();
            eth_tx_done();
            if (!irq_eth) write32(E.irqFlag, read32(E.irqFlag) | 1);
        }

        if (eth_is_rx_poll() && rxQueue.length > 0) {
            const idx = E.rxInjectIdx;
            E.rxInjectIdx = (E.rxInjectIdx + 1) % E.rxDescs;
            const descAddr = E.rxDesc + idx * 8;
            const bufAddr = E.rxBuf + idx * E.rxStride;
            const frame = rxQueue.shift();
            const len = Math.min(frame.length, E.rxStride);
            uc.mem_write(BigInt(bufAddr), frame.subarray(0, len));
            const wb = new Uint8Array(4);
            // Real F407: RDES0 high word = frame length [29:16], OWN cleared
            // for CPU ownership; FS/LS live in the low status word (set by
            // the DMA, not the driver). Writing FS/LS marker bits at 28/27
            // would corrupt the length read by the guest ISR.
            new DataView(wb.buffer).setUint32(0, len << 16, true);
            uc.mem_write(BigInt(descAddr), wb);
            if (!irq_eth) {
                write32(E.rxFrameIdx, idx);
                write32(E.rxFrameLen, len);
                write32(E.irqFlag, read32(E.irqFlag) | 2);
            }
            eth_clear_rx_poll();
            eth_rx_done();
        }
    };

    const isPeriphAddr = (a) => (a >= 0x40000000 && a < 0xB0000000) || (a >= 0xE0000000 && a < 0xE1000000);
    const processDma = () => {
        const count = dma_get_pending_count();
        for (let i = 0; i < count; i++) {
            const pending = dma_get_pending(0);
            if (pending.length < 5) continue;
            const dir = pending[0], stream = pending[1], src = pending[2], dst = pending[3], size = pending[4];
            const peri_addr = pending[5] || 0;
            const peripheral = pending[6] || 0;
            try {
                if (dir === 2 || !peripheral || !isPeriphAddr(peri_addr)) {
                    uc.mem_write(BigInt(dst), uc.mem_read(BigInt(src), size));
                } else if (dir === 0) { // read: peri -> RAM
                    const data = dma_periph_read(peri_addr, size, (pending[7] || 0) === 1, pending[8] || 4);
                    uc.mem_write(BigInt(dst), data);
                } else { // dir === 1: write: RAM -> peri
                    dma_periph_write(peri_addr, uc.mem_read(BigInt(src), size));
                }
            } catch (e) { /* ignore */ }
            dma_set_completed(stream, true);
        }
    };

    // ── interrupt pump (runs guest IRQ handlers between batches) ───────────
    // Port of cli.mjs processInterrupts: pushes an exception frame, runs the
    // handler until it aborts on bx lr (EXC_RETURN unsupported), then restores
    // the full context incl. XPSR (condition flags!).
    // Enabled ONLY for interrupt-driven firmware (enable_irqs) — the ETH
    // firmware must NOT use it: the driver signals TX/RX done via SRAM
    // irq_flag + model DMASR, and a guest ETH_IRQHandler run on top re-scans
    // rx_desc and corrupts rx_frame_idx/len (observed garbage response body).
    const processInterrupts = () => {
        while (!stopRequested) {
            const irq = get_next_pending_interrupt();
            if (process.env.FRDEBUG && irq > -100) console.log(`[PI] irq=${irq}`);
            if (irq <= -100) break;
            // In MCLASS mode ARM_REG_SP (id 12) is the *current* SP (== PSP in
            // thread mode), so for the handler frame we must use the dedicated
            // ARM_REG_MSP (the stack handlers actually run on in handler mode).
            const savedAt = uc.reg_read_i32(Module.ARM_REG_MSP);
            const frame = new Uint8Array(32);
            const sv = new DataView(frame.buffer);
            sv.setUint32(0, uc.reg_read_i32(Module.ARM_REG_XPSR), true);
            sv.setUint32(4, uc.reg_read_i32(Module.ARM_REG_PC), true);
            sv.setUint32(8, uc.reg_read_i32(Module.ARM_REG_LR), true);
            sv.setUint32(12, uc.reg_read_i32(Module.ARM_REG_R12), true);
            sv.setUint32(16, uc.reg_read_i32(Module.ARM_REG_R3), true);
            sv.setUint32(20, uc.reg_read_i32(Module.ARM_REG_R2), true);
            sv.setUint32(24, uc.reg_read_i32(Module.ARM_REG_R1), true);
            sv.setUint32(28, uc.reg_read_i32(Module.ARM_REG_R0), true);
            uc.mem_write(BigInt(savedAt - 32), frame);
            uc.reg_write_i32(Module.ARM_REG_MSP, savedAt - 32);
            // Force the handler to execute on the MSP stack. This Unicorn build
            // does not auto-switch to MSP on exception entry. ARM_REG_SP (id 12)
            // is the *current* SP, which (with CONTROL=0x2) aliases PSP — so we
            // must first drop to CONTROL=0 (handler mode => SP == MSP) before
            // setting the SP, otherwise we'd clobber the thread's PSP register.
            uc.reg_write_i32(Module.ARM_REG_CONTROL, 0x0);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt - 32);
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            // Capture the thread's live registers BEFORE we overwrite LR/PC with
            // the synthetic handler context — these are the exception-frame values.
            const threadRegs = {
                r0: uc.reg_read_i32(Module.ARM_REG_R0),
                r1: uc.reg_read_i32(Module.ARM_REG_R1),
                r2: uc.reg_read_i32(Module.ARM_REG_R2),
                r3: uc.reg_read_i32(Module.ARM_REG_R3),
                r12: uc.reg_read_i32(Module.ARM_REG_R12),
                lr: uc.reg_read_i32(Module.ARM_REG_LR),
                pc: uc.reg_read_i32(Module.ARM_REG_PC),
                xpsr: uc.reg_read_i32(Module.ARM_REG_XPSR),
            };
            // A task-context yield was stopped mid-`str PENDSVSET`; advance the
            // saved return PC to the next instruction so the resumed task does not
            // re-execute the store (which would re-pend PendSV and spin forever).
            if (icsrYieldPc !== 0 && (threadRegs.pc >>> 0) === icsrYieldPc) {
                threadRegs.pc = (icsrYieldPc + thumbInstLen(icsrYieldPc)) >>> 0;
            }
            icsrYieldPc = 0;
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);

            if (freertos) {
                // Simulate Cortex-M exception entry onto the *thread's* PSP stack
                // so the FreeRTOS handlers find the 8-word frame they expect
                // (R0,R1,R2,R3,R12,LR,PC,xPSR). SVC is taken from main (which
                // runs on MSP), so use a scratch PSP for its (handler-ignored)
                // frame. We track PSP in JS (curPSP): Unicorn PSP writes done
                // here (outside emu_start) do not stick.
                // The running thread's *actual* SP changes as it push/pops
                // (native instructions), so curPSP cannot stay in sync between
                // exceptions. Derive the frame base from the REAL Unicorn PSP
                // at entry time, which reflects the thread's current SP.
                const realPsp = uc.reg_read_i32(Module.ARM_REG_PSP);
                const psp = (irq === -5)
                    ? 0x2001FFE0
                    : realPsp;
                const fb = new Uint8Array(32);
                const fdv = new DataView(fb.buffer);
                fdv.setUint32(0, threadRegs.r0, true);
                fdv.setUint32(4, threadRegs.r1, true);
                fdv.setUint32(8, threadRegs.r2, true);
                fdv.setUint32(12, threadRegs.r3, true);
                fdv.setUint32(16, threadRegs.r12, true);
                fdv.setUint32(20, threadRegs.lr, true);
                fdv.setUint32(24, threadRegs.pc, true);
                fdv.setUint32(28, threadRegs.xpsr, true);
                uc.mem_write(BigInt(psp - 32), fb);
                curPSP = psp - 32;
                lastEntryPsp = curPSP;
                lastEntryRegs = threadRegs;
                if (process.env.FRDEBUG) {
                    const ew = [];
                    for (let i = 0; i < 8; i++) ew.push(fdv.getUint32(i * 4, true).toString(16));
                    console.log(`[ENTRY] irq=${irq} savedAt(MSP)=${savedAt.toString(16)} realPsp=${realPsp.toString(16)} psp=${psp.toString(16)} fr@=${(psp-32).toString(16)} frame=[${ew.join(',')}] entryPC=${uc.reg_read_i32(Module.ARM_REG_PC).toString(16)}`);
                }
            }

            // Run the handler. This Unicorn WASM build cannot re-enter the code
            // hook after a PC change within a single emu_start, so every
            // emulated instruction (mrs/msr PSP, isb/dsb) stops emulation and we
            // re-start at the new PC. We loop until the handler aborts on its
            // final `bx lr` (EXC_RETURN), which is the expected completion.
            deliveringIsr = true;
            let firstRun = true;
            let guard = 0;
            while (guard++ < 4000) {
                const hpc = firstRun ? handler_pc : (uc.reg_read_i32(Module.ARM_REG_PC) | 1);
                firstRun = false;
                try {
                    uc.emu_start(BigInt(hpc), 0n, 0n, 100000);
                    // Normal return == our hook's emu_stop while emulating an
                    // unsupported instruction; continue the handler.
                } catch (e) {
                    if (process.env.FRDEBUG) console.log(`[ABORT] irq=${irq} handler_pc=0x${handler_pc.toString(16)} pc=0x${(() => { try { return uc.reg_read_i32(Module.ARM_REG_PC).toString(16); } catch { return '?'; } })()} :: ${String(e).slice(0,80)}`);
                    break; // handler aborted on bx lr (EXC_RETURN) — expected
                }
            }
            deliveringIsr = false;

            const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
            const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
            if (freertos) {
                // Perform the Cortex-M exception return ourselves: the frame now
                // lives on PSP — either the original thread's frame (SysTick) or
                // the switched-to task's frame (PendSV/SVC). Restore those
                // registers and pop the frame (curPSP += 32). This is what makes
                // FreeRTOS context switches actually take effect.
                const sp = curPSP;
                // No-context-switch case: the handler left PSP pointing at the same
                // frame we saved at entry, but the handler (e.g. SysTick ->
                // vTaskIncrementTick) has clobbered that RAM. Restore the saved
                // registers from our JS capture instead. A real context switch
                // changes PSP to a different (uncorrupted) task frame, which we
                // restore from memory as normal.
                const switched = (sp !== lastEntryPsp);
                const src = switched ? null : lastEntryRegs;
                const get = (i) => switched
                    ? dv.getUint32(i * 4, true)
                    : [src.r0, src.r1, src.r2, src.r3, src.r12, src.lr, src.pc, src.xpsr][i];
                const pframe = switched ? uc.mem_read(BigInt(sp), 32) : null;
                const dv = switched ? new DataView(pframe.buffer, pframe.byteOffset, pframe.byteLength) : null;
                if (process.env.FRDEBUG) {
                    const w = [];
                    for (let i = 0; i < 8; i++) w.push(get(i).toString(16));
                    console.log(`[RET] irq=${irq} sp=${sp.toString(16)} switched=${switched} frame=[${w.join(',')}] restPC=${get(6).toString(16)} restLR=${get(5).toString(16)} xPSR=${get(7).toString(16)}`);
                }
                uc.reg_write_i32(Module.ARM_REG_R0, get(0));
                uc.reg_write_i32(Module.ARM_REG_R1, get(1));
                uc.reg_write_i32(Module.ARM_REG_R2, get(2));
                uc.reg_write_i32(Module.ARM_REG_R3, get(3));
                uc.reg_write_i32(Module.ARM_REG_R12, get(4));
                uc.reg_write_i32(Module.ARM_REG_LR, get(5));
                uc.reg_write_i32(Module.ARM_REG_PC, get(6) | 1);
                uc.reg_write_i32(Module.ARM_REG_XPSR, get(7));
                curPSP = sp + 32;
                // Sync the real CPU PSP from the next hook (writes after
                // emu_start don't stick — see curPSP/pendingPsp note above).
                pendingPsp = curPSP;
                uc.reg_write_i32(Module.ARM_REG_MSP, savedAt); // restore MSP
                uc.reg_write_i32(Module.ARM_REG_CONTROL, 0x2); // thread mode, use PSP
                uc.reg_write_i32(Module.ARM_REG_PSP, pendingPsp); // thread's real PSP
                if (process.env.FRDEBUG) console.log(`[RET-WRITE] irq=${irq} pendingPsp=0x${pendingPsp.toString(16)}`);
                processDma();
                processEth();
                continue;
            }
            uc.reg_write_i32(Module.ARM_REG_XPSR, savedSv.getUint32(0, true));
            uc.reg_write_i32(Module.ARM_REG_R0, savedSv.getUint32(28, true));
            uc.reg_write_i32(Module.ARM_REG_R1, savedSv.getUint32(24, true));
            uc.reg_write_i32(Module.ARM_REG_R2, savedSv.getUint32(20, true));
            uc.reg_write_i32(Module.ARM_REG_R3, savedSv.getUint32(16, true));
            uc.reg_write_i32(Module.ARM_REG_R12, savedSv.getUint32(12, true));
            uc.reg_write_i32(Module.ARM_REG_LR, savedSv.getUint32(8, true));
            uc.reg_write_i32(Module.ARM_REG_PC, savedSv.getUint32(4, true) | 1);
            uc.reg_write_i32(Module.ARM_REG_MSP, savedAt);
            uc.reg_write_i32(Module.ARM_REG_CONTROL, 0x0); // back to handler mode for next entry
            processDma();
            processEth();
        }
    };

    const step = (max_inst = maxBatch) => {
        if (sleeping) {
            // Core is halted in WFI/WFE. Advance virtual time so wakeup sources
            // (e.g. the RTC alarm) progress, then wake if anything is pending.
            tick_n(WAKE_STEP);
            periph_read(RTC_BASE, 4); // trigger RTC time advance -> may fire alarm -> NVIC
            if (has_pending_interrupt()) {
                if (sleeping === 2) pwr_wakeup(); // set PWR->CSR WUF on wake from STOP
                sleeping = 0;
                // fall through: the guest re-executes WFI, which is now a no-op
                // because an interrupt is pending, then the ISR runs.
            } else {
                instCount += WAKE_STEP;
                return { pc: uc.reg_read_i32(Module.ARM_REG_PC), stopped: false, instCount };
            }
        }
        processDma();
        processEth();
        syncFlashProtection();
        const current_pc = uc.reg_read_i32(Module.ARM_REG_PC);
        try {
            uc.emu_start(BigInt(current_pc | 1), 0n, 0n, max_inst);
        } catch (e) {
            if (String(e).includes('WRITE_PROT')) {
                if (flash_is_programming()) {
                    // Region toggled late: pg is active, so re-enable writes
                    // and re-execute the store from the same PC.
                    uc.mem_protect(FLASH_GUEST_START, FLASH_GUEST_LEN, Module.PROT_ALL);
                    flashWritable = true;
                    uc.emu_start(BigInt(uc.reg_read_i32(Module.ARM_REG_PC) | 1), 0n, 0n, max_inst);
                } else {
                    stepThroughFlashFault();
                }
            } else {
                const pc = uc.reg_read_i32(Module.ARM_REG_PC);
                throw new Error(`emu_start failed at pc=0x${pc.toString(16)}: ${e}`);
            }
        }
        // With no counting hook the budget issued IS the instruction account
        // (emu_start runs to the budget unless something stops it early).
        if (noCountHook) instCount += max_inst;
        if (freertos) {
            // Feed the SysTick deterministically from the step budget. FreeRTOS
            // has no ETH/DMA early-stops, so the full budget is the executed
            // count; relying on the per-instruction hook (which stops re-firing
            // after peripheral memory accesses in this Unicorn build) under-
            // counts and slows the tick ~6x, so TASK1/TASK2 never unblock.
            tickAcc = 0;
            tick_n(max_inst);
        } else {
            tick();
        }
        applyFlashErase();
        syncFlashProtection();
        processDma();
        processEth();
        processDevices();
        if (enable_irqs) processInterrupts();
        const stopped = stopRequested || is_watchdog_reset_requested();
        return { pc: uc.reg_read_i32(Module.ARM_REG_PC), stopped, instCount };
    };

    const run = (max_instructions = 0) => {
        stopRequested = false;
        let totalSteps = 0;
        while (!stopRequested) {
            step();
            totalSteps++;
            if (max_instructions > 0 && instCount >= max_instructions) break;
        }
        return { totalSteps, instCount };
    };

    return {
        uc, Module, read32, write32,
        step, run,
        drainUart() { return get_uart_output(); },
        injectFrame(frame) { rxQueue.push(frame); },
        // Inject a CAN frame from an external transmitter onto the shared bus.
        // Delivered to every CAN node whose accept filters pass it. id is an
        // 11-bit standard ID; data is up to 8 bytes.
        canInject(id, dlc, data) { can_inject(id & 0x7FF, dlc & 0xF, new Uint8Array(data)); },
        timInjectCapture(name, ch) { tim_inject_capture(name, ch & 0x3); },
        sendUartByte(b) { return uart_rx_byte(uart_addr, b & 0xFF); },
        sendUart(bytes) {
            for (const b of bytes) uart_rx_byte(uart_addr, b & 0xFF);
        },
        rxQueue,
        pin, watchPin, i2cRegfile, setAdcChannel, clearAdcChannel,
        oled: oled ? { fb: oled.fb, frame: () => oled.frame } : null,
        tft: tft ? { fb: tft.fb, w: tft.w, h: tft.h, frame: () => tft.frame } : null,
        buzzer: buzzer ? { get freq() { return buzzer.freq; }, get duty() { return buzzer.duty; }, get change() { return buzzer.change; } } : null,
        takeSpeakerSamples,
        rtc: rtc ? { get time() { return rtc.time; }, get temp() { return rtc.temp; }, get change() { return rtc.change; } } : null,
        // DCMI pixel source. Usable with or without `ext_devices.camera`:
        // feed() hands the model one frame directly, stop() empties the slot.
        camera: {
            feed(w, h, pixels) {
                dcmi_feed_frame(w, h, pixels instanceof Uint8Array ? pixels : Uint8Array.from(pixels));
            },
            // Unplug the sensor: drops the pending frame AND stops any
            // `ext_devices.camera` source from feeding more, so a later
            // feed() is not overwritten on the next step.
            stop() { cameraRunning = false; dcmi_clear(); },
            start() { cameraRunning = true; },
            get frames() { return cameraFrames; },
        },
        // Push values a memory-mapped FSMC device answers on bank reads,
        // for embedders driving a bank without a `fsmcDevices` handler.
        pushFsmcData(bank, values) { fsmc_push_data(bank, Uint32Array.from(values)); },
        takeFsmcEvents(bank) { return fsmc_take_events(bank); },
        stop() {
            stopRequested = true;
            try { uc.emu_stop(); } catch (e) {}
        },
        getRegisters() {
            const out = {};
            for (const n of ['R0','R1','R2','R3','R4','R5','R6','R7','R8','R9','R10','R11','R12']) {
                out[n] = uc.reg_read_i32(Module['ARM_REG_' + n]);
            }
            out.SP = uc.reg_read_i32(Module.ARM_REG_SP);
            out.LR = uc.reg_read_i32(Module.ARM_REG_LR);
            out.PC = uc.reg_read_i32(Module.ARM_REG_PC);
            out.XPSR = uc.reg_read_i32(Module.ARM_REG_XPSR);
            return out;
        },
        close() { try { uc.close(); } catch (e) {} },
    };
}
