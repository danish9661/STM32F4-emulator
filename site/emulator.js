// Universal STM32F407 emulator factory — runs in Node and the browser.
// No imports: the caller supplies the bindings (wasm-bindgen module),
// the SVD XML, and the firmware bytes. The CPU core is the WASM-native
// Thumb-2 interpreter (WasmCpu); there is no Unicorn dependency.
//
// Default eth_* constants match eth_http/eth_http.elf (nm-verified):
//   irq_flag 0x20000620, rx_frame_idx 0x20000628, rx_frame_len 0x2000062c,
//   tx_desc 0x20000610, rx_desc 0x20000630, tx_pkt 0x20000008, rx_buf 0x20000660

export async function createEmulator(opts) {
    const {
        firmware,
        bindings,             // wasm-bindgen module (web or nodejs build)
        svdXml,               // SVD XML string for init_svd
        wasmInit,             // optional: wasm bytes for bindings.default() (Node)
        wasmUrl,              // optional: versioned wasm URL for bindings.default() (browser cache-busting; see VENDOR_V)
        flash_size = 0x100000,
        ram_size = 0x20000,
        vector_table = 0x08000000,
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
        // xPortStartScheduler to start the first task) via exact inline
        // exception entry/return in the Rust core, with EXC_RETURN-based
        // context restore so FreeRTOS task switches take effect.
        // GATED: off by default, so existing firmwares are untouched.
        freertos = false,
        // Low-power: trap WFI/WFE and halt the core until a wakeup source
        // (e.g. RTC alarm) fires. Opt-in so default runs keep their speed.
        lowpower = false,
    } = opts;

    // Browser-safe env (Node `process` does not exist in the page; the
    // WASM-step debug flags below would otherwise throw on every step).
    const ENV = (typeof process !== 'undefined' && process.env) || {};

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
        periph_read, periph_write, tick, tick_n, tick_peripherals, get_uart_output,
        dma_get_pending_count, dma_get_pending, dma_set_completed,
        dma_periph_read, dma_periph_write,
        is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, qspi_register_flash, init_svd,
        eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll,
        eth_is_rx_poll, eth_get_rx_desc_addr, eth_clear_rx_poll, eth_tx_done, eth_rx_done,
        eth_mac_accept, eth_rx_csum_status, eth_check_wol, eth_tx_wire_busy,
        eth_rx_wire_busy, eth_arm_collision, eth_take_collision,
        eth_set_link, eth_link_up, eth_tx_deferred,
        eth_tx_done_now,
        eth_get_maccr, eth_loopback_tx, eth_ptp_tse, eth_ptp_sec, eth_ptp_sub,
        eth_station_addr, eth_tx_sarc, eth_ipco_on, eth_fwd_csum_bad,
        eth_tx_jabber_limit, eth_pause_rx, eth_take_pause_tx,
        eth_note_tx, eth_note_rx, eth_note_missed,
        eth_note_rx_stall, eth_rx_stall_clear, eth_note_jabber,
        get_next_pending_interrupt, set_intr_pending, has_pending_interrupt, pwr_wakeup, uart_rx_byte,
        flash_is_programming, flash_take_erase, flash_erase_applied,
        dma2d_take_job, dma2d_job_done, dma2d_convert, dma2d_blend,
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

    // IRQ-mode RX delivery: the guest owns its descriptor layout. The DMA
    // takes the frame ONLY if the polled head descriptor (DMARDLAR base)
    // is DMA-owned; otherwise (RBUS, like silicon) the frame stays queued
    // for the next poll. NEVER scan forward: past a single-entry ring
    // lies ordinary guest RAM, and an OWN-looking garbage word would
    // divert the frame into the wild (observed: DHCP Offer-shaped bytes
    // misdelivered, ping lost).
    // NO static-layout fallback: when no poll was ever armed
    // (listBase==0) there is nowhere silicon could deliver either —
    // hold the frame AND the (absent) poll for the next poll. (An older
    // revision delivered to a hardcoded eth_http SRAM layout here; that
    // wrote frames into unrelated guest RAM for every other firmware —
    // observed as a netsim-shaped "SARC" frame with len 0x74.)
    // Returns true on delivery.
    // rdesExtra carries RDES0 status bits (IPHCE/PCE); with PTP TSE the
    // snapshot goes to RDES6/7 (guest needs 32-byte descriptors).
    const injectRxIrq = (memWrite, memRead32, frame, len, rdesExtra) => {
        let listBase = 0;
        try { listBase = eth_get_rx_desc_addr() >>> 0; } catch {}
        if (listBase === 0) {
            // No poll ever armed: hold (do NOT latch RBUS — nothing is
            // "busy", the guest simply hasn't armed yet).
            return false;
        }
        {
            let rdes0 = 0, rdes1 = 0;
            try {
                rdes0 = memRead32(listBase) >>> 0;
                rdes1 = memRead32(listBase + 4) >>> 0;
            } catch { return false; }
            if (!((rdes0 & 0x80000000) && rdes1 !== 0)) {
                // Head still CPU-owned: hold the frame AND the poll
                // (silicon RBUS) for the next poll, like the driver loop.
                try { eth_note_rx_stall(); } catch {}
                return false;
            }
            try {
                // LEN RULE (silicon): RDES0[29:16] is the FRAME length
                // but the DMA never reports a runt — frames shorter than
                // 64 B (60 B payload + 4 B FCS the MAC strips) pad to 60.
                // A 50 B loopback frame therefore reports 60 (0x3C).
                // (Enhanced-desc layouts only: the PTP snapshot at +24
                // needs a 32-byte descriptor. On a short layout (fewer
                // than 32 bytes before the next descriptor/buffer) the
                // +24 write would clobber the NEXT descriptor or the RX
                // buffer — observed as RBUS CLEAR FAIL with rx0 stuck at
                // 0x003C0000 on the single-desc feat layout. Gate on the
                // configured stride.)
                // RDES0 status (HAL stm32f4xx_hal_eth.h ETH_DMARXDESC_*):
                // FS=bit9 + LS=bit8 ALWAYS set on a delivered frame (the
                // DMA owns single-buffer delivery end-to-end here; ES=bit15
                // rides along inside rdesExtra when the csum gate raised
                // it). Previously neither was set — firmware reading
                // FS/LS (any HAL-based RX path) saw a "middle fragment"
                // forever and never consumed the frame.
                const RDESC_FS = 0x200, RDESC_LS = 0x100;
                const wire = len < 60 ? 60 : len;
                const out = new Uint8Array(wire);
                out.set(frame.subarray(0, len));
                memWrite(BigInt(rdes1), out);
                const wb = new Uint8Array(4);
                new DataView(wb.buffer).setUint32(0, (wire << 16) | rdesExtra | RDESC_FS | RDESC_LS, true);
                memWrite(BigInt(listBase), wb);
                if (eth_ptp_tse() && (E.rxStride || 0) >= 32) {
                    const sb = new Uint8Array(8);
                    const sdv = new DataView(sb.buffer);
                    sdv.setUint32(0, eth_ptp_sec(), true);
                    sdv.setUint32(4, eth_ptp_sub(), true);
                    memWrite(BigInt(listBase + 24), sb);
                }
                return true;
            } catch { return false; }
        }
        const idx = E.rxInjectIdx;
        E.rxInjectIdx = (E.rxInjectIdx + 1) % E.rxDescs;
        // Same runt-pad LEN rule as the IRQ path above, same FS+LS status
        // (single-buffer delivery: first AND last segment always).
        const RDESC_FS_LS = 0x300;
        const wire = len < 60 ? 60 : len;
        const out = new Uint8Array(wire);
        out.set(frame.subarray(0, len));
        memWrite(BigInt(E.rxBuf + idx * E.rxStride), out);
        const wb = new Uint8Array(4);
        new DataView(wb.buffer).setUint32(0, (wire << 16) | rdesExtra | RDESC_FS_LS, true);
        memWrite(BigInt(E.rxDesc + idx * 8), wb);
        return true;
    };

    // Internet checksum over pkt[off..off+len].
    const ethCksum = (pkt, off, len) => {
        let s = 0;
        for (let i = 0; i + 1 < len; i += 2) s += (pkt[off + i] << 8) | pkt[off + i + 1];
        if (len & 1) s += pkt[off + len - 1] << 8;
        while (s >>> 16) s = (s & 0xFFFF) + (s >>> 16);
        return (~s) & 0xFFFF;
    };

    // PTP event-message test (silicon snapshots ONLY these): ethertype
    // 0x88F7 (any payload), or IPv4/IPv6 UDP to port 319/320. VLAN-tagged
    // forms count (one 802.1Q tag skipped). Everything else — the feat
    // UDP probe, TCP, ICMP, ARP — is NOT an event message.
    const ptpIsEvent = (pkt) => {
        if (!pkt || pkt.length < 14) return false;
        let off = 12;
        let et = (pkt[off] << 8) | pkt[off + 1];
        if (et === 0x8100) {
            if (pkt.length < 18) return false;
            et = (pkt[16] << 8) | pkt[17];
            off = 16;
        }
        if (et === 0x88F7) return true;
        if (et !== 0x0800 && et !== 0x86DD) return false;
        const ihl = et === 0x0800 ? (pkt[off + 2] & 0x0F) * 4 : 40;
        const l4 = off + 2 + ihl;
        if (pkt.length < l4 + 4) return false;
        const proto = et === 0x0800 ? pkt[off + 2 + 9] : pkt[off + 2 + 6];
        if (proto !== 17) return false;
        const dport = (pkt[l4 + 2] << 8) | pkt[l4 + 3];
        return dport === 319 || dport === 320;
    };

    // TX checksum offload (TDES0 CIC): insert IP header + TCP/UDP/ICMP
    // checksums into the captured copy AND guest buffer (via wr16, like
    // silicon). Defined here (wuc is in TDZ above); called from wProcessEth.
    const txInsertCsum = (pkt, wr16, cic) => {
        if (pkt.length < 14 || cic === 0) return;
        // Untagged frames: ethertype at 12, L3 at 14 (NOT 12 — that
        // misaligns IHL/proto and writes checksums into TTL/proto).
        let off = 14, et = (pkt[12] << 8) | pkt[13];
        if (et === 0x8100) {
            if (pkt.length < 18) return;
            off = 18; et = (pkt[16] << 8) | pkt[17];
        }
        if (et !== 0x0800 || pkt.length < off + 20) return;
        const ihl = (pkt[off] & 0xF) * 4;
        if (ihl < 20 || pkt.length < off + ihl) return;
        const set16 = (at, v) => {
            pkt[at] = (v >> 8) & 0xFF; pkt[at + 1] = v & 0xFF;
            wr16(at, v);
        };
        pkt[off + 10] = 0; pkt[off + 11] = 0;
        set16(off + 10, ethCksum(pkt, off, ihl));
        if (cic < 2) return; // IP header only
        const proto = pkt[off + 9], l4 = off + ihl;
        const pseudo = (pl, p) => {
            let s = 0;
            for (let i = 0; i < 4; i += 2) s += (pkt[off + 12 + i] << 8) | pkt[off + 13 + i];
            for (let i = 0; i < 4; i += 2) s += (pkt[off + 16 + i] << 8) | pkt[off + 17 + i];
            s += p + pl;
            for (let i = 0; i + 1 < pl; i += 2) s += (pkt[l4 + i] << 8) | pkt[l4 + i + 1];
            if (pl & 1) s += pkt[l4 + pl - 1] << 8;
            while (s >>> 16) s = (s & 0xFFFF) + (s >>> 16);
            return (~s) & 0xFFFF;
        };
        if (proto === 6 && pkt.length >= l4 + 20) {
            pkt[l4 + 16] = 0; pkt[l4 + 17] = 0;
            set16(l4 + 16, pseudo(pkt.length - l4, 6));
        } else if (proto === 17 && pkt.length >= l4 + 8) {
            pkt[l4 + 6] = 0; pkt[l4 + 7] = 0;
            set16(l4 + 6, pseudo(Math.max(8, Math.min(pkt.length - l4, ((pkt[l4 + 4] << 8) | pkt[l4 + 5]) || 8)), 17));
        } else if (proto === 1 && pkt.length >= l4 + 4) {
            pkt[l4 + 2] = 0; pkt[l4 + 3] = 0;
            set16(l4 + 2, ethCksum(pkt, l4, pkt.length - l4));
        }
    };

    if (typeof bindings.default === 'function') {
        if (wasmInit) await bindings.default({ module_or_path: wasmInit });
        else if (wasmUrl) await bindings.default({ module_or_path: wasmUrl });
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
    // QSPI flash backend. MUST run before init_svd(): the QUADSPI peripheral
    // binds its flash image at construction time (Qspi::new clones the global
    // QSPI_FLASH), so the backend must be registered first.
    for (const cfg of (ext_devices.qspi || [])) {
        qspi_register_flash(cfg.peripheral || 'QUADSPI', new Uint8Array(cfg.data || new Uint8Array(cfg.size || 256)));
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

    // ── shared virtual-peripheral devices (moved above the wasm branch
    // so BOTH backends use them; the wasm step() calls processDevices()
    // too). Register reads go through devRead32 (wread32/read32 per backend).
    let devRead32 = null;
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
        if (!buzzer || !devRead32) return;
        const cr1 = devRead32(buzzer.base + 0x00);
        const ccer = devRead32(buzzer.base + 0x20);
        const psc = devRead32(buzzer.base + 0x28);
        const arr = devRead32(buzzer.base + 0x2C);
        const ccr = devRead32(buzzer.base + 0x34);
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

    // Unified pin handle: pin(port, num) (components.js / facade form) or
    // pin('PA5') (ws-bridge form). Method set is the union both callers need.
    const pin = (portOrName, num) => {
        let p = portIndex(portOrName);
        let n = num;
        if (n === undefined && typeof portOrName === 'string') {
            const m = /^P([A-E])([0-9]|1[0-5])$/i.exec(portOrName || '');
            if (!m) return null;
            p = m[1].toUpperCase().charCodeAt(0) - 65;
            n = parseInt(m[2], 10);
        }
        if (p === null || p === undefined || n === undefined) return null;
        return {
            read: () => gpio_read_output(p, n),
            readInput: () => gpio_read_input(p, n),
            write: (level) => gpio_set_input(p, n, !!level),
            setInputValue: (v) => gpio_set_input(p, n, !!v),
            getOutputValue: () => gpio_read_output(p, n),
            getInputValue: () => gpio_read_input(p, n),
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
        if (oled) processOled();
        if (tft) processTft();
        if (buzzer) processBuzzer();
        // Speaker drain is the only device DOOM enables: keep its per-step
        // cost as one guarded wasm call (the FIFO is usually empty; the
        // take path copies only when samples exist). Everything else above
        // is a null check when the firmware didn't request the device.
        if (speaker) processSpeaker();
        if (rtc) processRtc();
        if (gpioWatchers.length) processGpioWatchers();
        if (spiDevices.length) processSpiDevices();
        if (i2cDevices.length) processI2cDevices();
        if (fsmcDevices.length) processFsmcDevices();
        if (camera) processCamera();
    };
    // Sole CPU backend: the WASM-native Thumb-2 interpreter.
    if (typeof bindings.WasmCpu !== 'function') {
        throw new Error('emulator.js requires the WasmCpu backend (Unicorn support was removed)');
    }
    {
        // Guest exception delivery (SysTick/ETH/USART IRQs, SVC/PendSV,
        // WFI sleep) runs inline in the Rust core — no ISR pump needed:
        // stacking is exact, so no mid-instruction hazard can occur.
        const wantIrqs = enable_irqs || irq_eth || freertos || lowpower;
        const sp0 = (firmware[0] | (firmware[1] << 8) | (firmware[2] << 16) | (firmware[3] << 24)) >>> 0;
        const pc0 = (firmware[4] | (firmware[5] << 8) | (firmware[6] << 16) | (firmware[7] << 24)) >>> 0;
        if (sp0 === 0 || pc0 === 0) throw new Error(`WasmCpu: invalid vector table sp=${sp0.toString(16)} pc=${pc0.toString(16)}`);
        const cpu = new bindings.WasmCpu(sp0, pc0 | 1, flash_size, ram_size);
        if (wantIrqs && typeof cpu.set_deliver_irqs === 'function') cpu.set_deliver_irqs(true);
        cpu.load_firmware(firmware, vector_table);
        for (const r of extra_ram) cpu.load_firmware(new Uint8Array(r.size), r.addr);
        for (const seg of extra_mem) cpu.load_firmware(seg.data, seg.addr);
        // Byte-correct uc shim over the wasm memory, used by the
        // processEth/processDma mirrors below.
        const wuc = {
            mem_read: (a, s) => { const aa = Number(a), ss = Number(s); wCheckMapped(aa, ss, 'read'); return new Uint8Array(cpu.mem_read(aa, ss)); },
            mem_write: (a, d) => { const aa = Number(a), dd = new Uint8Array(d); wCheckMapped(aa, dd.length, 'write'); return cpu.mem_write(aa, dd); },
            reg_read_i32: (r) => r === 15 ? cpu.get_pc() : r === 13 ? cpu.get_sp() : cpu.get_regs()[r],
        };
        const wread32 = (a) => cpu.read32(Number(a)) >>> 0;
        const wwrite32 = (a, v) => cpu.write32(Number(a), v >>> 0);
        devRead32 = wread32;
        // Reject out-of-range MMIO (the edge-case test requires
        // mem_read/mem_write to throw, not silently drop).
        const wMapped = (addr, len) => {
            if (addr >= 0x08000000 && addr + len <= 0x08000000 + flash_size) return true;
            if (addr >= 0x20000000 && addr + len <= 0x20000000 + ram_size) return true;
            if ((addr >= 0x40000000 && addr + len <= 0xB0000000) ||
                (addr >= 0xE0000000 && addr + len <= 0xE1000000)) return true;
            for (const r of extra_ram) {
                if (addr >= r.addr && addr + len <= r.addr + r.size) return true;
            }
            return false;
        };
        const wCheckMapped = (addr, len, what) => {
            if (!wMapped(addr, len)) {
                throw new Error(`WasmCpu: ${what} of unmapped address 0x${addr.toString(16)} (len ${len})`);
            }
        };
        const rxQueue = [];
        let instCount = 0;
        let lastTxLen = 0; // bytes of the last TX frame (wire pacing)
        // Bound the queue against a hung guest (real NICs tail-drop
        // too): newest frames past 32 are dropped, each counted as a
        // missed frame (MFC + ROS in the model).
        const trimRxQueue = () => {
            try {
                while (rxQueue.length > 32) { rxQueue.pop(); eth_note_missed(); }
            } catch {}
        };
        // Compact mirrors of processEth/processDma for the wasm memory.
        // Shared RX delivery: WOL inspect, accept gate, checksum
        // status, snapshot, descriptor write. `force` bypasses the
        // RX-poll requirement (sleep drain: the wire delivers even
        // when the guest can't re-arm). Returns true when a frame
        // was consumed from the queue.
            const wDeliverRx = (force) => {
            if (rxQueue.length === 0) return false;
            if (!force && !eth_is_rx_poll()) return false;
            // Peek first: a busy head (or filter drop) must not consume.
            const frame = rxQueue[0];
            const len = Math.min(frame.length, E.rxStride);
            // Wake-on-LAN inspection runs before filtering (it works
            // in powerdown too); then the MAC accept filter drops
            // rejected frames with no RS/flag (like silicon).
            try { eth_check_wol(frame); } catch {}
            // Dead receiver (RE clear) drops everything past WOL
            // inspection (powerdown WOL still sees); no RS, no count.
            try {
                if ((eth_get_maccr() & 0x4) === 0) {
                    rxQueue.shift();
                    eth_clear_rx_poll();
                    return true;
                }
            } catch {}
            // Flow-control pause: terminates here (never delivered or
            // counted) after arming the TX stall in the model.
            try {
                if (eth_pause_rx(frame)) {
                    rxQueue.shift();
                    eth_clear_rx_poll();
                    return true;
                }
            } catch {}
            let accepted = true;
            try { accepted = eth_mac_accept(frame); } catch {}
            if (!accepted) {
                rxQueue.shift();
                eth_clear_rx_poll();
                if (ENV.WASM_DBG) console.log('[wasm-rx] dropped by accept filter');
                return true;
            }
            // RX checksum status -> RDES0 IPHCE(7)/PCE(0)/ES(15), gated
            // by IPCO (engine off reports nothing, like silicon).
            // LEN RULE (silicon): RDES0[29:16] is the FRAME length but
            // the DMA never reports a runt — frames shorter than 64 B
            // (60 B payload + 4 B FCS the MAC strips) are padded to 60.
            // A 50 B loopback frame therefore reports 60 (0x3C), and
            // firmware must accept len >= its payload, not len ==.
            // FEF GATE (silicon): the checksum engine must actually RUN
            // for a frame to be "checksum-bad". With IPCO off the engine
            // is off, st==0, nothing is bad, and FEF is irrelevant — the
            // frame always delivers. Computing status unconditionally
            // and dropping on it starved every IPCO-off phase (RBUS f2
            // held forever behind a CPU-owned head: RBUS CLEAR FAIL,
            // then JABBER WD + all downstream filters).
            let rdesExtra = 0, csumBad = false;
            try {
                if (eth_ipco_on()) {
                    const st = (eth_rx_csum_status(frame) >>> 0);
                    if ((st & 1) && !(st & 2)) { rdesExtra |= 0x80; csumBad = true; }
                    if ((st & 4) && !(st & 8)) { rdesExtra |= 0x01; csumBad = true; }
                    if (csumBad) rdesExtra |= 0x8000; // ES error summary
                }
            } catch {}
            // Forward-error-frames gate: checksum-bad frames are dropped
            // unless FEF forwards them or DTCEFD disables the drop.
            if (csumBad) {
                try {
                    if (!eth_fwd_csum_bad()) {
                        rxQueue.shift();
                        eth_clear_rx_poll();
                        return true;
                    }
                } catch {}
            }
            let delivered = false;
            if (!irq_eth) {
                const idx = E.rxInjectIdx;
                E.rxInjectIdx = (E.rxInjectIdx + 1) % E.rxDescs;
                const descAddr = E.rxDesc + idx * 8;
                const bufAddr = E.rxBuf + idx * E.rxStride;
                try {
                    // Same runt-pad LEN rule as injectRxIrq above.
                    const wire = len < 60 ? 60 : len;
                    const out = new Uint8Array(wire);
                    out.set(frame.subarray(0, len));
                    wuc.mem_write(BigInt(bufAddr), out);
                    const wb = new Uint8Array(4);
                    new DataView(wb.buffer).setUint32(0, (wire << 16) | rdesExtra, true);
                    wuc.mem_write(BigInt(descAddr), wb);
                    wwrite32(E.rxFrameIdx, idx);
                    wwrite32(E.rxFrameLen, len);
                    wwrite32(E.irqFlag, wread32(E.irqFlag) | 2);
                    delivered = true;
                } catch {}
            } else {
                // IRQ-driven firmware owns its descriptor layout: deliver
                // at the polled head or hold for the next poll (RBUS).
                try {
                    delivered = injectRxIrq((a, d) => wuc.mem_write(a, d), wread32, frame, len, rdesExtra);
                } catch { delivered = false; }
            }
            if (!delivered) return false; // hold frame AND poll for retry
            rxQueue.shift();
            trimRxQueue();
            eth_clear_rx_poll();
            try { eth_rx_stall_clear(); } catch {}
            try { eth_note_rx(frame); } catch {}
            // Delivered frames pace RS by their wire time.
            try { eth_rx_wire_busy(len); } catch {}
            eth_rx_done();
            return true;
        };

        const wProcessEth = () => {
            // Emitted pause frame (MACFCR FCB/BPA with TFCE): a 64 B
            // pause frame (multicast pause DA, station SA, 0x8808/0001,
            // PT quanta) goes on the wire — or loopback, like any TX.
            // Dead wire drops it silently (no carrier, like silicon).
            try {
                const pq = eth_take_pause_tx() >>> 0;
                if (pq & 0x80000000) {
                    let linkUp = true;
                    try { linkUp = eth_link_up(); } catch {}
                    if (linkUp) {
                        const quanta = pq & 0xFFFF;
                        const sa = eth_station_addr();
                        const pf = new Uint8Array(64);
                        pf.set([0x01, 0x80, 0xC2, 0x00, 0x00, 0x01], 0);
                        for (let i = 0; i < 6; i++) pf[6 + i] = Number((sa >> BigInt(8 * (5 - i))) & 0xFFn);
                        pf[12] = 0x88; pf[13] = 0x08;
                        pf[14] = 0x00; pf[15] = 0x01;
                        pf[16] = (quanta >> 8) & 0xFF; pf[17] = quanta & 0xFF;
                        if (eth_loopback_tx()) {
                            rxQueue.push(pf);
                            trimRxQueue();
                        } else if (onTx) {
                            onTx(pf, { bufAddr: 0, len: 64 });
                        }
                    }
                }
            } catch {}
            if (eth_is_tx_poll()) {
                const descAddr = eth_get_tx_desc_addr();
                if (ENV.WASM_DBG) console.log(`[wasm-tx] poll desc=0x${descAddr.toString(16)}`);
                if (descAddr !== 0) {
                    const desc = wuc.mem_read(BigInt(descAddr), 8);
                    const dv = new DataView(desc.buffer, desc.byteOffset, desc.byteLength);
                    const tdes0 = dv.getUint32(0, true);
                    const tdes1 = dv.getUint32(4, true);
                    // EMPTY-POLL GUARD: OWN set but len 0 (or null buffer)
                    // is not a frame — the guest hasn't programmed this
                    // descriptor yet. Service it as an empty completion
                    // (OWN-clear + TS, no wire, no note): silicon
                    // completes zero-length descriptors the same way,
                    // and dropping the poll WITHOUT completing would
                    // wedge the guest's TX wait (its OWN bit never
                    // clears — observed: 46 back-to-back re-polls of
                    // len-0x2E while the guest spun).
                    if ((tdes0 & 0x80000000) && ((tdes0 & 0x3FFF) === 0 || (tdes1 & 0xFFFFFFFC) === 0)) {
                        const wb = new Uint8Array(4);
                        new DataView(wb.buffer).setUint32(0, tdes0 & ~0x80000000, true);
                        wuc.mem_write(BigInt(descAddr), wb);
                        eth_clear_tx_poll();
                        eth_tx_done();
                        if (!irq_eth) wwrite32(E.irqFlag, wread32(E.irqFlag) | 1);
                    } else if (tdes0 & 0x80000000) {
                        const bufAddr = tdes1 & 0xFFFFFFFC;
                        const bufSize = tdes0 & 0x3FFF;
                        if (ENV.WASM_DBG) console.log(`[wasm-tx] tdes0=0x${tdes0.toString(16)} buf=0x${bufAddr.toString(16)} len=${bufSize}`);
                        let linkUp = true, captured = false, txJabber = false, txDeadWire = false, txEc = 0;
                        try { linkUp = eth_link_up(); } catch {}
                        if (!linkUp) {
                            txDeadWire = true;
                            // Dead wire: NC status, nothing on the wire.
                            // Completion still raises TS (error completion,
                            // like silicon) — the driver learns it from NC.
                            const wb = new Uint8Array(4);
                            new DataView(wb.buffer).setUint32(0, (tdes0 & ~0x80000000) | 0x400, true);
                            wuc.mem_write(BigInt(descAddr), wb);
                        } else {
                        let ttss = 0;
                        let jlim = 2048;
                        try { jlim = eth_tx_jabber_limit() >>> 0; } catch {}
                        if (bufAddr !== 0 && bufSize > jlim) {
                            txJabber = true;
                            // Jabber (over the WD limit): JT status, TS
                            // error completion, nothing on the wire.
                            const wb = new Uint8Array(4);
                            new DataView(wb.buffer).setUint32(0, (tdes0 & ~0x80000000) | 0x4000, true);
                            wuc.mem_write(BigInt(descAddr), wb);
                            try { eth_note_jabber(); } catch {}
                        } else if (bufAddr !== 0 && bufSize > 0 && bufSize <= jlim) {
                            captured = true;
                            lastTxLen = bufSize;
                            const pkt = new Uint8Array(wuc.mem_read(BigInt(bufAddr), bufSize));
                            // Checksum offload: TDES0 CIC[23:22] inserts IP
                            // (+TCP/UDP/ICMP) checksums before capture.
                            txInsertCsum(pkt, (at, v) => {
                                wuc.mem_write(BigInt(bufAddr + at), new Uint8Array([(v >> 8) & 0xFF, v & 0xFF]));
                            }, (tdes0 >>> 22) & 3);
                            // TX timestamp snapshot (TTSE + PTP TSE + event
                            // message filter): silicon snapshots only PTP
                            // event messages (Sync/Delay_Req/Pdelay_Req/
                            // Pdelay_Resp — ethertype 0x88F7, or UDP dport
                            // 319/320). A TTSE data frame (like the feat
                            // UDP probe) gets TTSS=0 and no TDES6/7 write —
                            // the old code stamped every TTSE frame.
                            if ((tdes0 & 0x02000000) && eth_ptp_tse() && ptpIsEvent(pkt)) {
                                const sb = new Uint8Array(8);
                                const sdv = new DataView(sb.buffer);
                                sdv.setUint32(0, eth_ptp_sec(), true);
                                sdv.setUint32(4, eth_ptp_sub(), true);
                                wuc.mem_write(BigInt(descAddr + 24), sb);
                                ttss = 0x20000; // TTSS status
                            }
                            // SARC (MACCR[29:28] insert/replace): the MAC stamps
                            // the station address into the on-wire frame
                            // (guest buffer keeps the original bytes).
                            try {
                                const sarc = eth_tx_sarc() >>> 0;
                                if ((sarc === 2 || sarc === 3) && pkt.length >= 12) {
                                    const sa = eth_station_addr();
                                    for (let i = 0; i < 6; i++) {
                                        pkt[6 + i] = Number((sa >> BigInt(8 * (5 - i))) & 0xFFn);
                                    }
                                }
                            } catch {}
                            // MAC loopback (LM): internal only, never on
                            // the wire; ROD filtering happens at accept().
                            if (eth_loopback_tx()) {
                                rxQueue.push(pkt.slice());
                                trimRxQueue();
                            } else if (onTx) {
                                onTx(pkt, { bufAddr, len: bufSize });
                            }
                        }
                        // Shared normal-completion writeback (NOT for jabber:
                        // the JT word above is final — falling through here
                        // would overwrite JT with a normal completion).
                        if (!txJabber) {
                        const wb = new Uint8Array(4);
                        // Single-node collision report: an armed collision
                        // ORs EC + CC=15 into the writeback, but only in
                        // half-duplex (silicon never collides full-duplex).
                        // NOTE: `txEc` is function-scope (declared with
                        // linkUp/captured above) — the MMC note below the
                        // block reads it. Do NOT re-scope it here (a
                        // block-local `let ec` throws ReferenceError at the
                        // note call, swallowed by its catch — that silenced
                        // ALL TX counting once before).
                        txEc = 0;
                        try {
                            // NOTE: (x & MASK) === 0 needs the inner parens —
                            // & binds looser than === in JS.
                            // Single take() call: it is one-shot, and even
                            // logging it would consume the arm.
                            const tk = eth_take_collision();
                            if (tk && ((eth_get_maccr() & 0x800) === 0)) txEc = 0x100 | (0xF << 3);
                        } catch {}
                        // CSMA/CD deferral: half-duplex TX while a receive
                        // still occupies the wire reports DB (TS normal).
                        try {
                            const df = eth_tx_deferred();
                            if (df) txEc |= 0x1;
                        } catch {}
                        new DataView(wb.buffer).setUint32(0, (tdes0 & ~0x80000000) | 0x20000000 | ttss | txEc, true);
                        wuc.mem_write(BigInt(descAddr), wb);
                        } // end if (!txJabber) normal writeback
                        } // end link-up else (normal + jabber writeback)
                        if (captured) {
                            try { eth_note_tx((txEc & 0x100) !== 0); } catch {}
                        }
                        eth_clear_tx_poll();
                        // Wire pacing: TS completion waits for the frame's
                        // wire time (normal path only — jabber/dead-wire
                        // use done_now below, empty never reaches here).
                        try { if (captured) eth_tx_wire_busy(lastTxLen); } catch {}
                        try {
                            if (txDeadWire || txJabber) eth_tx_done_now();
                            else eth_tx_done();
                        } catch { try { eth_tx_done(); } catch {} }
                        if (!irq_eth) wwrite32(E.irqFlag, wread32(E.irqFlag) | 1);
                    } // end OWN-bit branch (descAddr serviced)
                    else if (tdes0 & 0x80000000) {
                        // OWN set but empty (len 0 / null buffer): the
                        // guard above already completed it (OWN-clear +
                        // TS). Nothing more to do.
                        eth_clear_tx_poll();
                        return;
                    }
                    // OWN clear: stale re-poll of an already-completed
                    // descriptor. Drop the poll, complete nothing (the
                    // frame's TS already fired when OWN cleared).
                    eth_clear_tx_poll();
                    return;
                } // end descAddr !== 0
                // descAddr === 0 (poll armed before DMATDLAR programmed):
                // nothing to service; drop the poll.
                eth_clear_tx_poll();
                return;
            }
            if (eth_is_rx_poll() && rxQueue.length > 0) wDeliverRx(false);
            // Stale polls (armed, queue empty) are dropped: delivery then
            // requires a poll armed after the frame queued. All firmware
            // re-arms periodically while waiting, so nothing is lost — and
            // a synchronously-queued reply (e.g. a WOL trigger's magic)
            // can no longer be delivered before the guest sleeps on it
            // (pre-sleep delivery lets the guest consume the IRQ62 wake
            // before WFI, then it sleeps forever — observed STOP hang).
            else if (eth_is_rx_poll()) { try { eth_clear_rx_poll(); } catch {} }
        };
        const wIsPeriph = (a) => (a >= 0x40000000 && a < 0xB0000000) || (a >= 0xE0000000 && a < 0xE1000000);
        const wProcessDma = () => {
            const count = dma_get_pending_count();
            for (let i = 0; i < count; i++) {
                const pending = dma_get_pending(0);
                if (pending.length < 5) continue;
                const dir = pending[0], stream = pending[1], src = pending[2], dst = pending[3], size = pending[4];
                const peri_addr = pending[5] || 0;
                const peripheral = pending[6] || 0;
                try {
                    if (dir === 2 || !peripheral || !wIsPeriph(peri_addr)) {
                        wuc.mem_write(BigInt(dst), wuc.mem_read(BigInt(src), size));
                    } else if (dir === 0) {
                        const data = dma_periph_read(peri_addr, size, (pending[7] || 0) === 1, pending[8] || 4);
                        wuc.mem_write(BigInt(dst), data);
                    } else {
                        dma_periph_write(peri_addr, wuc.mem_read(BigInt(src), size));
                    }
                } catch (e) { /* ignore */ }
                dma_set_completed(stream, true);
            }
        };
        // Flash erase completion (restored post-§23: the imports survived the
        // rewrite but the drain didn't, stalling flash_test at BSY forever).
        // The model holds BSY until the driver applies the queued fill.
        const wProcessFlash = () => {
            try {
                const q = flash_take_erase();
                if (!q || q.length < 2) return;
                try { cpu.flash_fill_erase(q[0] >>> 0, q[1] >>> 0); } catch {}
                try { flash_erase_applied(); } catch {}
            } catch {}
        };
        // DMA2D Chrom-ART job: gather source lines from guest RAM, convert/
        // blend them through the model helpers, scatter output lines honoring
        // the OR line offsets, then complete (TCIF + IRQ56 when TCIE is set).
        const dma2dBpp = (cm) => cm === 0 ? 4 : cm === 1 ? 3 : 2;
        const wProcessDma2d = () => {
            let job;
            try { job = dma2d_take_job(); } catch { return; }
            if (!job || job.length < 16) return;
            const j = Array.from(job, (v) => v >>> 0);
            const [mode, w, h, fgA, fgCM, fgOff, bgA, bgCM, bgOff, outA, outCM, outOff, ocolr, fgAlpha] = j;
            try {
                const ob = dma2dBpp(outCM);
                if (mode === 0) {
                    // M2M: raw 32-bit word copy with line offsets.
                    for (let y = 0; y < h; y++) {
                        const src = wuc.mem_read(BigInt(((fgA + y * (w + fgOff) * 4) >>> 0)), w * 4);
                        wuc.mem_write(BigInt(((outA + y * (w + outOff) * 4) >>> 0)),
                            new Uint8Array(src.buffer, src.byteOffset, w * 4));
                    }
                } else if (mode === 3) {
                    // R2M: OCOLR is an ARGB8888 constant; convert it once to
                    // the output format, then fill every line.
                    const src = new Uint8Array([(ocolr & 255), ((ocolr >> 8) & 255), ((ocolr >> 16) & 255), ((ocolr >>> 24) & 255)]);
                    const px = dma2d_convert(0, outCM, 1, 1, src);
                    const line = new Uint8Array(w * ob);
                    for (let x = 0; x < w; x++) line.set(px, x * ob);
                    for (let y = 0; y < h; y++) {
                        wuc.mem_write(BigInt(((outA + y * (w + outOff) * ob) >>> 0)), line);
                    }
                } else if (mode === 1) {
                    // M2M+PFC: convert FG lines.
                    const fb = dma2dBpp(fgCM);
                    for (let y = 0; y < h; y++) {
                        const src = wuc.mem_read(BigInt(((fgA + y * (w + fgOff) * fb) >>> 0)), w * fb);
                        const out = dma2d_convert(fgCM, outCM, w, 1,
                            new Uint8Array(src.buffer, src.byteOffset, w * fb));
                        wuc.mem_write(BigInt(((outA + y * (w + outOff) * ob) >>> 0)), out);
                    }
                } else {
                    // M2M+blend: FG over BG, one line per convert call.
                    const fb = dma2dBpp(fgCM), bb = dma2dBpp(bgCM);
                    for (let y = 0; y < h; y++) {
                        const fgs = wuc.mem_read(BigInt(((fgA + y * (w + fgOff) * fb) >>> 0)), w * fb);
                        const bgs = wuc.mem_read(BigInt(((bgA + y * (w + bgOff) * bb) >>> 0)), w * bb);
                        const out = dma2d_blend(fgCM, bgCM, outCM, w, 1, fgAlpha,
                            new Uint8Array(fgs.buffer, fgs.byteOffset, w * fb),
                            new Uint8Array(bgs.buffer, bgs.byteOffset, w * bb));
                        wuc.mem_write(BigInt(((outA + y * (w + outOff) * ob) >>> 0)), out);
                    }
                }
            } catch {}
            try { dma2d_job_done(); } catch {}
        };
        return {
            uc: wuc,
            read32: wread32, write32: wwrite32,
            getRegisters: () => { const r = cpu.get_regs(); return { R0: r[0], R1: r[1], R2: r[2], R3: r[3], R4: r[4], R5: r[5], R6: r[6], R7: r[7], R8: r[8], R9: r[9], R10: r[10], R11: r[11], R12: r[12], SP: r[13], LR: r[14], PC: r[15], XPSR: cpu.get_xpsr() >>> 0 }; },
            // VFPv4-SP file visibility (debugger pokes; Dd aliases S(2d)/S(2d+1)).
            getFpuState: () => ({ s: Array.from(cpu.get_sregs(), (v) => v >>> 0), fpscr: cpu.get_fpscr() >>> 0 }),
            setSreg: (i, v) => { cpu.set_sreg(i >>> 0, v >>> 0); },
            setFpscr: (v) => { cpu.set_fpscr(v >>> 0); },
            step: (n = 100000) => {
                // WFI/WFE sleep: advance virtual time (which fires the RTC
                // alarm etc.), drain any queued RX frames (the wire
                // delivers even though the guest can't re-arm — this is
                // what lets a WOL magic packet wake STOP), then wake
                // when an interrupt is pending.
                if (typeof cpu.sleeping === 'function' && cpu.sleeping()) {
                    try { tick_n(120000); } catch {}
                    try { periph_read(0x40002800, 4); } catch {}
                    // Sleep drain: force-deliver (no poll needed — the
                    // guest can't re-arm while asleep). The stale-poll
                    // drop above guarantees nothing was delivered early,
                    // so the queued magic is still here for IRQ62.
                    try { wDeliverRx(true); } catch {}
                    if (has_pending_interrupt()) {
                        try {
                            if (((wread32(0xE000ED10) >>> 0) >> 2) & 1) pwr_wakeup();
                        } catch {}
                        try { cpu.wake(); } catch {}
                    } else {
                        instCount += 120000;
                        return { instCount, stopped: false, pc: cpu.get_pc() };
                    }
                }
                const c = cpu.step(n);
                instCount += c;
                // The core published its executed count itself mid-step, so
                // tick WITHOUT adding here (tick_n(c) would double-count).
                try { tick_peripherals(); } catch {}
                wProcessDma();
                wProcessFlash();
                wProcessDma2d();
                wProcessEth();
                try { processDevices(); } catch {}
                if (ENV.WASM_DBG && rxQueue.length > 0) console.log(`[wasm-step] pc=0x${(cpu.get_pc() >>> 0).toString(16)} rxpoll=${eth_is_rx_poll() ? 1 : 0} q=${rxQueue.length}`);
                if (is_watchdog_reset_requested()) cpu.reset_cpu(sp0, pc0 | 1);
                const faulted = cpu.fault_pc() !== 0xFFFFFFFF;
                return { instCount, stopped: faulted, pc: cpu.get_pc() };
            },
            drainUart: () => { try { return get_uart_output(); } catch { return ''; } },
            injectFrame: (frame) => { rxQueue.push(frame instanceof Uint8Array ? frame : new Uint8Array(frame)); trimRxQueue(); },
            // Inject a CAN frame from an external transmitter onto the shared bus.
            canInject: (id, dlc, data) => { can_inject(id & 0x7FF, dlc & 0xF, new Uint8Array(data)); },
            timInjectCapture: (name, ch) => { tim_inject_capture(name, ch & 0x3); },
            sendUart: (bytes) => {
                const arr = typeof bytes === 'string' ? [...bytes].map((c) => c.charCodeAt(0)) : [...bytes];
                for (const b of arr) { try { uart_rx_byte(uart_addr, b & 0xFF); } catch {} }
            },
            sendUartByte(b) { try { uart_rx_byte(uart_addr, b & 0xFF); } catch {} },
            rxQueue,
            pin, watchPin, i2cRegfile, setAdcChannel, clearAdcChannel,
            takeSpeakerSamples,
            traceStart: () => { try { cpu.trace_start(); } catch {} },
            traceStop: () => { try { cpu.trace_stop(); } catch {} },
            takeTrace: () => { try { return cpu.take_trace(); } catch { return []; } },
            oled: oled ? { fb: oled.fb, frame: () => oled.frame } : null,
            tft: tft ? { fb: tft.fb, w: tft.w, h: tft.h, frame: () => tft.frame } : null,
            buzzer: buzzer ? { get freq() { return buzzer.freq; }, get duty() { return buzzer.duty; }, get change() { return buzzer.change; } } : null,
            rtc: rtc ? { get time() { return rtc.time; }, get temp() { return rtc.temp; }, get change() { return rtc.change; } } : null,
            camera: {
                feed(w, h, pixels) {
                    dcmi_feed_frame(w, h, pixels instanceof Uint8Array ? pixels : Uint8Array.from(pixels));
                },
                stop() { cameraRunning = false; dcmi_clear(); },
                start() { cameraRunning = true; },
                get frames() { return cameraFrames; },
            },
            pushFsmcData(bank, values) { fsmc_push_data(bank, Uint32Array.from(values)); },
            takeFsmcEvents(bank) { return fsmc_take_events(bank); },
            dmaPendingCount() { return dma_get_pending_count(); },
            dmaSetCompleted(streamIdx, success) { dma_set_completed(streamIdx, !!success); },
            stop() { /* steps are bounded round-trips; nothing to interrupt */ },
            loadImage(opts = {}) {
                const flash = opts.flash || new Uint8Array(0);
                if (flash.length) cpu.load_firmware(flash, vector_table);
                for (const seg of opts.extraMem || []) cpu.load_firmware(seg.data, seg.addr);
                const sp = wread32(vector_table);
                const pc = wread32(vector_table + 4);
                try { cpu.wake(); } catch {}
                cpu.reset_cpu(sp, pc | 1);
                instCount = 0;
            },
            close: () => { try { cpu.free(); } catch {} },
            reset: () => { cpu.reset_cpu(sp0, pc0 | 1); },
            faultInfo: () => {
                const fpc = cpu.fault_pc() >>> 0;
                if (fpc === 0xFFFFFFFF) return null;
                return { pc: fpc, op1: cpu.fault_op1(), op2: cpu.fault_op2(), len: cpu.fault_len() };
            },
        };
    }
}
