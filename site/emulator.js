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
        maxBatch = 100000,    // emu_start instruction budget per step()
        onTx = null,          // (frame: Uint8Array, meta) called per TX capture
        eth = {},             // firmware-specific SRAM addresses (defaults above)
        ext_devices = {},
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
    } = opts;

    const {
        periph_read, periph_write, tick, tick_n, get_uart_output,
        dma_get_pending_count, dma_get_pending, dma_set_completed,
        dma_periph_read, dma_periph_write,
        is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, init_svd,
        eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll,
        eth_is_rx_poll, eth_clear_rx_poll, eth_tx_done, eth_rx_done,
        get_next_pending_interrupt, uart_rx_byte,
        flash_is_programming, flash_take_erase, flash_erase_applied,
        spi_tap, spi_take_events, i2c_register_slave, i2c_take_events,
        audio_take_capture,
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
    for (const cfg of (ext_devices.spi_flash || [])) {
        add_spi_flash(cfg.peripheral, cfg.jedec_id, cfg.data, cfg.cs ?? null);
    }
    for (const cfg of (ext_devices.i2c_eeprom || [])) {
        add_i2c_eeprom(cfg.peripheral, cfg.address, cfg.data);
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
    init_svd(svdXml);
    bindings.init();

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
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

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const val = periph_read(Number(address), size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) bytes[i] = (val >> (i * 8)) & 0xFF;
        uc.mem_write(address, bytes);
    };
    const memWriteHook = (handle, type, address, size, value, user_data) => {
        const a = Number(address);
        periph_write(a, size, Number(value));
        if (a >= 0x40023C00 && a <= 0x40023C18) syncFlashProtection(); // FLASH CR writes flip pg
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
    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    let stopRequested = false;
    let instCount = 0;
    let tickAcc = 0;
    let pollAcc = 0;
    const rxQueue = [];

    const codeHook = (handle, address, size, user_data) => {
        instCount++;
        if (rxQueue.length > 0 && eth_is_rx_poll()) {
            uc.emu_stop();
            return;
        }
        tickAcc++;
        if (tickAcc >= tickEvery) {
            tickAcc = 0;
            tick_n(tickEvery);
            if (is_watchdog_reset_requested()) {
                stopRequested = true;
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
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

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

    const processDevices = () => {
        processOled();
        processTft();
        processBuzzer();
        processSpeaker();
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

    const isPeriphAddr = (a) => a >= 0x40000000 && a < 0x50000000;
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
            if (irq <= -100) break;
            const savedAt = uc.reg_read_i32(Module.ARM_REG_SP);
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
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt - 32);
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
            try {
                uc.emu_start(BigInt(handler_pc), 0n, 0n, 100000);
            } catch (e) {
                // handler aborted on bx lr (EXC_RETURN not supported) — expected
            }
            const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
            const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
            uc.reg_write_i32(Module.ARM_REG_XPSR, savedSv.getUint32(0, true));
            uc.reg_write_i32(Module.ARM_REG_R0, savedSv.getUint32(28, true));
            uc.reg_write_i32(Module.ARM_REG_R1, savedSv.getUint32(24, true));
            uc.reg_write_i32(Module.ARM_REG_R2, savedSv.getUint32(20, true));
            uc.reg_write_i32(Module.ARM_REG_R3, savedSv.getUint32(16, true));
            uc.reg_write_i32(Module.ARM_REG_R12, savedSv.getUint32(12, true));
            uc.reg_write_i32(Module.ARM_REG_LR, savedSv.getUint32(8, true));
            uc.reg_write_i32(Module.ARM_REG_PC, savedSv.getUint32(4, true) | 1);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt);
            processDma();
            processEth();
        }
    };

    const step = (max_inst = maxBatch) => {
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
            } else throw e;
        }
        tick();
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
        sendUartByte(b) { return uart_rx_byte(uart_addr, b & 0xFF); },
        sendUart(bytes) {
            for (const b of bytes) uart_rx_byte(uart_addr, b & 0xFF);
        },
        rxQueue,
        oled: oled ? { fb: oled.fb, frame: () => oled.frame } : null,
        tft: tft ? { fb: tft.fb, w: tft.w, h: tft.h, frame: () => tft.frame } : null,
        buzzer: buzzer ? { get freq() { return buzzer.freq; }, get duty() { return buzzer.duty; }, get change() { return buzzer.change; } } : null,
        takeSpeakerSamples,
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
