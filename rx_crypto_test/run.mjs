import { readFileSync } from 'fs';
import { initSync, periph_read, periph_write, tick, get_next_pending_interrupt, dma_get_pending_count, dma_get_pending, dma_set_completed, is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte } from '../stm32-periph-wasm/pkg/stm32_periph_wasm.js';
initSync(readFileSync('../stm32-periph-wasm/pkg/stm32_periph_wasm_bg.wasm'));

const { createRequire } = await import('module');
const require = createRequire(import.meta.url);
const MUnicorn = require('../stm32-periph-wasm/pkg/unicorn_arm.cjs');
const Module = await MUnicorn({});

const firmware = readFileSync('rx_crypto_test.bin');
init_svd(readFileSync('../monox/stm32f407.svd', 'utf8'));

const uc = new Module.Unicorn(Module.ARCH_ARM, Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN);
uc.mem_map(0x08000000, 0x100000, Module.PROT_ALL);
uc.mem_write(0x08000000, firmware);
uc.mem_map(0x20000000, 0x20000, Module.PROT_ALL);
uc.mem_map(0x40000000, 0x10000000, Module.PROT_READ | Module.PROT_WRITE);
uc.mem_map(0xE0000000, 0x00100000, Module.PROT_READ | Module.PROT_WRITE);

const read32 = (addr) => {
    const b = uc.mem_read(BigInt(addr), 4);
    return new DataView(b.buffer, b.byteOffset, b.byteLength).getUint32(0, true);
};

const sp_init = read32(0x08000000);
const pc_init = read32(0x08000004);
uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);
console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);

const periphRanges = [[0x40000000, 0xB0000000], [0xE0000000, 0xE1000000]];
for (const [start, end] of periphRanges) {
    uc.hook_add(Module.HOOK_MEM_READ,
        (h, t, addr, size, val, ud) => {
            const data = uc.mem_read(addr, size);
            const bytes = new Uint8Array(size);
            const v = periph_read(Number(addr), size) >>> 0;
            for (let i = 0; i < size; i++) bytes[i] = (v >> (i * 8)) & 0xFF;
            uc.mem_write(addr, bytes);
        }, null, start, end);
    uc.hook_add(Module.HOOK_MEM_WRITE,
        (h, t, addr, size, val, ud) => periph_write(Number(addr), size, Number(val)),
        null, start, end);
}

let instCount = 0n;
uc.hook_add(Module.HOOK_CODE, (h, addr, size, ud) => {
    instCount++;
    tick();
});

uc.hook_add(Module.HOOK_INTR, (h, intno, ud) => {
    if (intno === 8) {
        const sp = uc.reg_read_i32(Module.ARM_REG_SP);
        const frame = uc.mem_read(BigInt(sp), 32);
        const sv = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
        uc.reg_write_i32(Module.ARM_REG_R0, sv.getUint32(28, true));
        uc.reg_write_i32(Module.ARM_REG_R1, sv.getUint32(24, true));
        uc.reg_write_i32(Module.ARM_REG_R2, sv.getUint32(20, true));
        uc.reg_write_i32(Module.ARM_REG_R3, sv.getUint32(16, true));
        uc.reg_write_i32(Module.ARM_REG_R12, sv.getUint32(12, true));
        uc.reg_write_i32(Module.ARM_REG_LR, sv.getUint32(8, true));
        uc.reg_write_i32(Module.ARM_REG_PC, sv.getUint32(4, true) | 1);
        uc.reg_write_i32(Module.ARM_REG_SP, sp + 32);
    }
});

const processInterrupts = () => {
    while (true) {
        const irq = get_next_pending_interrupt();
        if (irq <= -100) break;
        console.log(`\nProcessing IRQ ${irq}...`);
        
        const sp = uc.reg_read_i32(Module.ARM_REG_SP);
        const pc = uc.reg_read_i32(Module.ARM_REG_PC);
        const lr = uc.reg_read_i32(Module.ARM_REG_LR);
        const xpsr = uc.reg_read_i32(Module.ARM_REG_XPSR);
        const r0 = uc.reg_read_i32(Module.ARM_REG_R0);
        const r1 = uc.reg_read_i32(Module.ARM_REG_R1);
        const r2 = uc.reg_read_i32(Module.ARM_REG_R2);
        const r3 = uc.reg_read_i32(Module.ARM_REG_R3);
        const r12 = uc.reg_read_i32(Module.ARM_REG_R12);
        
        console.log(`  Before: PC=0x${pc.toString(16)} SP=0x${sp.toString(16)} LR=0x${lr.toString(16)} R0=0x${r0.toString(16)}`);

        const frame = new Uint8Array(32);
        const sv = new DataView(frame.buffer);
        sv.setUint32(0, xpsr, true);
        sv.setUint32(4, pc, true);
        sv.setUint32(8, lr, true);
        sv.setUint32(12, r12, true);
        sv.setUint32(16, r3, true);
        sv.setUint32(20, r2, true);
        sv.setUint32(24, r1, true);
        sv.setUint32(28, r0, true);
        
        const savedAt = sp - 32;
        uc.mem_write(BigInt(savedAt), frame);
        uc.reg_write_i32(Module.ARM_REG_SP, savedAt);
        
        const handler_pc = read32(0x08000000 + 4 * (16 + irq));
        console.log(`  Handler at 0x${handler_pc.toString(16)}`);
        
        uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
        uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
        
        try {
            uc.emu_start(BigInt(handler_pc), 0n, 0n, 100000);
        } catch (e) {
            console.log(`  Handler crashed: ${e.message || e}`);
        }
        
        const savedSp = uc.reg_read_i32(Module.ARM_REG_SP);
        console.log(`  After handler: SP=0x${savedSp.toString(16)} (expected 0x${savedAt.toString(16)})`);
        
        // Restore from WHERE WE SAVED it (not from current SP, which handler may have changed)
        const restoreFrom = savedAt; // <-- FIX: use savedAt instead of savedSp
        const savedFrame = uc.mem_read(BigInt(restoreFrom), 32);
        const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
        
        const restoredPc = savedSv.getUint32(4, true);
        const restoredSpVal = restoreFrom + 32;
        console.log(`  Restoring: PC=0x${restoredPc.toString(16)} SP=0x${restoredSpVal.toString(16)}`);
        
        uc.reg_write_i32(Module.ARM_REG_R0, savedSv.getUint32(28, true));
        uc.reg_write_i32(Module.ARM_REG_R1, savedSv.getUint32(24, true));
        uc.reg_write_i32(Module.ARM_REG_R2, savedSv.getUint32(20, true));
        uc.reg_write_i32(Module.ARM_REG_R3, savedSv.getUint32(16, true));
        uc.reg_write_i32(Module.ARM_REG_R12, savedSv.getUint32(12, true));
        uc.reg_write_i32(Module.ARM_REG_LR, savedSv.getUint32(8, true));
        uc.reg_write_i32(Module.ARM_REG_PC, restoredPc | 1);
        uc.reg_write_i32(Module.ARM_REG_SP, restoredSpVal);
    }
};

// Run firmware, inject bytes
const maxBatch = 50000;
let totalSteps = 0;
const maxSteps = 200;

console.log('\n=== Starting ===\n');
uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

const helloSrc = "Hello\n";
let helloIdx = 0;

for (let step = 0; step < maxSteps && instCount < 10000000n; step++) {
    // Inject "Hello\n" bytes at beginning
    if (step === 5) {
        for (let bi = 0; bi < helloSrc.length; bi++) {
            uart_rx_byte(0x40011000, helloSrc.charCodeAt(bi));
        }
        console.log(`Step ${step}: Injected "${helloSrc}"`);
    }
    
    processInterrupts();
    
    const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
    try {
        uc.emu_start(BigInt(curPc | 1), 0n, 0n, maxBatch);
    } catch (e) {
        if (String(e).includes('UC_ERR_READ_UNMAPPED') || String(e).includes('UC_ERR_FETCH_UNMAPPED')) {
            const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
            uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
        } else {
            console.error('Fatal:', e.message || e);
            break;
        }
    }
    
    processInterrupts();
    totalSteps++;
    
    // Check if main returned
    const p = uc.reg_read_i32(Module.ARM_REG_PC);
    if (p <= 0x080001fc && p >= 0x080001f4) break;
}

const uartOut = get_uart_output();
if (uartOut) console.log(`\n=== UART Output ===\n${uartOut}`);

console.log(`\nDone: ${totalSteps} steps, ${instCount} instructions`);
uc.close();
