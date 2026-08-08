/* tslint:disable */
/* eslint-disable */

/**
 * Add an I2C EEPROM device. Must be called before init().
 */
export function add_i2c_eeprom(peripheral: string, address: number, data: Uint8Array): void;

/**
 * Register a software SPI device. Must be called before init().
 */
export function add_software_spi(name: string, cs: string | null | undefined, clk: string, miso: string, mosi: string): void;

/**
 * Add an SPI flash device. Must be called before init().
 */
export function add_spi_flash(peripheral: string, jedec_id: number, data: Uint8Array, cs?: string | null): void;

export function dma_get_pending(index: number): Uint32Array;

export function dma_get_pending_count(): number;

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

export function periph_read(addr: number, width: number): number;

export function periph_write(addr: number, width: number, value: number): void;

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
    readonly dma_get_pending: (a: number) => [number, number];
    readonly dma_get_pending_count: () => number;
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
    readonly get_next_pending_interrupt: () => number;
    readonly get_uart_output: () => [number, number];
    readonly gpio_read_input: (a: number, b: number) => number;
    readonly gpio_read_output: (a: number, b: number) => number;
    readonly gpio_set_input: (a: number, b: number, c: number) => void;
    readonly has_pending_interrupt: () => number;
    readonly init: () => void;
    readonly init_svd: (a: number, b: number) => void;
    readonly is_watchdog_reset_requested: () => number;
    readonly periph_read: (a: number, b: number) => number;
    readonly periph_write: (a: number, b: number, c: number) => void;
    readonly tick: () => void;
    readonly tick_n: (a: number) => void;
    readonly uart_rx_byte: (a: number, b: number) => number;
    readonly __wbindgen_free: (a: number, b: number, c: number) => void;
    readonly __wbindgen_malloc: (a: number, b: number) => number;
    readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_externrefs: WebAssembly.Table;
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
