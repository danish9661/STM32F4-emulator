/* @ts-self-types="./stm32_periph_wasm.d.ts" */

/**
 * Remove a channel override, reverting it to the synthetic default.
 * @param {string} peripheral
 * @param {number} channel
 */
export function adc_clear_channel_value(peripheral, channel) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.adc_clear_channel_value(ptr0, len0, channel);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.adc_set_channel_value(ptr0, len0, channel, value);
}

/**
 * @param {string} peripheral
 * @param {number} address
 * @param {Uint8Array} data
 */
export function add_i2c_eeprom(peripheral, address, data) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
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
    const ptr0 = passStringToWasm0(name, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len1 = WASM_VECTOR_LEN;
    const ptr2 = passStringToWasm0(clk, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len2 = WASM_VECTOR_LEN;
    const ptr3 = passStringToWasm0(miso, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len3 = WASM_VECTOR_LEN;
    const ptr4 = passStringToWasm0(mosi, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(data, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.audio_load_wav(ptr0, len0);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
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
    const ret = wasm.audio_take_capture();
    var v1 = getArrayU16FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 2, 2);
    return v1;
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
    const ptr0 = passArray8ToWasm0(pixels, wasm.__wbindgen_malloc);
    const len0 = WASM_VECTOR_LEN;
    wasm.dcmi_feed_frame(w, h, ptr0, len0);
}

/**
 * @param {number} index
 * @returns {Uint32Array}
 */
export function dma_get_pending(index) {
    const ret = wasm.dma_get_pending(index);
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
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
    const ret = wasm.dma_periph_read(addr, size, pinc, psize);
    var v1 = getArrayU8FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 1, 1);
    return v1;
}

/**
 * DMA peripheral-side chunked write: write `bytes` to peripheral `addr` in
 * 4-byte chunks (tail chunk partial). Replaces the JS per-chunk periph_write
 * loop (one WASM call instead of size/4).
 * @param {number} addr
 * @param {Uint8Array} bytes
 */
export function dma_periph_write(addr, bytes) {
    const ptr0 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
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
 * Signal to the peripheral that RX descriptor processing is complete.
 * Call this after writing received data into RX buffers.
 */
export function eth_rx_done() {
    wasm.eth_rx_done();
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
 * Signal to the peripheral that TX descriptor processing is complete.
 * Call this after walking TX descriptors and sending the packet.
 */
export function eth_tx_done() {
    wasm.eth_tx_done();
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
 * Consume a completed erase request (start, len) the JS driver must apply
 * to guest memory (all bytes 0xFF). Empty vec = nothing pending.
 * @returns {Uint32Array}
 */
export function flash_take_erase() {
    const ret = wasm.flash_take_erase();
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
}

/**
 * Queue values the JS device answers on subsequent bank reads, oldest
 * first. An exhausted queue reads back 0.
 * @param {number} bank
 * @param {Uint32Array} values
 */
export function fsmc_push_data(bank, values) {
    const ptr0 = passArray32ToWasm0(values, wasm.__wbindgen_malloc);
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
    const ret = wasm.fsmc_take_events(bank);
    var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v1;
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
        const ret = wasm.get_uart_output();
        deferred1_0 = ret[0];
        deferred1_1 = ret[1];
        return getStringFromWasm0(ret[0], ret[1]);
    } finally {
        wasm.__wbindgen_free(deferred1_0, deferred1_1, 1);
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
 * Queue bytes the tapped I2C slave answers on master reads.
 * @param {string} peripheral
 * @param {Uint8Array} bytes
 */
export function i2c_push_rx(peripheral, bytes) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(init, wasm.__wbindgen_malloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.i2c_take_events(ptr0, len0);
    var v2 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v2;
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
    const ptr0 = passStringToWasm0(svd_xml, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
 * Clear all process-lifetime globals so a NEW emulator instance starts
 * clean.  Must be called before registering that instance's devices.
 * Without it, `ExtDevices` accumulates and a second instance silently binds
 * to the FIRST instance's devices (see system::reset_globals).
 */
export function reset_state() {
    wasm.reset_state();
}

/**
 * Set a pending interrupt in the NVIC. Negative `irq` values select system
 * exceptions (SVC = -5, PENDSV = -2, SYSTICK = -1) and are always deliverable.
 * Used by the FreeRTOS path in the JS driver, which detects `svc` in the CPU
 * hook and synthesizes the SVC exception here instead of letting Unicorn take
 * it natively (this WASM build cannot perform the Cortex-M exception return).
 * @param {number} irq
 */
export function set_intr_pending(irq) {
    wasm.set_intr_pending(irq);
}

/**
 * Debug: flash state summary [wel, status1, cs_state, dummy_pending, pending_program_len]
 * @param {string} peripheral
 * @returns {Uint32Array}
 */
export function spi_flash_debug(peripheral) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.spi_flash_debug(ptr0, len0);
    var v2 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v2;
}

/**
 * Push bytes the JS device answers on the MISO line (read transactions).
 * @param {string} peripheral
 * @param {Uint8Array} bytes
 */
export function spi_push_miso(peripheral, bytes) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ptr1 = passArray8ToWasm0(bytes, wasm.__wbindgen_malloc);
    const len1 = WASM_VECTOR_LEN;
    wasm.spi_push_miso(ptr0, len0, ptr1, len1);
}

/**
 * Drain all SPI tap events for a peripheral since the last call.
 * @param {string} peripheral
 * @returns {Uint32Array}
 */
export function spi_take_events(peripheral) {
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    const ret = wasm.spi_take_events(ptr0, len0);
    var v2 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
    wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
    return v2;
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
    const ptr0 = passStringToWasm0(peripheral, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    const len0 = WASM_VECTOR_LEN;
    var ptr1 = isLikeNone(cs) ? 0 : passStringToWasm0(cs, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
    var len1 = WASM_VECTOR_LEN;
    var ptr2 = isLikeNone(dc) ? 0 : passStringToWasm0(dc, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
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
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./stm32_periph_wasm_bg.js": import0,
    };
}

function getArrayU16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
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

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
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
    cachedDataViewMemory0 = null;
    cachedUint16ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
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
