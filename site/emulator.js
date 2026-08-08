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
    } = opts;

    const {
        periph_read, periph_write, tick, tick_n, get_uart_output,
        dma_get_pending_count, dma_get_pending, dma_set_completed,
        is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, init_svd,
        eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll,
        eth_is_rx_poll, eth_clear_rx_poll, eth_tx_done, eth_rx_done,
        get_next_pending_interrupt, uart_rx_byte,
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
    if (wasmInit) await bindings.default({ module_or_path: wasmInit });
    else await bindings.default();
    for (const cfg of (ext_devices.spi_flash || [])) {
        add_spi_flash(cfg.peripheral, cfg.jedec_id, cfg.data, cfg.cs ?? null);
    }
    for (const cfg of (ext_devices.i2c_eeprom || [])) {
        add_i2c_eeprom(cfg.peripheral, cfg.address, cfg.data);
    }
    init_svd(svdXml);
    bindings.init();

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    uc.mem_map(vector_table & ~0x1FFFF, flash_size, Module.PROT_ALL);
    uc.mem_write(BigInt(vector_table), firmware);
    uc.mem_map(0x20000000, ram_size, Module.PROT_ALL);
    for (const seg of extra_mem) {
        uc.mem_write(BigInt(seg.addr), seg.data);
    }

    const periphRanges = [
        [0x40000000, 0xB0000000],
        [0xE0000000, 0xE1000000],
    ];
    for (const [start, end] of periphRanges) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
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
        periph_write(Number(address), size, Number(value));
    };
    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

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
            if (dma_get_pending_count() > 0 || eth_is_tx_poll()) {
                uc.emu_stop();
                return;
            }
        }
    };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

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
            write32(E.irqFlag, read32(E.irqFlag) | 1);
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
            new DataView(wb.buffer).setUint32(0, (1 << 28) | (1 << 27) | (len << 16), true);
            uc.mem_write(BigInt(descAddr), wb);
            write32(E.rxFrameIdx, idx);
            write32(E.rxFrameLen, len);
            write32(E.irqFlag, read32(E.irqFlag) | 2);
            eth_clear_rx_poll();
            eth_rx_done();
        }
    };

    const processDma = () => {
        const count = dma_get_pending_count();
        for (let i = 0; i < count; i++) {
            const pending = dma_get_pending(0);
            if (pending.length < 5) continue;
            const dir = pending[0], stream = pending[1], src = pending[2], dst = pending[3], size = pending[4];
            const peri_addr = pending[5] || 0;
            const peripheral = pending[6] || 0;
            try {
                if (dir === 2) { // memcpy
                    uc.mem_write(BigInt(dst), uc.mem_read(BigInt(src), size));
                } else if (dir === 0) { // read
                    const data = uc.mem_read(BigInt(src), size);
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            let val = 0;
                            for (let k = 0; k < chunk; k++) val |= data[j + k] << (k * 8);
                            periph_write(peri_addr, chunk, val);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), data);
                    }
                } else if (dir === 1) { // write
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            const val = periph_read(peri_addr, chunk) >>> 0;
                            const bytes = new Uint8Array(chunk);
                            for (let k = 0; k < chunk; k++) bytes[k] = (val >> (k * 8)) & 0xFF;
                            uc.mem_write(BigInt(dst + j), bytes);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), uc.mem_read(BigInt(src), size));
                    }
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
        const current_pc = uc.reg_read_i32(Module.ARM_REG_PC);
        uc.emu_start(BigInt(current_pc | 1), 0n, 0n, max_inst);
        tick();
        processDma();
        processEth();
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
