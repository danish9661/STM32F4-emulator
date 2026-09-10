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
        is_watchdog_reset_requested, is_mpu_enabled, add_spi_flash, add_i2c_eeprom, qspi_register_flash, init_svd,
        eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll,
        eth_is_rx_poll, eth_get_rx_desc_addr, eth_clear_rx_poll, eth_tx_done, eth_rx_done,
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

    // IRQ-mode RX delivery: the guest owns its descriptor layout, so walk its
    // RX list from the model's poll address (DMARDLAR base) for the first
    // DMA-owned descriptor and deliver the frame there. Falls back to the
    // static E layout when no owned descriptor is found. The guest ISR scans
    // the list itself, so no idx/flag bookkeeping is needed.
    const injectRxIrq = (memWrite, memRead32, frame, len) => {
        let listBase = 0;
        try { listBase = eth_get_rx_desc_addr() >>> 0; } catch {}
        if (listBase !== 0) {
            for (let i = 0; i < 8; i++) {
                let rdes0 = 0, rdes1 = 0;
                try {
                    rdes0 = memRead32(listBase + i * 8) >>> 0;
                    rdes1 = memRead32(listBase + i * 8 + 4) >>> 0;
                } catch { break; }
                if ((rdes0 & 0x80000000) && rdes1 !== 0) {
                    try {
                        memWrite(BigInt(rdes1), frame.subarray(0, len));
                        const wb = new Uint8Array(4);
                        new DataView(wb.buffer).setUint32(0, len << 16, true);
                        memWrite(BigInt(listBase + i * 8), wb);
                        return;
                    } catch { return; }
                }
            }
        }
        const idx = E.rxInjectIdx;
        E.rxInjectIdx = (E.rxInjectIdx + 1) % E.rxDescs;
        memWrite(BigInt(E.rxBuf + idx * E.rxStride), frame.subarray(0, len));
        const wb = new Uint8Array(4);
        new DataView(wb.buffer).setUint32(0, len << 16, true);
        memWrite(BigInt(E.rxDesc + idx * 8), wb);
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
        // Driver-level halt reason (model-requested stops like MPU enable
        // that carry no CPU fault). Read via modelHaltInfo().
        let modelHalt = null;
        // Compact mirrors of processEth/processDma for the wasm memory.
        const wProcessEth = () => {
            if (eth_is_tx_poll()) {
                const descAddr = eth_get_tx_desc_addr();
                if (ENV.WASM_DBG) console.log(`[wasm-tx] poll desc=0x${descAddr.toString(16)}`);
                if (descAddr !== 0) {
                    const desc = wuc.mem_read(BigInt(descAddr), 8);
                    const dv = new DataView(desc.buffer, desc.byteOffset, desc.byteLength);
                    const tdes0 = dv.getUint32(0, true);
                    const tdes1 = dv.getUint32(4, true);
                    if (tdes0 & 0x80000000) {
                        const bufAddr = tdes1 & 0xFFFFFFFC;
                        const bufSize = tdes0 & 0x3FFF;
                        if (ENV.WASM_DBG) console.log(`[wasm-tx] tdes0=0x${tdes0.toString(16)} buf=0x${bufAddr.toString(16)} len=${bufSize}`);
                        if (bufAddr !== 0 && bufSize > 0 && bufSize <= 2000) {
                            const pkt = new Uint8Array(wuc.mem_read(BigInt(bufAddr), bufSize));
                            if (onTx) onTx(pkt, { bufAddr, len: bufSize });
                        }
                        const wb = new Uint8Array(4);
                        new DataView(wb.buffer).setUint32(0, (tdes0 & ~0x80000000) | 0x20000000, true);
                        wuc.mem_write(BigInt(descAddr), wb);
                    }
                }
                eth_clear_tx_poll();
                eth_tx_done();
                if (!irq_eth) wwrite32(E.irqFlag, wread32(E.irqFlag) | 1);
            }
            if (eth_is_rx_poll() && rxQueue.length > 0) {
                if (ENV.WASM_DBG) console.log(`[wasm-rx] inject idx=${E.rxInjectIdx} q=${rxQueue.length} poll=1`);
                const frame = rxQueue.shift();
                const len = Math.min(frame.length, E.rxStride);
                if (!irq_eth) {
                    const idx = E.rxInjectIdx;
                    E.rxInjectIdx = (E.rxInjectIdx + 1) % E.rxDescs;
                    const descAddr = E.rxDesc + idx * 8;
                    const bufAddr = E.rxBuf + idx * E.rxStride;
                    wuc.mem_write(BigInt(bufAddr), frame.subarray(0, len));
                    const wb = new Uint8Array(4);
                    new DataView(wb.buffer).setUint32(0, len << 16, true);
                    wuc.mem_write(BigInt(descAddr), wb);
                    wwrite32(E.rxFrameIdx, idx);
                    wwrite32(E.rxFrameLen, len);
                    wwrite32(E.irqFlag, wread32(E.irqFlag) | 2);
                } else {
                    // IRQ-driven firmware owns its descriptor layout: walk the
                    // guest RX list from the model's poll address for the first
                    // DMA-owned descriptor and deliver there (its ISR scans).
                    injectRxIrq((a, d) => wuc.mem_write(a, d), wread32, frame, len);
                }
                eth_clear_rx_poll();
                eth_rx_done();
            }
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
                // alarm etc.), then wake when an interrupt is pending.
                if (typeof cpu.sleeping === 'function' && cpu.sleeping()) {
                    try { tick_n(120000); } catch {}
                    try { periph_read(0x40002800, 4); } catch {}
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
                wProcessEth();
                try { processDevices(); } catch {}
                if (ENV.WASM_DBG && rxQueue.length > 0) console.log(`[wasm-step] pc=0x${(cpu.get_pc() >>> 0).toString(16)} rxpoll=${eth_is_rx_poll() ? 1 : 0} q=${rxQueue.length}`);
                if (is_watchdog_reset_requested()) cpu.reset_cpu(sp0, pc0 | 1);
                // MPU enabled but protection unmodeled: halt loudly with the
                // reason on the handle (running on would silently skip the
                // MemManage faults the guest expects).
                if (typeof is_mpu_enabled === 'function' && is_mpu_enabled()) {
                    modelHalt = 'MPU_CTRL.ENABLE set but MPU protection is not modeled';
                    return { instCount, stopped: true, pc: cpu.get_pc() };
                }
                const faulted = cpu.fault_pc() !== 0xFFFFFFFF;
                return { instCount, stopped: faulted, pc: cpu.get_pc() };
            },
            drainUart: () => { try { return get_uart_output(); } catch { return ''; } },
            injectFrame: (frame) => { rxQueue.push(frame instanceof Uint8Array ? frame : new Uint8Array(frame)); },
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
                modelHalt = null;
            },
            close: () => { try { cpu.free(); } catch {} },
            reset: () => { modelHalt = null; cpu.reset_cpu(sp0, pc0 | 1); },
            faultInfo: () => {
                const fpc = cpu.fault_pc() >>> 0;
                if (fpc === 0xFFFFFFFF) return null;
                return { pc: fpc, op1: cpu.fault_op1(), op2: cpu.fault_op2(), len: cpu.fault_len() };
            },
            // Model-requested halt reason (e.g. MPU enabled but unmodeled),
            // or null when running clean. Complements faultInfo().
            modelHaltInfo: () => modelHalt,
        };
    }
}
