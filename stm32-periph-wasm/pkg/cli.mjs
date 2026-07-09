import { createRequire } from 'module';
import { readFileSync } from 'fs';

const require = createRequire(import.meta.url);

const MUnicorn = require('./unicorn_arm.cjs');
const periph = require('./stm32_periph_wasm.js');

async function main() {
    const args = process.argv.slice(2);
    const firmwarePath = args[0] || process.env.FIRMWARE;
    const maxInst = parseInt(args[1] || process.env.MAX_INST || '1000000', 10);
    const showRegs = args.includes('--regs') || process.env.SHOW_REGS === '1';

    if (!firmwarePath) {
        console.error('Usage: node cli.mjs <firmware.bin> [max_instructions] [--regs]');
        console.error('  or set FIRMWARE env var');
        process.exit(1);
    }

    const firmware = readFileSync(firmwarePath);

    console.log(`Loading firmware: ${firmwarePath} (${firmware.length} bytes)`);
    console.log(`Max instructions: ${maxInst}`);
    console.log('Initializing Unicorn...');

    const Module = await MUnicorn({});
    const periphWasmBuf = readFileSync(new URL('./stm32_periph_wasm_bg.wasm', import.meta.url));
    await periph.default({ module_or_path: periphWasmBuf.buffer });

    if (periph.add_spi_flash || periph.add_i2c_eeprom) {
        // Add ext devices if needed
    }
    const svdPath = new URL('../../monox/stm32f407.svd', import.meta.url);
    const svdXml = readFileSync(svdPath, 'utf8');
    periph.init_svd(svdXml);

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    const vector_table = 0x08000000;
    const flash_size = 0x100000;
    const ram_size = 0x20000;

    uc.mem_map(vector_table, flash_size, Module.PROT_ALL);
    uc.mem_write(vector_table, firmware);

    uc.mem_map(0x20000000, ram_size, Module.PROT_ALL);

    const periphRanges = [
        [0x40000000, 0xB0000000],
        [0xE0000000, 0xE1000000],
    ];
    for (const [start, end] of periphRanges) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
    }

    const read32 = (addr) => {
        const b = uc.mem_read(BigInt(addr), 4);
        const dt = new DataView(b.buffer, b.byteOffset, b.byteLength);
        return dt.getUint32(0, true);
    };

    const sp_init = read32(vector_table);
    const pc_init = read32(vector_table + 4);

    uc.reg_write_i32(Module.ARM_REG_SP, sp_init);
    uc.reg_write_i32(Module.ARM_REG_PC, pc_init | 1);

    console.log(`SP=0x${sp_init.toString(16)} PC=0x${(pc_init | 1).toString(16)}`);

    const memReadHook = (handle, type, address, size, value, user_data) => {
        const addr32 = Number(address);
        const val = periph.periph_read(addr32, size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (val >> (i * 8)) & 0xFF;
        }
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        periph.periph_write(Number(address), size, Number(value));
    };

    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    let instCount = 0n;
    let stopRequested = false;
    const tickInterval = 1000;

    const codeHook = (handle, address, size, user_data) => {
        instCount++;
        periph.tick();
        if (periph.is_watchdog_reset_requested()) {
            stopRequested = true;
            uc.emu_stop();
            return;
        }
        if (instCount % BigInt(tickInterval) === 0n) {
            const irq = periph.get_next_pending_interrupt();
            if (irq >= 0) {
                uc.emu_stop();
            }
        }
    };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

    const intrHook = (handle, intno, user_data) => {
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
    };
    uc.hook_add(Module.HOOK_INTR, intrHook, null);

    const processDma = () => {
        const count = periph.dma_get_pending_count();
        for (let i = 0; i < count; i++) {
            const pending = periph.dma_get_pending(0);
            if (pending.length < 5) continue;
            const dir = pending[0];
            const stream = pending[1];
            const src = pending[2];
            const dst = pending[3];
            const size = pending[4];
            const peri_addr = pending[5] || 0;
            const peripheral = pending[6] || 0;
            try {
                if (dir === 2) {
                    const data = uc.mem_read(BigInt(src), size);
                    uc.mem_write(BigInt(dst), data);
                } else if (dir === 0) {
                    const data = uc.mem_read(BigInt(src), size);
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            let val = 0;
                            for (let k = 0; k < chunk; k++) val |= data[j + k] << (k * 8);
                            periph.periph_write(peri_addr, chunk, val);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), data);
                    }
                } else if (dir === 1) {
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            const val = periph.periph_read(peri_addr, chunk);
                            const bytes = new Uint8Array(chunk);
                            for (let k = 0; k < chunk; k++) bytes[k] = (val >> (k * 8)) & 0xFF;
                            uc.mem_write(BigInt(dst + j), bytes);
                        }
                    } else {
                        const data = uc.mem_read(BigInt(src), size);
                        uc.mem_write(BigInt(dst), data);
                    }
                }
            } catch (e) {
                console.warn('DMA error:', e.message);
            }
            periph.dma_set_completed(stream, true);
        }
    };

    const processInterrupts = () => {
        while (!stopRequested) {
            const irq = periph.get_next_pending_interrupt();
            if (irq < 0) break;
            const sp = uc.reg_read_i32(Module.ARM_REG_SP);
            const pc = uc.reg_read_i32(Module.ARM_REG_PC);
            const lr = uc.reg_read_i32(Module.ARM_REG_LR);
            const xpsr = uc.reg_read_i32(Module.ARM_REG_XPSR);
            const r0 = uc.reg_read_i32(Module.ARM_REG_R0);
            const r1 = uc.reg_read_i32(Module.ARM_REG_R1);
            const r2 = uc.reg_read_i32(Module.ARM_REG_R2);
            const r3 = uc.reg_read_i32(Module.ARM_REG_R3);
            const r12 = uc.reg_read_i32(Module.ARM_REG_R12);
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
            uc.mem_write(BigInt(sp - 32), frame);
            uc.reg_write_i32(Module.ARM_REG_SP, sp - 32);
            const handler_pc = read32(vector_table + 4 + irq * 4);
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
            try {
                uc.emu_start(BigInt(handler_pc), 0n, 0n, 100000);
            } catch (e) {
                if (!stopRequested) break;
            }
            processDma();
        }
    };

    const maxBatch = 100000;
    let totalSteps = 0;
    const startTime = Date.now();

    while (!stopRequested) {
        processDma();
        const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
        try {
            uc.emu_start(BigInt(curPc | 1), 0n, 0n, maxBatch);
        } catch (e) {
            const msg = String(e);
            if (msg.includes('UC_ERR_READ_UNMAPPED') || msg.includes('UC_ERR_FETCH_UNMAPPED')) {
                const pc2 = uc.reg_read_i32(Module.ARM_REG_PC);
                uc.reg_write_i32(Module.ARM_REG_PC, (pc2 + 2) | 1);
            } else {
                console.error('Emulation error:', e.message || e);
                break;
            }
        }
        processDma();
        processInterrupts();
        totalSteps++;

        if (stopRequested || periph.is_watchdog_reset_requested()) break;
        if (totalSteps * maxBatch >= maxInst) break;
    }

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(2);
    const finalPc = uc.reg_read_i32(Module.ARM_REG_PC);
    const finalSp = uc.reg_read_i32(Module.ARM_REG_SP);

    const uartOut = periph.get_uart_output();
    if (uartOut) {
        console.log(`\n=== UART Output ===\n${uartOut}`);
    }

    console.log(`\nDone: ${totalSteps} steps, ${instCount} instructions in ${elapsed}s`);
    console.log(`PC=0x${finalPc.toString(16)} SP=0x${finalSp.toString(16)}`);

    if (showRegs) {
        for (let i = 0; i <= 12; i++) {
            const reg = uc[`reg_read_i32`](Module[`ARM_REG_R${i}`]);
            process.stdout.write(`R${i}=0x${reg.toString(16).padStart(8, '0')} `);
            if (i % 4 === 3) console.log();
        }
        console.log(`LR=0x${uc.reg_read_i32(Module.ARM_REG_LR).toString(16).padStart(8, '0')}`);
        console.log(`xPSR=0x${uc.reg_read_i32(Module.ARM_REG_XPSR).toString(16).padStart(8, '0')}`);
    }

    uc.close();
}

main().catch(e => {
    console.error('Fatal:', e.name, e.message);
    console.error('Stack:', e.stack?.substring(0, 1000));
    process.exit(1);
});
