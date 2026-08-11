/* tslint:disable */
/* eslint-disable */

export function add_i2c_eeprom(peripheral: string, address: number, data: Uint8Array): void;

/**
 * Register a software SPI device. Must be called before init().
 */
export function add_software_spi(name: string, cs: string | null | undefined, clk: string, miso: string, mosi: string): void;

/**
 * Add an SPI flash device. Must be called before init().
 */
export function add_spi_flash(peripheral: string, jedec_id: number, data: Uint8Array, cs?: string | null): void;

/**
 * Reset the audio source and capture FIFO.
 */
export function audio_clear(): void;

/**
 * Load a WAV file (PCM 16-bit) as the I2S/SAI sample source. DR reads then
 * consume samples from it. Returns an error string on malformed input.
 */
export function audio_load_wav(bytes: Uint8Array): void;

/**
 * Remaining source samples (0 when no WAV is loaded or it is exhausted).
 */
export function audio_source_remaining(): number;

/**
 * Drain the I2S/SAI TX capture FIFO (all DR writes since the last call).
 */
export function audio_take_capture(): Uint16Array;

/**
 * Forget any fed frame (stop the camera).
 */
export function dcmi_clear(): void;

/**
 * Provide the next camera frame to the DCMI controller (8-bit pixels,
 * row-major, width x height). The next CAPTURE start consumes it.
 */
export function dcmi_feed_frame(w: number, h: number, pixels: Uint8Array): void;

export function dma_get_pending(index: number): Uint32Array;

export function dma_get_pending_count(): number;

/**
 * DMA peripheral-side chunked read: read `size` bytes from peripheral
 * `addr` (4-byte-aligned, tail chunk partial). Replaces the JS per-chunk
 * periph_read loop (one WASM call instead of size/4).
 */
export function dma_periph_read(addr: number, size: number, pinc: boolean, psize: number): Uint8Array;

/**
 * DMA peripheral-side chunked write: write `bytes` to peripheral `addr` in
 * 4-byte chunks (tail chunk partial). Replaces the JS per-chunk periph_write
 * loop (one WASM call instead of size/4).
 */
export function dma_periph_write(addr: number, bytes: Uint8Array): void;

export function dma_set_completed(stream_idx: number, success: boolean): void;

/**
 * Clear the RX poll flag (call after processing descriptors).
 */
export function eth_clear_rx_poll(): void;

/**
 * Clear the TX poll flag (call after processing descriptors).
 */
export function eth_clear_tx_poll(): void;

/**
 * Get the RX descriptor list address for the current poll.
 */
export function eth_get_rx_desc_addr(): number;

/**
 * Get the TX descriptor list address for the current poll.
 */
export function eth_get_tx_desc_addr(): number;

/**
 * Check if an Ethernet RX poll is pending (firmware wants to receive a packet).
 */
export function eth_is_rx_poll(): boolean;

/**
 * Check if an Ethernet TX poll is pending (firmware wants to send a packet).
 */
export function eth_is_tx_poll(): boolean;

/**
 * Signal to the peripheral that RX descriptor processing is complete.
 * Call this after writing received data into RX buffers.
 */
export function eth_rx_done(): void;

/**
 * Re-arm the RX poll flag from JS (used when more packets are pending in gwRxQueue).
 */
export function eth_signal_rx_poll(desc_addr: number): void;

/**
 * Re-arm the TX poll flag from JS (used when more TX descriptors are pending).
 */
export function eth_signal_tx_poll(desc_addr: number): void;

/**
 * Signal to the peripheral that TX descriptor processing is complete.
 * Call this after walking TX descriptors and sending the packet.
 */
export function eth_tx_done(): void;

/**
 * Called by the JS driver after it applied the queued erase to guest memory;
 * clears BSY/EOP so the firmware's busy-wait can proceed.
 */
export function flash_erase_applied(): void;

/**
 * True when the FLASH peripheral is unlocked with PG set and !BSY — the
 * JS driver applies program writes to guest memory when this is true.
 */
export function flash_is_programming(): boolean;

/**
 * Consume a completed erase request (start, len) the JS driver must apply
 * to guest memory (all bytes 0xFF). Empty vec = nothing pending.
 */
export function flash_take_erase(): Uint32Array;

export function get_next_pending_interrupt(): number;

/**
 * Collect UART output since last call.
 */
export function get_uart_output(): string;

export function gpio_read_input(port: number, pin: number): boolean;

export function gpio_read_output(port: number, pin: number): boolean;

export function gpio_set_input(port: number, pin: number, value: boolean): void;

/**
 * Check if any interrupt is pending (non-consuming).
 */
export function has_pending_interrupt(): boolean;

/**
 * Queue bytes the tapped I2C slave answers on master reads.
 */
export function i2c_push_rx(peripheral: string, bytes: Uint8Array): void;

/**
 * Register a protocol-agnostic I2C slave on a peripheral. Must be called
 * before init(). The address is ACKed like any other registered slave;
 * master writes queue for JS (`i2c_take_tx`) and JS-pushed bytes are
 * returned on master reads (`i2c_push_rx`).
 */
export function i2c_register_slave(peripheral: string, address: number): void;

/**
 * Drain all events for a tapped I2C slave since the last call. Each entry
 * is a u32: bit31 = START/STOP boundary event (bit30 = 1 START / 0 STOP),
 * otherwise the low byte is one byte the master wrote to the slave.
 */
export function i2c_take_events(peripheral: string): Uint32Array;

/**
 * Initialize the emulator with hardcoded peripheral map.
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init(): void;

/**
 * Initialize the emulator from an SVD XML string (e.g., STM32F407.svd).
 * Must be called after adding all ext devices (add_spi_flash, add_i2c_eeprom).
 */
export function init_svd(svd_xml: string): void;

export function is_watchdog_reset_requested(): boolean;

/**
 * Frames completed by the LTDC scanout since enable.
 */
export function ltdc_get_frame_count(): number;

/**
 * Current LTDC scanline (0xFFFF when the controller is disabled).
 */
export function ltdc_get_scanline(): number;

export function periph_read(addr: number, width: number): number;

export function periph_write(addr: number, width: number, value: number): void;

/**
 * Debug: flash state summary [wel, status1, cs_state, dummy_pending, pending_program_len]
 */
export function spi_flash_debug(peripheral: string): Uint32Array;

/**
 * Push bytes the JS device answers on the MISO line (read transactions).
 */
export function spi_push_miso(peripheral: string, bytes: Uint8Array): void;

/**
 * Drain all SPI tap events for a peripheral since the last call.
 */
export function spi_take_events(peripheral: string): Uint32Array;

/**
 * Register a protocol-agnostic tap on an SPI peripheral. Must be called
 * before init(). `cs` optionally names the GPIO pin used as chip select
 * ("PA4"); when given, CS edges are reported in the event stream. `dc`
 * optionally names a data/command pin; its level is reported in bit 29 of
 * each byte event (1 = data) so the JS device can parse TFT-style traffic.
 */
export function spi_tap(peripheral: string, cs?: string | null, dc?: string | null): void;

export function tick(): void;

/**
 * Same as tick() but accounts for `delta` instructions at once. Timer
 * peripherals are instruction-count driven, so batching ticks with a
 * delta is semantically identical to one tick per instruction.
 */
export function tick_n(delta: number): void;

/**
 * Inject a received byte into the UART at the given peripheral base address.
 * Returns true if a peripheral was found at that address.
 */
export function uart_rx_byte(addr: number, byte: number): boolean;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly add_i2c_eeprom: (a: number, b: number, c: number, d: number, e: number) => void;
    readonly add_software_spi: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number) => void;
    readonly add_spi_flash: (a: number, b: number, c: number, d: number, e: number, f: number, g: number) => void;
    readonly audio_clear: () => void;
    readonly audio_load_wav: (a: number, b: number) => [number, number];
    readonly audio_source_remaining: () => number;
    readonly audio_take_capture: () => [number, number];
    readonly dcmi_clear: () => void;
    readonly dcmi_feed_frame: (a: number, b: number, c: number, d: number) => void;
    readonly dma_get_pending: (a: number) => [number, number];
    readonly dma_get_pending_count: () => number;
    readonly dma_periph_read: (a: number, b: number, c: number, d: number) => [number, number];
    readonly dma_periph_write: (a: number, b: number, c: number) => void;
    readonly dma_set_completed: (a: number, b: number) => void;
    readonly eth_clear_rx_poll: () => void;
    readonly eth_clear_tx_poll: () => void;
    readonly eth_get_rx_desc_addr: () => number;
    readonly eth_get_tx_desc_addr: () => number;
    readonly eth_is_rx_poll: () => number;
    readonly eth_is_tx_poll: () => number;
    readonly eth_rx_done: () => void;
    readonly eth_signal_rx_poll: (a: number) => void;
    readonly eth_signal_tx_poll: (a: number) => void;
    readonly eth_tx_done: () => void;
    readonly flash_erase_applied: () => void;
    readonly flash_is_programming: () => number;
    readonly flash_take_erase: () => [number, number];
    readonly get_next_pending_interrupt: () => number;
    readonly get_uart_output: () => [number, number];
    readonly gpio_read_input: (a: number, b: number) => number;
    readonly gpio_read_output: (a: number, b: number) => number;
    readonly gpio_set_input: (a: number, b: number, c: number) => void;
    readonly has_pending_interrupt: () => number;
    readonly i2c_push_rx: (a: number, b: number, c: number, d: number) => void;
    readonly i2c_register_slave: (a: number, b: number, c: number) => void;
    readonly i2c_take_events: (a: number, b: number) => [number, number];
    readonly init: () => void;
    readonly init_svd: (a: number, b: number) => void;
    readonly is_watchdog_reset_requested: () => number;
    readonly ltdc_get_frame_count: () => number;
    readonly ltdc_get_scanline: () => number;
    readonly periph_read: (a: number, b: number) => number;
    readonly periph_write: (a: number, b: number, c: number) => void;
    readonly spi_flash_debug: (a: number, b: number) => [number, number];
    readonly spi_push_miso: (a: number, b: number, c: number, d: number) => void;
    readonly spi_take_events: (a: number, b: number) => [number, number];
    readonly spi_tap: (a: number, b: number, c: number, d: number, e: number, f: number) => void;
    readonly tick: () => void;
    readonly tick_n: (a: number) => void;
    readonly uart_rx_byte: (a: number, b: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
    readonly __externref_table_dealloc: (a: number) => void;
    readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
