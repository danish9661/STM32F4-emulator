/* @ts-self-types="./stm32_periph_wasm.d.ts" */

export class WasmCpu {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        WasmCpuFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_wasmcpu_free(ptr, 0);
    }
    /**
     * @returns {number}
     */
    fault_len() {
        const ret = wasm.wasmcpu_fault_len(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Packed fault detail: op1 | op2<<16 | len<<... (see fault_op2/fault_len).
     * @returns {number}
     */
    fault_op1() {
        const ret = wasm.wasmcpu_fault_op1(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    fault_op2() {
        const ret = wasm.wasmcpu_fault_op2(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Fault program counter, or 0xFFFF_FFFF when running clean.
     * @returns {number}
     */
    fault_pc() {
        const ret = wasm.wasmcpu_fault_pc(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Fill a flash range with 0xFF (erase applied by the JS flash driver
     * after `flash_take_erase`; clamped to mapped flash, then completed
     * with `flash_erase_applied`). Debugger/firmware images use `load_firmware`.
     * @param {number} start
     * @param {number} len
     */
    flash_fill_erase(start, len) {
        wasm.wasmcpu_flash_fill_erase(this.__wbg_ptr, start, len);
    }
    /**
     * FPSCR (cumulative flags, RMode, FZ/DN).
     * @returns {number}
     */
    get_fpscr() {
        const ret = wasm.wasmcpu_get_fpscr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get_ipsr() {
        const ret = wasm.wasmcpu_get_ipsr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get_pc() {
        const ret = wasm.wasmcpu_get_pc(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {number}
     */
    get_primask() {
        const ret = wasm.wasmcpu_get_primask(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @returns {Uint32Array}
     */
    get_regs() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcpu_get_regs(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get_sp() {
        const ret = wasm.wasmcpu_get_sp(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Raw S0-S31 file (f32 bits; Dd aliases S(2d)/S(2d+1)). Debugger and
     * driver visibility for the VFPv4-SP unit (nothing else reads these).
     * @returns {Uint32Array}
     */
    get_sregs() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcpu_get_sregs(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @returns {number}
     */
    get_xpsr() {
        const ret = wasm.wasmcpu_get_xpsr(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Load firmware bytes (writes through flash protection).
     * @param {Uint8Array} data
     * @param {number} base
     */
    load_firmware(data, base) {
        const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasmcpu_load_firmware(this.__wbg_ptr, ptr0, len0, base);
    }
    /**
     * Last unmapped-memory access address, or 0xFFFF_FFFF when none.
     * @returns {number}
     */
    mem_fault() {
        const ret = wasm.wasmcpu_mem_fault(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {number} addr
     * @param {number} len
     * @returns {Uint8Array}
     */
    mem_read(addr, len) {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcpu_mem_read(retptr, this.__wbg_ptr, addr, len);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU8FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 1, 1);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * @param {number} addr
     * @param {Uint8Array} data
     */
    mem_write(addr, data) {
        const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.wasmcpu_mem_write(this.__wbg_ptr, addr, ptr0, len0);
    }
    /**
     * @param {number} sp
     * @param {number} pc
     * @param {number} flash_size
     * @param {number} ram_size
     */
    constructor(sp, pc, flash_size, ram_size) {
        const ret = wasm.wasmcpu_new(sp, pc, flash_size, ram_size);
        this.__wbg_ptr = ret;
        WasmCpuFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @param {number} addr
     * @returns {number}
     */
    read32(addr) {
        const ret = wasm.wasmcpu_read32(this.__wbg_ptr, addr);
        return ret >>> 0;
    }
    /**
     * @param {number} addr
     * @returns {number}
     */
    read8(addr) {
        const ret = wasm.wasmcpu_read8(this.__wbg_ptr, addr);
        return ret;
    }
    /**
     * @param {number} sp
     * @param {number} pc
     */
    reset_cpu(sp, pc) {
        wasm.wasmcpu_reset_cpu(this.__wbg_ptr, sp, pc);
    }
    /**
     * Enable/disable inline guest exception delivery (NVIC SysTick, ETH,
     * USART RX, SVC, PendSV...). Off by default (polling-only: pending
     * model IRQs never stop execution).
     * @param {boolean} v
     */
    set_deliver_irqs(v) {
        wasm.wasmcpu_set_deliver_irqs(this.__wbg_ptr, v);
    }
    /**
     * Debugger poke for FPSCR, under the same write mask the guest VMSR
     * uses (NZCVQC + AHP/DN/FZ/RMode + enables/flags; STRIDE/LEN/reserved
     * stay zero).
     * @param {number} v
     */
    set_fpscr(v) {
        wasm.wasmcpu_set_fpscr(this.__wbg_ptr, v);
    }
    /**
     * Debugger poke for one S register (out of range is ignored).
     * @param {number} i
     * @param {number} v
     */
    set_sreg(i, v) {
        wasm.wasmcpu_set_sreg(this.__wbg_ptr, i, v);
    }
    /**
     * True while halted in WFI/WFE (low-power). The driver advances virtual
     * time and calls `wake()` once an interrupt is pending.
     * @returns {boolean}
     */
    sleeping() {
        const ret = wasm.wasmcpu_sleeping(this.__wbg_ptr);
        return ret !== 0;
    }
    /**
     * @param {number} budget
     * @returns {number}
     */
    step(budget) {
        const ret = wasm.wasmcpu_step(this.__wbg_ptr, budget);
        return ret >>> 0;
    }
    /**
     * @returns {Uint32Array}
     */
    take_trace() {
        try {
            const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
            wasm.wasmcpu_take_trace(retptr, this.__wbg_ptr);
            var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
            var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
            var v1 = getArrayU32FromWasm0(r0, r1).slice();
            wasm.__wbindgen_export(r0, r1 * 4, 4);
            return v1;
        } finally {
            wasm.__wbindgen_add_to_stack_pointer(16);
        }
    }
    /**
     * PC-trace control for differential debugging (see cpu::trace_*).
     */
    trace_start() {
        wasm.wasmcpu_trace_start(this.__wbg_ptr);
    }
    trace_stop() {
        wasm.wasmcpu_trace_stop(this.__wbg_ptr);
    }
    wake() {
        wasm.wasmcpu_wake(this.__wbg_ptr);
    }
    /**
     * @param {number} addr
     * @param {number} v
     */
    write32(addr, v) {
        wasm.wasmcpu_write32(this.__wbg_ptr, addr, v);
    }
    /**
     * @param {number} addr
     * @param {number} v
     */
    write8(addr, v) {
        wasm.wasmcpu_write8(this.__wbg_ptr, addr, v);
    }
}
if (Symbol.dispose) WasmCpu.prototype[Symbol.dispose] = WasmCpu.prototype.free;

/**
 * Remove a channel override, reverting it to the synthetic default.
 * @param {string} peripheral
 * @param {number} channel
 */
export function adc_clear_channel_value(peripheral, channel) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.adc_clear_channel_value(ptr0, len0, channel);
}

/**
 * Whether the last ADC CDR read returned a latched simultaneous pair
 * (dual regular-simultaneous mode with both sides conversion-ready).
 * Harness scope probe for dual-mode simultaneity.
 * @returns {boolean}
 */
export function adc_dual_latched() {
    const ret = wasm.adc_dual_latched();
    return ret !== 0;
}

/**
 * Force an ADC channel's next conversion(s) to return `value` (clamped to
 * 12-bit) instead of the synthetic temp/vref/vbat/random default. Unlike
 * spi_tap/i2c_register_slave this can be called any time, including after
 * init() — it's a global override table, not per-instance device wiring
 * (see docs/components.md).
 * @param {string} peripheral
 * @param {number} channel
 * @param {number} value
 */
export function adc_set_channel_value(peripheral, channel, value) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.adc_set_channel_value(ptr0, len0, channel, value);
}

/**
 * Drain ADC samples staged by EOC-triggered DMA requests (CR2 DMA bit).
 * Each entry is one 12-bit conversion result, oldest first; empty when no
 * conversion with DMA enabled has completed since the last drain. The JS
 * DMA driver calls this after servicing a DMA stream aimed at an ADC DR.
 * @returns {Uint16Array}
 */
export function adc_take_dma() {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.adc_take_dma(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU16FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 2, 2);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * @param {string} peripheral
 * @param {number} address
 * @param {Uint8Array} data
 */
export function add_i2c_eeprom(peripheral, address, data) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    wasm.add_i2c_eeprom(ptr0, len0, address, ptr1, len1);
}

/**
 * Register a software SPI device. Must be called before init().
 * @param {string} name
 * @param {string | null | undefined} cs
 * @param {string} clk
 * @param {string} miso
 * @param {string} mosi
 */
export function add_software_spi(name, cs, clk, miso, mosi) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    var len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(clk, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len2 = WASM_VECTOR_LEN;
    const ptr3 = passStringToWasm0(miso, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len3 = WASM_VECTOR_LEN;
    const ptr4 = passStringToWasm0(mosi, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len4 = WASM_VECTOR_LEN;
    wasm.add_software_spi(ptr0, len0, ptr1, len1, ptr2, len2, ptr3, len3, ptr4, len4);
}

/**
 * Add an SPI flash device. Must be called before init().
 * @param {string} peripheral
 * @param {number} jedec_id
 * @param {Uint8Array} data
 * @param {string | null} [cs]
 */
export function add_spi_flash(peripheral, jedec_id, data, cs) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    var len2 = WASM_VECTOR_LEN;
    wasm.add_spi_flash(ptr0, len0, jedec_id, ptr1, len1, ptr2, len2);
}

/**
 * Reset the audio source and capture FIFO.
 */
export function audio_clear() {
    wasm.audio_clear();
}

/**
 * Load a WAV file (PCM 16-bit) as the I2S/SAI sample source. DR reads then
 * consume samples from it. Returns an error string on malformed input.
 * @param {Uint8Array} bytes
 */
export function audio_load_wav(bytes) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.audio_load_wav(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        if (r1) {
            throw takeObject(r0);
        }
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Remaining source samples (0 when no WAV is loaded or it is exhausted).
 * @returns {number}
 */
export function audio_source_remaining() {
    const ret = wasm.audio_source_remaining();
    return ret >>> 0;
}

/**
 * Drain the I2S/SAI TX capture FIFO (all DR writes since the last call).
 * @returns {Uint16Array}
 */
export function audio_take_capture() {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.audio_take_capture(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU16FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 2, 2);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Read one CAN FD payload byte for (`base`, `fifo`, `slot`, `idx`) —
 * the harness path into the FD window (firmware uses the MMIO window).
 * @param {number} base
 * @param {number} fifo
 * @param {number} slot
 * @param {number} idx
 * @returns {number}
 */
export function can_fd_byte(base, fifo, slot, idx) {
    const ret = wasm.can_fd_byte(base, fifo, slot, idx);
    return ret;
}

/**
 * FDCAN wire-time cost (virtual instructions) for a frame: arbitration
 * at the nominal rate, FD payload at the data rate iff BRS. Firmware
 * pacing TX-to-TX gaps observes exactly this contract.
 * @param {number} base
 * @param {boolean} fd
 * @param {boolean} brs
 * @param {number} payload_bytes
 * @returns {bigint}
 */
export function can_fd_cost(base, fd, brs, payload_bytes) {
    const ret = wasm.can_fd_cost(base, fd, brs, payload_bytes);
    return BigInt.asUintN(64, ret);
}

/**
 * Valid CAN FD payload length for (`base`, `fifo`, `slot`).
 * @param {number} base
 * @param {number} fifo
 * @param {number} slot
 * @returns {number}
 */
export function can_fd_len(base, fifo, slot) {
    const ret = wasm.can_fd_len(base, fifo, slot);
    return ret;
}

/**
 * Inject a CAN frame from an external transmitter onto the shared bus. The
 * frame is delivered to every CAN node (CAN1/CAN2) whose accept filters pass
 * it, so the guest sees it exactly as if another node sent it. `data` is up
 * to 8 bytes; `dlc` caps the length. Standard 11-bit frames.
 * @param {number} id
 * @param {number} dlc
 * @param {Uint8Array} data
 */
export function can_inject(id, dlc, data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.can_inject(id, dlc, ptr0, len0);
}

/**
 * Inject a CAN FD frame (FDF set, up to 64 bytes) from an external
 * transmitter. Same filter + FIFO path as classic (arbitration on the ID
 * is identical); the payload lands in the FD window (`can_fd_read`),
 * TDTR reports DLC + FDF. `brs` marks bit-rate-switch (flag only).
 * @param {number} id
 * @param {Uint8Array} data
 * @param {boolean} brs
 */
export function can_inject_fd(id, data, brs) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.can_inject_fd(id, ptr0, len0, brs);
}

/**
 * Harness = the wire fault: one error event on the CAN node at `base`
 * (TEC +8, LEC latched 0-7; BOFF at TEC > 255). `recover=true` models
 * 128x11 recessive bits (counters + LEC cleared, bus recovered).
 * @param {number} base
 * @param {number} lec
 * @param {boolean} recover
 */
export function can_note_error(base, lec, recover) {
    wasm.can_note_error(base, lec, recover);
}

export function clear_watchdog_reset_flags() {
    wasm.clear_watchdog_reset_flags();
}

/**
 * Forget any fed frame (stop the camera).
 */
export function dcmi_clear() {
    wasm.dcmi_clear();
}

/**
 * Provide the next camera frame to the DCMI controller (8-bit pixels,
 * row-major, width x height). The next CAPTURE start consumes it.
 * @param {number} w
 * @param {number} h
 * @param {Uint8Array} pixels
 */
export function dcmi_feed_frame(w, h, pixels) {
    const ptr0 = passArray8ToWasm0(pixels, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.dcmi_feed_frame(w, h, ptr0, len0);
}

/**
 * Harness = the camera sync lines: drive VSYNC/HSYNC levels and the PCLK
 * divider. A rising VSYNC edge loads an armed capture (CAPTURE set, frame
 * fed); HSYNC low holds pixels (horizontal blanking); PCLK div scales
 * pixels-per-tick (16/div, min 1). Defaults (true, true, 1) = free-run.
 * @param {boolean} vsync
 * @param {boolean} hsync
 * @param {number} pclk_div
 */
export function dcmi_set_sync(vsync, hsync, pclk_div) {
    wasm.dcmi_set_sync(vsync, hsync, pclk_div);
}

/**
 * Blend FG over BG ("over" operator) into the output mode. `fg_alpha`
 * supplies the alpha for formats without one (FGPFCCR.ALPHA).
 * @param {number} fg_cm
 * @param {number} bg_cm
 * @param {number} out_cm
 * @param {number} _w
 * @param {number} _h
 * @param {number} fg_alpha
 * @param {Uint8Array} fg
 * @param {Uint8Array} bg
 * @returns {Uint8Array}
 */
export function dma2d_blend(fg_cm, bg_cm, out_cm, _w, _h, fg_alpha, fg, bg) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passArray8ToWasm0(fg, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray8ToWasm0(bg, wasm.__wbindgen_export2);
        const len1 = WASM_VECTOR_LEN;
        wasm.dma2d_blend(retptr, fg_cm, bg_cm, out_cm, _w, _h, fg_alpha, ptr0, len0, ptr1, len1);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v3 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v3;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Convert a line-packed pixel buffer between color modes
 * (0 ARGB8888, 1 RGB888, 2 RGB565). Pure function of its inputs.
 * @param {number} fg_cm
 * @param {number} out_cm
 * @param {number} _w
 * @param {number} _h
 * @param {Uint8Array} px
 * @returns {Uint8Array}
 */
export function dma2d_convert(fg_cm, out_cm, _w, _h, px) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passArray8ToWasm0(px, wasm.__wbindgen_export2);
        const len0 = WASM_VECTOR_LEN;
        wasm.dma2d_convert(retptr, fg_cm, out_cm, _w, _h, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v2 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Complete the staged DMA2D transfer (TCIF + IRQ56 when TCIE is set).
 */
export function dma2d_job_done() {
    wasm.dma2d_job_done();
}

/**
 * Take the staged DMA2D transfer for the JS driver: 16 words
 * [mode, w, h, fg_addr, fg_cm, fg_off, bg_addr, bg_cm, bg_off,
 *  out_addr, out_cm, out_off, ocolr, 0, 0, 0], or empty when idle.
 * The driver gathers source lines, converts/blends them, scatters the
 * output lines (honoring the OR line offsets), then calls dma2d_job_done.
 * @returns {Uint32Array}
 */
export function dma2d_take_job() {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.dma2d_take_job(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * @param {number} index
 * @returns {Uint32Array}
 */
export function dma_get_pending(index) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.dma_get_pending(retptr, index);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * @returns {number}
 */
export function dma_get_pending_count() {
    const ret = wasm.dma_get_pending_count();
    return ret >>> 0;
}

/**
 * DMA peripheral-side chunked read: read `size` bytes from peripheral
 * `addr` (4-byte-aligned, tail chunk partial). Replaces the JS per-chunk
 * periph_read loop (one WASM call instead of size/4).
 * @param {number} addr
 * @param {number} size
 * @param {boolean} pinc
 * @param {number} psize
 * @returns {Uint8Array}
 */
export function dma_periph_read(addr, size, pinc, psize) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.dma_periph_read(retptr, addr, size, pinc, psize);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * DMA peripheral-side chunked write: write `bytes` to peripheral `addr` in
 * 4-byte chunks (tail chunk partial). Replaces the JS per-chunk periph_write
 * loop (one WASM call instead of size/4).
 * @param {number} addr
 * @param {Uint8Array} bytes
 */
export function dma_periph_write(addr, bytes) {
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.dma_periph_write(addr, ptr0, len0);
}

/**
 * @param {number} stream_idx
 * @param {boolean} success
 */
export function dma_set_completed(stream_idx, success) {
    wasm.dma_set_completed(stream_idx, success);
}

/**
 * Arm a single-node collision for the next TX completion (consumed once;
 * the driver reports EC + CC=15 when the MAC is half-duplex).
 */
export function eth_arm_collision() {
    wasm.eth_arm_collision();
}

/**
 * Wake-on-LAN inspection of a received frame. Returns bit 0 on a magic
 * packet (latches MPR when MPE is set, pends IRQ 62 when PMTIM is set).
 * Wakeup-frame CRC matching is not modeled (RWKPR never sets).
 * @param {Uint8Array} frame
 * @returns {number}
 */
export function eth_check_wol(frame) {
    const ptr0 = passArray8ToWasm0(frame, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.eth_check_wol(ptr0, len0);
    return ret >>> 0;
}

/**
 * Clear the RX poll flag (call after processing descriptors).
 */
export function eth_clear_rx_poll() {
    wasm.eth_clear_rx_poll();
}

/**
 * Clear the TX poll flag (call after processing descriptors).
 */
export function eth_clear_tx_poll() {
    wasm.eth_clear_tx_poll();
}

/**
 * Forward checksum-bad frames (FEF) or drop-disable (DTCEFD); else drop.
 * @returns {boolean}
 */
export function eth_fwd_csum_bad() {
    const ret = wasm.eth_fwd_csum_bad();
    return ret !== 0;
}

/**
 * Current MACCR (FES/DM/LM/ROD checks for pacing + loopback).
 * @returns {number}
 */
export function eth_get_maccr() {
    const ret = wasm.eth_get_maccr();
    return ret >>> 0;
}

/**
 * Get the RX descriptor list address for the current poll.
 * @returns {number}
 */
export function eth_get_rx_desc_addr() {
    const ret = wasm.eth_get_rx_desc_addr();
    return ret >>> 0;
}

/**
 * Get the TX descriptor list address for the current poll.
 * @returns {number}
 */
export function eth_get_tx_desc_addr() {
    const ret = wasm.eth_get_tx_desc_addr();
    return ret >>> 0;
}

/**
 * IPCO (MACCR[10]) gates the RX checksum status.
 * @returns {boolean}
 */
export function eth_ipco_on() {
    const ret = wasm.eth_ipco_on();
    return ret !== 0;
}

/**
 * Check if an Ethernet RX poll is pending (firmware wants to receive a packet).
 * @returns {boolean}
 */
export function eth_is_rx_poll() {
    const ret = wasm.eth_is_rx_poll();
    return ret !== 0;
}

/**
 * Check if an Ethernet TX poll is pending (firmware wants to send a packet).
 * @returns {boolean}
 */
export function eth_is_tx_poll() {
    const ret = wasm.eth_is_tx_poll();
    return ret !== 0;
}

/**
 * Wire link currently up.
 * @returns {boolean}
 */
export function eth_link_up() {
    const ret = wasm.eth_link_up();
    return ret !== 0;
}

/**
 * Loopback active (MACCR LM). The driver re-injects TX into RX.
 * @returns {boolean}
 */
export function eth_loopback_tx() {
    const ret = wasm.eth_loopback_tx();
    return ret !== 0;
}

/**
 * MAC accept filtering for a received frame (perfect slots + hash table +
 * broadcast/multicast/promiscuous + VLAN tag). The driver drops rejected
 * frames before writing any descriptor.
 * @param {Uint8Array} frame
 * @returns {boolean}
 */
export function eth_mac_accept(frame) {
    const ptr0 = passArray8ToWasm0(frame, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.eth_mac_accept(ptr0, len0);
    return ret !== 0;
}

/**
 * TX jabber completion (over the WD limit): TJTS.
 */
export function eth_note_jabber() {
    wasm.eth_note_jabber();
}

/**
 * RX queue-full drop: missed-frame counter + ROS.
 */
export function eth_note_missed() {
    wasm.eth_note_missed();
}

/**
 * Accepted-RX delivery for the MMC good-unicast counter.
 * @param {Uint8Array} frame
 */
export function eth_note_rx(frame) {
    const ptr0 = passArray8ToWasm0(frame, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.eth_note_rx(ptr0, len0);
}

/**
 * Delivery deferred on a CPU-owned head (silicon RBUS).
 */
export function eth_note_rx_stall() {
    wasm.eth_note_rx_stall();
}

/**
 * MMC counting hooks for TX completions / accepted RX / queue-full
 * drops, plus RX-stall (RBUS) and jabber (TJTS) status.
 * @param {boolean} collided
 */
export function eth_note_tx(collided) {
    wasm.eth_note_tx(collided);
}

/**
 * RX flow-control step: true when the frame is a pause frame for us
 * (arms the stall, terminates the frame — never delivered/counted).
 * @param {Uint8Array} frame
 * @returns {boolean}
 */
export function eth_pause_rx(frame) {
    const ptr0 = passArray8ToWasm0(frame, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.eth_pause_rx(ptr0, len0);
    return ret !== 0;
}

/**
 * PPS edge count: the observable sink for the PPS output pin (frequency
 * 2^n Hz from PTPPPSCR, gated by TSE). Test harnesses read this like a
 * scope probe; the guest itself cannot see it, like silicon.
 * @returns {number}
 */
export function eth_pps_count() {
    const ret = wasm.eth_pps_count();
    return ret >>> 0;
}

/**
 * PPS pin level (square wave at the PTPPPSCR rate, 50% duty). The
 * readable model of the PPS output — sample it like a logic analyzer.
 * @returns {boolean}
 */
export function eth_pps_level() {
    const ret = wasm.eth_pps_level();
    return ret !== 0;
}

/**
 * PTP current seconds / subseconds for TDES6/7 + RDES6/7 snapshots.
 * @returns {number}
 */
export function eth_ptp_sec() {
    const ret = wasm.eth_ptp_sec();
    return ret >>> 0;
}

/**
 * @returns {number}
 */
export function eth_ptp_sub() {
    const ret = wasm.eth_ptp_sub();
    return ret >>> 0;
}

/**
 * PTP timestamping enabled (PTPTSCR TSE). Gates RX/TX snapshots.
 * @returns {boolean}
 */
export function eth_ptp_tse() {
    const ret = wasm.eth_ptp_tse();
    return ret !== 0;
}

/**
 * RX checksum status for descriptor bits: bit 0 = has IPv4, bit 1 = IP
 * header OK, bit 2 = has TCP/UDP/ICMP, bit 3 = L4 OK. Maps to RDES0
 * IPHCE (bit 7) / PCE (bit 0).
 * @param {Uint8Array} frame
 * @returns {number}
 */
export function eth_rx_csum_status(frame) {
    const ptr0 = passArray8ToWasm0(frame, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.eth_rx_csum_status(ptr0, len0);
    return ret >>> 0;
}

/**
 * Signal to the peripheral that RX descriptor processing is complete.
 * Call this after writing received data into RX buffers.
 */
export function eth_rx_done() {
    wasm.eth_rx_done();
}

/**
 * Clear a latched RX stall (delivery succeeded).
 */
export function eth_rx_stall_clear() {
    wasm.eth_rx_stall_clear();
}

/**
 * Arm RX wire pacing for a delivered frame (RS waits the wire time).
 * @param {number} len
 */
export function eth_rx_wire_busy(len) {
    wasm.eth_rx_wire_busy(len);
}

/**
 * Set the wire link state (test-harness peer control).
 * @param {boolean} up
 */
export function eth_set_link(up) {
    wasm.eth_set_link(up);
}

/**
 * Re-arm the RX poll flag from JS (used when more packets are pending in gwRxQueue).
 * @param {number} desc_addr
 */
export function eth_signal_rx_poll(desc_addr) {
    wasm.eth_signal_rx_poll(desc_addr);
}

/**
 * Re-arm the TX poll flag from JS (used when more TX descriptors are pending).
 * @param {number} desc_addr
 */
export function eth_signal_tx_poll(desc_addr) {
    wasm.eth_signal_tx_poll(desc_addr);
}

/**
 * Station address (MACA0) packed as u64 (48 bits used) for SARC insert.
 * @returns {bigint}
 */
export function eth_station_addr() {
    const ret = wasm.eth_station_addr();
    return BigInt.asUintN(64, ret);
}

/**
 * Take a pending armed collision (one-shot, false when none armed).
 * @returns {boolean}
 */
export function eth_take_collision() {
    const ret = wasm.eth_take_collision();
    return ret !== 0;
}

/**
 * Take a pending pause-frame emission ((1<<31)|quanta, 0 when none).
 * @returns {number}
 */
export function eth_take_pause_tx() {
    const ret = wasm.eth_take_pause_tx();
    return ret >>> 0;
}

/**
 * True when a TX completing now must report deferral (half-duplex while
 * a receive still occupies the wire).
 * @returns {boolean}
 */
export function eth_tx_deferred() {
    const ret = wasm.eth_tx_deferred();
    return ret !== 0;
}

/**
 * Signal to the peripheral that TX descriptor processing is complete.
 * Call this after walking TX descriptors and sending the packet.
 */
export function eth_tx_done() {
    wasm.eth_tx_done();
}

/**
 * Immediate error completion (dead-wire NC / jabber JT): TS raises on
 * the next tick with no wire wait (the driver already wrote the error
 * status into the descriptor).
 */
export function eth_tx_done_now() {
    wasm.eth_tx_done_now();
}

/**
 * TX jabber limit from MACCR WD (2048, or 16383 with WD set).
 * @returns {number}
 */
export function eth_tx_jabber_limit() {
    const ret = wasm.eth_tx_jabber_limit();
    return ret >>> 0;
}

/**
 * SARC mode (MACCR[29:28]): 0/1 off, 2 insert-if-present, 3 replace.
 * @returns {number}
 */
export function eth_tx_sarc() {
    const ret = wasm.eth_tx_sarc();
    return ret >>> 0;
}

/**
 * Arm TX wire pacing for a `len`-byte frame: TS completion waits until the
 * frame has left the wire at the MACCR FES speed (168 MHz virtual clock).
 * @param {number} len
 */
export function eth_tx_wire_busy(len) {
    wasm.eth_tx_wire_busy(len);
}

/**
 * Called by the JS driver after it applied the queued erase to guest memory;
 * clears BSY/EOP so the firmware's busy-wait can proceed.
 */
export function flash_erase_applied() {
    wasm.flash_erase_applied();
}

/**
 * True when the FLASH peripheral is unlocked with PG set and !BSY — the
 * JS driver applies program writes to guest memory when this is true.
 * @returns {boolean}
 */
export function flash_is_programming() {
    const ret = wasm.flash_is_programming();
    return ret !== 0;
}

/**
 * FLASH readout-protection level from OPTCR RDP (0/1/2).
 * @returns {number}
 */
export function flash_rdp_level() {
    const ret = wasm.flash_rdp_level();
    return ret;
}

/**
 * Harness = the option-byte programmer: set the RDP byte (respects
 * OPTLOCK like the register path).
 * @param {number} level_byte
 */
export function flash_set_rdp(level_byte) {
    wasm.flash_set_rdp(level_byte);
}

/**
 * Consume a completed erase request (start, len) the JS driver must apply
 * to guest memory (all bytes 0xFF). Empty vec = nothing pending.
 * @returns {Uint32Array}
 */
export function flash_take_erase() {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.flash_take_erase(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Harness = the NAND flash array: bind an erased (0xFF) backing array of
 * `size` bytes to FSMC bank `bank` (0-3). Untapped data accesses go to
 * this array (program clears bits, reads return stored bytes).
 * @param {number} bank
 * @param {number} size
 */
export function fsmc_bind_nand(bank, size) {
    wasm.fsmc_bind_nand(bank, size);
}

/**
 * Erase `len` bytes at `offset` in FSMC bank `bank` (restore 0xFF) —
 * the silicon block-erase firmware runs before reprogram.
 * @param {number} bank
 * @param {number} offset
 * @param {number} len
 */
export function fsmc_nand_erase(bank, offset, len) {
    wasm.fsmc_nand_erase(bank, offset, len);
}

/**
 * Queue values the JS device answers on subsequent bank reads, oldest
 * first. An exhausted queue reads back 0.
 * @param {number} bank
 * @param {Uint32Array} values
 */
export function fsmc_push_data(bank, values) {
    const ptr0 = passArray32ToWasm0(values, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.fsmc_push_data(bank, ptr0, len0);
}

/**
 * Drain all FSMC tap events for a bank since the last call (2 words per
 * access, see `fsmc_tap`).
 * @param {number} bank
 * @returns {Uint32Array}
 */
export function fsmc_take_events(bank) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.fsmc_take_events(retptr, bank);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Register a protocol-agnostic tap on an FSMC memory bank (0 = BANK1, the
 * 0x6000_0000 window). Must be called before init(): the Fsmc peripheral
 * binds its banks' devices once at construction and never rescans.
 *
 * Every data-space access to the bank is queued for JS as TWO words —
 * header then value — where the header is `1<<31 | offset` for a write and
 * `offset` for a read. The offset matters: memory-mapped displays in
 * 8080 mode decode an address line as RS/DC, so the address is the only
 * thing separating a command write from a pixel write.
 * @param {number} bank
 */
export function fsmc_tap(bank) {
    wasm.fsmc_tap(bank);
}

/**
 * @returns {number}
 */
export function get_next_pending_interrupt() {
    const ret = wasm.get_next_pending_interrupt();
    return ret;
}

/**
 * Collect UART output since last call.
 * @returns {string}
 */
export function get_uart_output() {
    let deferred1_0;
    let deferred1_1;
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.get_uart_output(retptr);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        deferred1_0 = r0;
        deferred1_1 = r1;
        return getStringFromWasm0(r0, r1);
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
        wasm.__wbindgen_export(deferred1_0, deferred1_1, 1);
    }
}

/**
 * @param {number} port
 * @param {number} pin
 * @returns {boolean}
 */
export function gpio_read_input(port, pin) {
    const ret = wasm.gpio_read_input(port, pin);
    return ret !== 0;
}

/**
 * @param {number} port
 * @param {number} pin
 * @returns {boolean}
 */
export function gpio_read_output(port, pin) {
    const ret = wasm.gpio_read_output(port, pin);
    return ret !== 0;
}

/**
 * @param {number} port
 * @param {number} pin
 * @param {boolean} value
 */
export function gpio_set_input(port, pin, value) {
    wasm.gpio_set_input(port, pin, value);
}

/**
 * Check if any interrupt is pending (non-consuming).
 * @returns {boolean}
 */
export function has_pending_interrupt() {
    const ret = wasm.has_pending_interrupt();
    return ret !== 0;
}

/**
 * Harness = the other master: arm arbitration loss on the next address
 * phase of the I2C block at `base` (one-shot; ARLO latches, bus lost).
 * @param {number} base
 */
export function i2c_arm_arb_loss(base) {
    wasm.i2c_arm_arb_loss(base);
}

/**
 * Harness = the SMBus alerting device (host-notify source): arm the
 * address returned in DR on the next Alert-Response-Address read.
 * @param {number} base
 * @param {number} addr
 */
export function i2c_arm_smbus_alert(base, addr) {
    wasm.i2c_arm_smbus_alert(base, addr);
}

/**
 * Current PEC accumulator of the I2C block at `base` (scope probe).
 * @param {number} base
 * @returns {number}
 */
export function i2c_pec(base) {
    const ret = wasm.i2c_pec(base);
    return ret;
}

/**
 * Queue bytes the tapped I2C slave answers on master reads.
 * @param {string} peripheral
 * @param {Uint8Array} bytes
 */
export function i2c_push_rx(peripheral, bytes) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    wasm.i2c_push_rx(ptr0, len0, ptr1, len1);
}

/**
 * Read one register of the first matching regfile on a peripheral.
 * @param {string} peripheral
 * @param {number} offset
 * @returns {number}
 */
export function i2c_regfile_get(peripheral, offset) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.i2c_regfile_get(ptr0, len0, offset);
    return ret;
}

/**
 * Write one register of the first matching regfile on a peripheral
 * (JS-side poke, e.g. temperature coming from outside the guest).
 * @param {string} peripheral
 * @param {number} offset
 * @param {number} value
 */
export function i2c_regfile_set(peripheral, offset, value) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.i2c_regfile_set(ptr0, len0, offset, value);
}

/**
 * Register a pointer-addressed register file (DS3231 RTC style) on an I2C
 * peripheral. Must be called before init(). The first write byte of each
 * transaction is the register pointer, subsequent bytes land at `ptr++`;
 * reads return `regs[ptr++]` (pointer persists across address matches).
 * @param {string} peripheral
 * @param {number} address
 * @param {number} size
 * @param {Uint8Array} init
 */
export function i2c_register_regfile(peripheral, address, size, init) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(init, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    wasm.i2c_register_regfile(ptr0, len0, address, size, ptr1, len1);
}

/**
 * Register a protocol-agnostic I2C slave on a peripheral. Must be called
 * before init(). The address is ACKed like any other registered slave;
 * master writes queue for JS (`i2c_take_tx`) and JS-pushed bytes are
 * returned on master reads (`i2c_push_rx`).
 * @param {string} peripheral
 * @param {number} address
 */
export function i2c_register_slave(peripheral, address) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.i2c_register_slave(ptr0, len0, address);
}

/**
 * Drain all events for a tapped I2C slave since the last call. Each entry
 * is a u32: bit31 = START/STOP boundary event (bit30 = 1 START / 0 STOP),
 * otherwise the low byte is one byte the master wrote to the slave.
 * @param {string} peripheral
 * @returns {Uint32Array}
 */
export function i2c_take_events(peripheral) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
        const len0 = WASM_VECTOR_LEN;
        wasm.i2c_take_events(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v2 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Initialize the emulator with hardcoded peripheral map.
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init() {
    wasm.init();
}

/**
 * Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 * @param {string} svd_xml
 */
export function init_svd(svd_xml) {
    const ptr0 = passStringToWasm0(svd_xml, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.init_svd(ptr0, len0);
}

/**
 * @returns {boolean}
 */
export function is_watchdog_reset_requested() {
    const ret = wasm.is_watchdog_reset_requested();
    return ret !== 0;
}

/**
 * Queued backlog (bytes) for one ITM stimulus port.
 * @param {number} port
 * @returns {number}
 */
export function itm_port_pending(port) {
    const ret = wasm.itm_port_pending(port);
    return ret >>> 0;
}

/**
 * Drain one ITM stimulus port's queued trace bytes (oldest first).
 * Port 0 sinks into the UART console instead and always drains empty here;
 * ports 1-31 queue per-port streams for the JS driver.
 * @param {number} port
 * @returns {Uint8Array}
 */
export function itm_take_port(port) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.itm_take_port(retptr, port);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * @returns {boolean}
 */
export function iwdg_reset_flag() {
    const ret = wasm.iwdg_reset_flag();
    return ret !== 0;
}

/**
 * Frames completed by the LTDC scanout since enable.
 * @returns {number}
 */
export function ltdc_get_frame_count() {
    const ret = wasm.ltdc_get_frame_count();
    return ret >>> 0;
}

/**
 * Current LTDC scanline (0xFFFF when the controller is disabled).
 * @returns {number}
 */
export function ltdc_get_scanline() {
    const ret = wasm.ltdc_get_scanline();
    return ret >>> 0;
}

/**
 * @param {number} addr
 * @param {number} width
 * @returns {number}
 */
export function periph_read(addr, width) {
    const ret = wasm.periph_read(addr, width);
    return ret >>> 0;
}

/**
 * @param {number} addr
 * @param {number} width
 * @param {number} value
 */
export function periph_write(addr, width, value) {
    wasm.periph_write(addr, width, value);
}

/**
 * Record STANDBY entry (PDDS=1 + SLEEPDEEP WFI): sets SBF (CSR bit 1).
 * The driver calls this when it observes the guest enter WFI sleep with
 * PDDS set, before advancing virtual time.
 */
export function pwr_enter_standby() {
    wasm.pwr_enter_standby();
}

/**
 * Mark the PWR peripheral as having woken from a low-power (WFI/WFE) state.
 * The emulator calls this when the core resumes after a sleep halt so firmware
 * can read PWR->CSR WUF to confirm the wakeup source.
 */
export function pwr_wakeup() {
    wasm.pwr_wakeup();
}

/**
 * Wakeup from STANDBY: set WUF (CSR bit 0), keep SBF until the guest
 * clears it via CR CSBF.
 */
export function pwr_wakeup_standby() {
    wasm.pwr_wakeup_standby();
}

/**
 * Register an external QSPI flash image for the named QUADSPI peripheral.
 * Must be called before init(): the model binds its flash backend once at
 * construction and never rescans. `data` is the raw flash contents (e.g. a
 * W25Q-style image); indirect read/write transfers are serviced from it.
 * @param {string} name
 * @param {Uint8Array} data
 */
export function qspi_register_flash(name, data) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    wasm.qspi_register_flash(ptr0, len0, ptr1, len1);
}

/**
 * Harness = the failing oscillator: mark RCC HSE (bit 0) / PLL (bit 1)
 * dead or alive. Dead sources read RDY 0 and SWS falls back to HSI.
 * @param {number} src_mask
 * @param {boolean} dead
 */
export function rcc_inject_failure(src_mask, dead) {
    wasm.rcc_inject_failure(src_mask, dead);
}

/**
 * Clear all process-lifetime globals so a NEW emulator instance starts
 * clean.  Must be called before registering that instance's devices.
 * Without it, `ExtDevices` accumulates and a second instance silently binds
 * to the FIRST instance's devices (see system::reset_globals).
 */
export function reset_state() {
    wasm.reset_state();
}

/**
 * Host entropy words currently pooled (0 = LCG fallback active).
 * @returns {number}
 */
export function rng_entropy_avail() {
    const ret = wasm.rng_entropy_avail();
    return ret >>> 0;
}

/**
 * Push host entropy words into the RNG pool (true-noise samples from JS
 * `crypto.getRandomValues` or equivalent). Consumed FIFO, one word per
 * regen; when the pool drains the model falls back to the deterministic
 * LCG (SR SECS reports fallback-active).
 * @param {Uint32Array} words
 */
export function rng_seed_entropy(words) {
    const ptr0 = passArray32ToWasm0(words, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.rng_seed_entropy(ptr0, len0);
}

/**
 * Harness = the tamper pin: latch RTC TAMP1F (IRQ 2 when TAMPIE).
 */
export function rtc_tamper() {
    wasm.rtc_tamper();
}

/**
 * Harness = the tamper pin with physics (TAMP1E/TRG/FLT-gated, erases
 * backup registers, optional timestamp via TAMPTS).
 * @param {boolean} level
 */
export function rtc_tamper_pin(level) {
    wasm.rtc_tamper_pin(level);
}

/**
 * Harness = the timestamp pin event: capture TR/DR/SSR, latch TSF.
 */
export function rtc_timestamp() {
    wasm.rtc_timestamp();
}

/**
 * Current SDIO bus-width select (0 = 1-bit, 1 = 4-bit, 2 = 8-bit).
 * @returns {number}
 */
export function sdio_bus_width() {
    const ret = wasm.sdio_bus_width();
    return ret;
}

/**
 * Harness = the card's DAT1 interrupt line: latch/clear SDIOIT.
 * @param {boolean} set
 */
export function sdio_card_irq(set) {
    wasm.sdio_card_irq(set);
}

/**
 * Harness = the bad card: next CMD17/18 completion latches DCRCFAIL.
 */
export function sdio_fault_data_crc() {
    wasm.sdio_fault_data_crc();
}

/**
 * Set a pending interrupt in the NVIC. Negative `irq` values select system
 * exceptions (SVC = -5, PENDSV = -2, SYSTICK = -1) and are always deliverable.
 * Used by the FreeRTOS path: the Rust core synthesizes these exceptions
 * with exact inline entry/return.
 * @param {number} irq
 */
export function set_intr_pending(irq) {
    wasm.set_intr_pending(irq);
}

/**
 * Harness = the faulty peer: corrupt the RX CRC so the next CRCNEXT
 * compare on the SPI block at `base` mismatches (latches CRCERR).
 * @param {number} base
 */
export function spi_fault_crc(base) {
    wasm.spi_fault_crc(base);
}

/**
 * Harness = the NSS pin fault: latch MODF on the SPI block at `base`.
 * @param {number} base
 */
export function spi_fault_modf(base) {
    wasm.spi_fault_modf(base);
}

/**
 * Debug: flash state summary [wel, status1, cs_state, dummy_pending, pending_program_len]
 * @param {string} peripheral
 * @returns {Uint32Array}
 */
export function spi_flash_debug(peripheral) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
        const len0 = WASM_VECTOR_LEN;
        wasm.spi_flash_debug(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v2 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Push bytes the JS device answers on the MISO line (read transactions).
 * @param {string} peripheral
 * @param {Uint8Array} bytes
 */
export function spi_push_miso(peripheral, bytes) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_export2);
    const len1 = WASM_VECTOR_LEN;
    wasm.spi_push_miso(ptr0, len0, ptr1, len1);
}

/**
 * Harness = the SPI master clock: shift one frame through the slave at
 * `base` (returns the MISO word). No-op when the gate is closed.
 * @param {number} base
 * @param {number} mosi
 * @returns {number}
 */
export function spi_slave_clock(base, mosi) {
    const ret = wasm.spi_slave_clock(base, mosi);
    return ret >>> 0;
}

/**
 * Slave gate state of the SPI block at `base` (scope probe).
 * @param {number} base
 * @returns {boolean}
 */
export function spi_slave_gate(base) {
    const ret = wasm.spi_slave_gate(base);
    return ret !== 0;
}

/**
 * Harness = the SPI master: drive the slave's NSS level (true = asserted).
 * @param {number} base
 * @param {boolean} asserted
 */
export function spi_slave_select(base, asserted) {
    wasm.spi_slave_select(base, asserted);
}

/**
 * Drain all SPI tap events for a peripheral since the last call.
 * @param {string} peripheral
 * @returns {Uint32Array}
 */
export function spi_take_events(peripheral) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
        const len0 = WASM_VECTOR_LEN;
        wasm.spi_take_events(retptr, ptr0, len0);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v2 = getArrayU32FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 4, 4);
        return v2;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Register a protocol-agnostic tap on an SPI peripheral. Must be called
 * before init(). `cs` optionally names the GPIO pin used as chip select
 * ("PA4"); when given, CS edges are reported in the event stream. `dc`
 * optionally names a data/command pin; its level is reported in bit 29 of
 * each byte event (1 = data) so the JS device can parse TFT-style traffic.
 * @param {string} peripheral
 * @param {string | null} [cs]
 * @param {string | null} [dc]
 */
export function spi_tap(peripheral, cs, dc) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    var len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(dc) ? 0 : passStringToWasm0(dc, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    var len2 = WASM_VECTOR_LEN;
    wasm.spi_tap(ptr0, len0, ptr1, len1, ptr2, len2);
}

export function tick() {
    wasm.tick();
}

/**
 * Same as tick() but accounts for `delta` instructions at once. Timer
 * peripherals are instruction-count driven, so batching ticks with a
 * delta is semantically identical to one tick per instruction.
 * @param {number} delta
 */
export function tick_n(delta) {
    wasm.tick_n(delta);
}

/**
 * Run one peripheral-model tick WITHOUT advancing the instruction clock.
 * The CPU core publishes its executed count itself while stepping, so the
 * post-step driver tick must not add the budget a second time.
 */
export function tick_peripherals() {
    wasm.tick_peripherals();
}

/**
 * Host/JS-driven quadrature step on an encoder-mode timer: one TI edge
 * (`ti` 0 = TI1, 1 = TI2; `rising` = edge polarity). Counts per the
 * SMS/polarity rules; no-op outside encoder modes 1-3.
 * @param {string} name
 * @param {number} ti
 * @param {boolean} rising
 */
export function tim_encoder_step(name, ti, rising) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.tim_encoder_step(ptr0, len0, ti, rising);
}

/**
 * Host/JS-driven TIM input-capture edge. Simulate a TIx edge on timer `name`
 * channel `ch` and latch the live counter into its capture register (only if
 * the channel is configured for input capture via CCxS). Mirrors
 * `can_inject`: tests have no external signal source, so the edge is injected
 * from the driver. `name` is e.g. "TIM3"; `ch` is 0..3.
 * @param {string} name
 * @param {number} ch
 */
export function tim_inject_capture(name, ch) {
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
    const len0 = WASM_VECTOR_LEN;
    wasm.tim_inject_capture(ptr0, len0, ch);
}

/**
 * Harness = the noisy wire: arm framing (FE) / parity (PE) faults on the
 * next received byte of the USART at `base`. PE needs PCE enabled.
 * @param {number} base
 * @param {boolean} fe
 * @param {boolean} pe
 */
export function uart_fault_rx(base, fe, pe) {
    wasm.uart_fault_rx(base, fe, pe);
}

/**
 * Harness = the IR transmitter: inject a byte with a pulse class
 * (low_power selects the 1.6µs class, else the 3/16 class).
 * @param {number} base
 * @param {number} byte
 * @param {boolean} low_power
 */
export function uart_irda_rx(base, byte, low_power) {
    wasm.uart_irda_rx(base, byte, low_power);
}

/**
 * Pulse class of the last TX byte (scope probe for IrDA mode).
 * @param {number} base
 * @returns {number}
 */
export function uart_irda_tx_class(base) {
    const ret = wasm.uart_irda_tx_class(base);
    return ret;
}

/**
 * Harness = the LIN master: deliver a break frame to the USART at `base`.
 * @param {number} base
 */
export function uart_lin_break(base) {
    wasm.uart_lin_break(base);
}

/**
 * Inject a received byte into the UART at the given peripheral base address.
 * Returns true if a peripheral was found at that address.
 * @param {number} addr
 * @param {number} byte
 * @returns {boolean}
 */
export function uart_rx_byte(addr, byte) {
    const ret = wasm.uart_rx_byte(addr, byte);
    return ret !== 0;
}

/**
 * Harness = the smartcard: NACK the next transmitted byte.
 * @param {number} base
 */
export function uart_sc_nack(base) {
    wasm.uart_sc_nack(base);
}

/**
 * Smartcard retry counter of the USART at `base` (scope probe).
 * @param {number} base
 * @returns {number}
 */
export function uart_sc_retries(base) {
    const ret = wasm.uart_sc_retries(base);
    return ret;
}

/**
 * Harness = the CTS peer: drive the CTS input of the USART at `base`.
 * With CTSE set, deasserted CTS holds TX (TXE/TC clear, byte dropped).
 * @param {number} base
 * @param {boolean} asserted
 */
export function uart_set_cts(base, asserted) {
    wasm.uart_set_cts(base, asserted);
}

/**
 * Queued TX length of the USART at `base` (scope probe for CTSE-hold).
 * @param {number} base
 * @returns {number}
 */
export function uart_tx_len(base) {
    const ret = wasm.uart_tx_len(base);
    return ret >>> 0;
}

/**
 * Internal-DMA progress on the FS block: [in_bytes, out_bytes] moved
 * while GAHBCFG DMAEN was set (firmware polls the EP DMA registers;
 * the harness reads these counters directly).
 * @param {number} ep
 * @returns {BigUint64Array}
 */
export function usb_dma_progress(ep) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.usb_dma_progress(retptr, ep);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU64FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 8, 8);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Simulate enumeration-done at full speed (ENUMDNE + FS speed in DSTS).
 */
export function usb_enumerated() {
    wasm.usb_enumerated();
}

/**
 * Internal-DMA progress on the HS block.
 * @param {number} ep
 * @returns {BigUint64Array}
 */
export function usb_hs_dma_progress(ep) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.usb_hs_dma_progress(retptr, ep);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU64FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 8, 8);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * Simulate enumeration-done on the HS block (ENUMDNE + HS speed in DSTS).
 */
export function usb_hs_enumerated() {
    wasm.usb_hs_enumerated();
}

/**
 * HS IN transfer status: 0 none, 1 data ready, 2 STALL handshake.
 * @param {number} ep
 * @returns {number}
 */
export function usb_hs_in_status(ep) {
    const ret = wasm.usb_hs_in_status(ep);
    return ret >>> 0;
}

/**
 * Inject an OUT data packet to an HS endpoint.
 * @param {number} ep
 * @param {Uint8Array} data
 */
export function usb_hs_inject_out(ep, data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.usb_hs_inject_out(ep, ptr0, len0);
}

/**
 * Inject an 8-byte SETUP packet to HS EP0.
 * @param {Uint8Array} data
 */
export function usb_hs_inject_setup(data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.usb_hs_inject_setup(ptr0, len0);
}

/**
 * HS OUT transfer status: 0 none, 2 STALL handshake.
 * @param {number} ep
 * @returns {number}
 */
export function usb_hs_out_status(ep) {
    const ret = wasm.usb_hs_out_status(ep);
    return ret >>> 0;
}

/**
 * USB OTG HS (FS-mode personality) host-side test API: same semantics as
 * the FS exports above, driven against the HS block (0x40040000, IRQ 77).
 * Each falls back to the FS block when no HS slot exists.
 * Simulate a USB bus reset on the HS block.
 */
export function usb_hs_reset() {
    wasm.usb_hs_reset();
}

/**
 * Harness = the cable on the HS block.
 * @param {boolean} present
 */
export function usb_hs_set_vbus(present) {
    wasm.usb_hs_set_vbus(present);
}

/**
 * Drain a completed HS device-to-host IN blob.
 * @param {number} ep
 * @returns {Uint8Array}
 */
export function usb_hs_take_in(ep) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.usb_hs_take_in(retptr, ep);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * HS microframe index for the HS block.
 * @returns {number}
 */
export function usb_hs_uframe() {
    const ret = wasm.usb_hs_uframe();
    return ret >>> 0;
}

/**
 * ULPI rate for the HS block.
 * @returns {number}
 */
export function usb_hs_ulpi_rate() {
    const ret = wasm.usb_hs_ulpi_rate();
    return ret >>> 0;
}

/**
 * IN transfer status: 0 none, 1 data ready, 2 STALL handshake.
 * @param {number} ep
 * @returns {number}
 */
export function usb_in_status(ep) {
    const ret = wasm.usb_in_status(ep);
    return ret >>> 0;
}

/**
 * Inject an OUT data packet to an endpoint.
 * @param {number} ep
 * @param {Uint8Array} data
 */
export function usb_inject_out(ep, data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.usb_inject_out(ep, ptr0, len0);
}

/**
 * Inject an 8-byte SETUP packet to EP0.
 * @param {Uint8Array} data
 */
export function usb_inject_setup(data) {
    const ptr0 = passArray8ToWasm0(data, wasm.__wbindgen_export2);
    const len0 = WASM_VECTOR_LEN;
    wasm.usb_inject_setup(ptr0, len0);
}

/**
 * OUT transfer status: 0 none, 2 STALL handshake (OUT has no data-ready
 * slot — reception completes via the endpoint interrupt).
 * @param {number} ep
 * @returns {number}
 */
export function usb_out_status(ep) {
    const ret = wasm.usb_out_status(ep);
    return ret >>> 0;
}

/**
 * USB OTG FS host-side test API (the harness plays USB host; see
 * peripherals/usb.rs). Drive reset -> enum-done -> SETUP/OUT inject,
 * and drain device-to-host IN blobs with usb_take_in.
 * Simulate a USB bus reset: fresh device session, USBRST latched.
 */
export function usb_reset() {
    wasm.usb_reset();
}

/**
 * Harness = the cable: plug/unplug VBUS on the FS block (default present).
 * Unplug suspends the device, stops SOF, latches SEDET + BSVLD clear.
 * @param {boolean} present
 */
export function usb_set_vbus(present) {
    wasm.usb_set_vbus(present);
}

/**
 * Drain a completed device-to-host IN blob (empty = none pending;
 * check usb_in_status first to tell ZLP apart).
 * @param {number} ep
 * @returns {Uint8Array}
 */
export function usb_take_in(ep) {
    try {
        const retptr = wasm.__wbindgen_add_to_stack_pointer(-16);
        wasm.usb_take_in(retptr, ep);
        var r0 = getDataViewMemory0().getInt32(retptr + 4 * 0, true);
        var r1 = getDataViewMemory0().getInt32(retptr + 4 * 1, true);
        var v1 = getArrayU8FromWasm0(r0, r1).slice();
        wasm.__wbindgen_export(r0, r1 * 1, 1);
        return v1;
    } finally {
        wasm.__wbindgen_add_to_stack_pointer(16);
    }
}

/**
 * HS microframe index (DSTS FNSOF low 3 bits, 0..7) for the FS block
 * (always 0 — FS has 1 ms frames, no microframes).
 * @returns {number}
 */
export function usb_uframe() {
    const ret = wasm.usb_uframe();
    return ret >>> 0;
}

/**
 * ULPI PHY packet wire rate in Mbit/s (480 HS / 12 FS) for the FS block.
 * @returns {number}
 */
export function usb_ulpi_rate() {
    const ret = wasm.usb_ulpi_rate();
    return ret >>> 0;
}

/**
 * @returns {boolean}
 */
export function wwdg_reset_flag() {
    const ret = wasm.wwdg_reset_flag();
    return ret !== 0;
}
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg___wbindgen_throw_344f42d3211c4765: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_export(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return addHeapObject(ret);
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = getObject(arg1).stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_export2, wasm.__wbindgen_export3);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return addHeapObject(ret);
        },
        __wbindgen_object_drop_ref: function(arg0) {
            takeObject(arg0);
        },
    };
    return {
        __proto__: null,
        "./stm32_periph_wasm_bg.js": import0,
    };
}

const WasmCpuFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_wasmcpu_free(ptr, 1));

function addHeapObject(obj) {
    if (heap_next === heap.length) heap.push(heap.length + 1);
    const idx = heap_next;
    heap_next = heap[idx];

    heap[idx] = obj;
    return idx;
}

function dropObject(idx) {
    if (idx < 1028) return;
    heap[idx] = heap_next;
    heap_next = idx;
}

function getArrayU16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU64FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getBigUint64ArrayMemory0().subarray(ptr / 8, ptr / 8 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedBigUint64ArrayMemory0 = null;
function getBigUint64ArrayMemory0() {
    if (cachedBigUint64ArrayMemory0 === null || cachedBigUint64ArrayMemory0.byteLength === 0) {
        cachedBigUint64ArrayMemory0 = new BigUint64Array(wasm.memory.buffer);
    }
    return cachedBigUint64ArrayMemory0;
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint16ArrayMemory0 = null;
function getUint16ArrayMemory0() {
    if (cachedUint16ArrayMemory0 === null || cachedUint16ArrayMemory0.byteLength === 0) {
        cachedUint16ArrayMemory0 = new Uint16Array(wasm.memory.buffer);
    }
    return cachedUint16ArrayMemory0;
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function getObject(idx) { return heap[idx]; }

let heap = new Array(1024).fill(undefined);
heap.push(undefined, null, true, false);

let heap_next = heap.length;

function isLikeNone(x) {
    return x === undefined || x === null;
}

function passArray32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getUint32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArray8ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 1, 1) >>> 0;
    getUint8ArrayMemory0().set(arg, ptr / 1);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeObject(idx) {
    const ret = getObject(idx);
    dropObject(idx);
    return ret;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedBigUint64ArrayMemory0 = null;
    cachedDataViewMemory0 = null;
    cachedUint16ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('stm32_periph_wasm_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
