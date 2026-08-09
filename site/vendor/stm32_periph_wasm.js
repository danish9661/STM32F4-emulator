/* @ts-self-types="./stm32_periph_wasm.d.ts" */

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
 * @returns {Uint8Array}
 */
export function dma_periph_read(addr, size) {
    const ret = wasm.dma_periph_read(addr, size);
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
