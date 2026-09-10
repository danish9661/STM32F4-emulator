// Board/chip variants for the multi-board emulator. Every F4 in this list
// is a Cortex-M4F — the CPU core is shared byte-identical; a board differs
// only in SVD (register map), flash/RAM sizes, and clock. Firmware presets
// map to boards via BOARD_OF_FIRMWARE (default: stm32f407).
//
// SVD files live in site/vendor/ next to stm32f407.svd (wasm-pack deletes
// that dir on rebuild — restore them alongside the SVD, see AGENTS.md).

export const BOARDS = {
    // BlackPill F401CC / Nucleo-F401RE (STM32F401CC/Rx: 256K flash, 64K RAM)
    stm32f401: {
        svd: 'stm32f401.svd',
        flash_size: 0x40000,
        ram_size: 0x10000,
        maxClockMHz: 84,
        label: 'STM32F401 (256K/64K)',
    },
    // BlackPill F411CE / Nucleo-F411RE (STM32F411CE/Rx: 512K flash, 128K RAM)
    stm32f411: {
        svd: 'stm32f411.svd',
        flash_size: 0x80000,
        ram_size: 0x20000,
        maxClockMHz: 100,
        label: 'STM32F411 (512K/128K)',
    },
    // F407VG/VG (Discovery, 1M flash, 128K RAM + 64K CCM)
    stm32f407: {
        svd: 'stm32f407.svd',
        flash_size: 0x100000,
        ram_size: 0x20000,
        maxClockMHz: 168,
        label: 'STM32F407 (1M/128K)',
    },
    // F407VE/ZE black boards (512K flash, 128K RAM — same die, new package)
    stm32f407ve: {
        svd: 'stm32f407.svd',
        flash_size: 0x80000,
        ram_size: 0x20000,
        maxClockMHz: 168,
        label: 'STM32F407VE/ZE (512K/128K)',
    },
    // STM32F429ZI (Discovery: 2M flash, 256K RAM + 64K CCM)
    stm32f429: {
        svd: 'stm32f429.svd',
        flash_size: 0x200000,
        ram_size: 0x40000,
        maxClockMHz: 180,
        label: 'STM32F429 (2M/256K)',
    },
};

// Firmware preset -> board. Anything absent here runs as stm32f407
// (all existing presets predate boards.js).
export const BOARD_OF_FIRMWARE = {
    blinky_f401: 'stm32f401',
    blinky_f411: 'stm32f411',
    blinky_nucleo_f401: 'stm32f401',
    blinky_nucleo_f411: 'stm32f411',
    blinky_f429: 'stm32f429',
    blinky_f407ve: 'stm32f407ve',
    blinky_f407ze: 'stm32f407ve',
};

export function boardFor(fwName) {
    return BOARDS[BOARD_OF_FIRMWARE[fwName] || 'stm32f407'];
}
