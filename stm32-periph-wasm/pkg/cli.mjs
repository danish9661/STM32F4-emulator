import { readFileSync, writeSync } from 'fs';
import { spawn } from 'child_process';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const yaml = require('js-yaml');
const path = require('path');
import * as periph from './stm32_periph_wasm.js';
const { periph_read, periph_write, tick, tick_n, get_next_pending_interrupt, dma_get_pending_count, dma_get_pending, dma_set_completed, is_watchdog_reset_requested, add_spi_flash, add_i2c_eeprom, init_svd, has_pending_interrupt, get_uart_output, uart_rx_byte, eth_is_tx_poll, eth_get_tx_desc_addr, eth_clear_tx_poll, eth_is_rx_poll, eth_get_rx_desc_addr, eth_clear_rx_poll, eth_tx_done, eth_rx_done, eth_signal_rx_poll } = periph;

const parseHex = (v) => typeof v === 'number' ? v : parseInt(v, 16);

async function getMUnicorn() {
    const { createRequire } = await import('module');
    const require = createRequire(import.meta.url);
    return require('./unicorn_arm.cjs');
}

async function main() {
    const args = process.argv.slice(2);
    const configPaths = args.filter(a => a.startsWith('--config=')).map(a => a.split('=')[1]);
    const posArgs = args.filter(a => !a.startsWith('--'));
    const maxInst = parseInt(posArgs[1] || process.env.MAX_INST || '1000000', 10);
    const showRegs = args.includes('--regs') || process.env.SHOW_REGS === '1';
    const useGateway = (args.includes('--gateway') || args.includes('--connect')) || process.env.ETH_GATEWAY === '1';
    const spawnGateway = args.includes('--gateway') && !args.includes('--connect');
    let uartAddr = parseInt(args.find(a => a.startsWith('--uart='))?.split('=')[1] || process.env.UART_ADDR || '0x40011000', 16);

    // Load and merge configs
    let config = {};
    if (configPaths.length > 0) {
        for (const cp of configPaths) {
            const raw = yaml.load(readFileSync(cp, 'utf8'));
            const cfgDir = path.dirname(path.resolve(cp));
            if (raw.regions) raw.regions = raw.regions.map(r => ({ ...r, _dir: cfgDir }));
            if (raw.patches) raw.patches = raw.patches.map(p => ({ ...p, _dir: cfgDir }));
            raw._devices_dir = cfgDir;
            // Merge: later configs override earlier ones
            config = { ...config, ...raw, regions: [...(config.regions || []), ...(raw.regions || [])], patches: [...(config.patches || []), ...(raw.patches || [])] };
        }
        console.log(`Using config(s): ${configPaths.join(', ')}`);
    }

    const MUnicorn = await getMUnicorn();
    const Module = await MUnicorn({});

    let firmware;
    let vector_table;
    let memRegions;

    if (config.regions) {
        // Config mode
        memRegions = config.regions.map(r => ({ ...r, start: parseHex(r.start), size: parseHex(r.size) }));
        const romRegion = memRegions.find(r => r.load);
        if (!romRegion) { console.error('No region with load file found'); process.exit(1); }
        vector_table = parseHex(config.cpu?.vector_table || romRegion.start);
        const romFile = path.resolve(romRegion._dir || config._devices_dir, romRegion.load);
        firmware = readFileSync(romFile);
        console.log(`Loading firmware: ${romFile} (${firmware.length} bytes)`);

        const svdPath = path.resolve(config._devices_dir, config.cpu?.svd || 'stm32f407.svd');
        const svdXml = readFileSync(svdPath, 'utf8');
        init_svd(svdXml);

        // Patches
        if (config.patches) {
            for (const p of config.patches) {
                const start = BigInt(parseHex(p.start));
                const data = new Uint8Array(p.data);
                const romRegionStart = BigInt(romRegion.start);
                const relOff = Number(start - romRegionStart);
                if (relOff >= 0 && relOff + data.length <= firmware.length) {
                    data.forEach((b, i) => firmware[relOff + i] = b);
                    console.log(`Applied patch at 0x${start.toString(16)}: [${data.join(', ')}]`);
                }
            }
        }
    } else {
        // Default fallback (no config)
        const firmwarePath = posArgs[0] || process.env.FIRMWARE;
        if (!firmwarePath) {
            console.error('Usage: node cli.mjs <firmware.bin> [max_instructions] [--config=path]');
            console.error('  or set FIRMWARE env var');
            process.exit(1);
        }
        firmware = readFileSync(firmwarePath);
        console.log(`Loading firmware: ${firmwarePath} (${firmware.length} bytes)`);

        const fwDir = firmwarePath.replace(/\\/g, '/').replace(/\/[^/]+$/, '');
        for (const fn of ['eeprom.bin', 'spi_flash.bin']) {
            try {
                const data = readFileSync(`${fwDir}/${fn}`);
                if (fn.startsWith('eeprom')) add_i2c_eeprom("I2C1", 0x50, data);
                else add_spi_flash("SPI3", 0xef4016, data, null);
                console.log(`Loaded ext device: ${fwDir}/${fn} (${data.length} bytes)`);
            } catch (_) {}
        }

        const svdPath = new URL('../../monox/stm32f407.svd', import.meta.url);
        const svdXml = readFileSync(svdPath, 'utf8');
        init_svd(svdXml);

        vector_table = 0x08000000;
        memRegions = [
            { start: 0x08000000, size: 0x100000 },
            { start: 0x20000000, size: 0x20000 },
        ];
    }

    console.log(`Max instructions: ${maxInst}`);
    console.log('Initializing Unicorn...');

    const uc = new Module.Unicorn(
        Module.ARCH_ARM,
        Module.MODE_THUMB | Module.MODE_LITTLE_ENDIAN
    );

    // Map memory regions
    for (const r of memRegions) {
        uc.mem_map(r.start, r.size, Module.PROT_ALL);
    }
    // Write firmware into first writable-mapped ROM region
    const romRegion = memRegions.find(r => firmware && (r._firmware || r.load || (r.start <= vector_table && r.start + r.size > vector_table)));
    const romStart = romRegion ? romRegion.start : (memRegions[0]?.start || 0x08000000);
    if (firmware) uc.mem_write(BigInt(romStart), firmware);

    // Also write firmware to the exact vector_table region if different
    if (romStart !== vector_table) uc.mem_write(BigInt(vector_table), firmware);

    // Peripheral ranges
    const periphRanges = [
        [0x40000000, 0xB0000000],
        [0xE0000000, 0xE1000000],
    ];
    for (const [start, end] of periphRanges) {
        uc.mem_map(start, end - start, Module.PROT_READ | Module.PROT_WRITE);
    }

    // Config devices
    if (config.devices) {
        for (const [type, devs] of Object.entries(config.devices)) {
            for (const d of devs || []) {
                if (type === 'i2c_eeprom') {
                    const data = d.file ? readFileSync(path.resolve(config._devices_dir, d.file)) : new Uint8Array(d.size || 0);
                    add_i2c_eeprom(d.peripheral, parseHex(d.addr), data);
                } else if (type === 'spi_flash') {
                    const data = d.file ? readFileSync(path.resolve(config._devices_dir, d.file)) : new Uint8Array(d.size || 0);
                    add_spi_flash(d.peripheral, parseHex(d.jedec_id), data, null);
                } else if (type === 'usart_probe') {
                    uartAddr = parseHex(d.peripheral.match(/[0-9a-fA-F]+/)?.[0]) ? parseInt(d.peripheral, 16) : (PERIPH_ADDR[d.peripheral] || uartAddr);
                }
            }
        }
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
        const val = periph_read(addr32, size) >>> 0;
        const bytes = new Uint8Array(size);
        for (let i = 0; i < size; i++) {
            bytes[i] = (val >> (i * 8)) & 0xFF;
        }
        uc.mem_write(address, bytes);
    };

    const memWriteHook = (handle, type, address, size, value, user_data) => {
        const a = Number(address);
        if (process.env.DBG_DMA && (a === 0x40029004 || a === 0x40029018 || a === 0x40029010 || a === 0x40029014)) {
            let pc = 0;
            try { pc = uc.reg_read_i32(Module.ARM_REG_PC); } catch (_) {}
            console.log(`[DMA] wr 0x${a.toString(16)} = 0x${(Number(value) >>> 0).toString(16)} pc=0x${(pc >>> 0).toString(16)}`);
        }
        if (process.env.DBG_DMA2 && a >= 0x40029000 && a <= 0x4002901c && (a & 3) === 0) {
            let pc = 0;
            try { pc = uc.reg_read_i32(Module.ARM_REG_PC); } catch (_) {}
            console.log(`[DMA2] wr 0x${a.toString(16)} = 0x${(Number(value) >>> 0).toString(16)} pc=0x${(pc >>> 0).toString(16)}`);
        }
        periph_write(a, size, Number(value));
    };

    for (const [start, end] of periphRanges) {
        uc.hook_add(Module.HOOK_MEM_READ, memReadHook, null, start, end);
        uc.hook_add(Module.HOOK_MEM_WRITE, memWriteHook, null, start, end);
    }

    if (process.env.DBG_FLAG) {
        const flagHook = (handle, type, address, size, value, user_data) => {
            const a = Number(address);
            if (a >= 0x20000618 && a < 0x20000628) {
                const pc = (uc.reg_read_i32(Module.ARM_REG_PC) >>> 0).toString(16);
                const val = (Number(value) >>> 0).toString(2).padStart(32, '0');
                console.log(`[FLAG] ${type === 2 ? 'WR' : 'rd'} 0x${a.toString(16)} = ${val} pc=0x${pc}`);
            }
        };
        uc.hook_add(Module.HOOK_MEM_READ, flagHook, null, 0x20000600n, 0x20000640n);
        uc.hook_add(Module.HOOK_MEM_WRITE, flagHook, null, 0x20000600n, 0x20000640n);
    }

    let instCount = 0n;
    let stopRequested = false;
    const TICK_EVERY = 5000;
    const POLL_EVERY = 1000;
    let tickAcc = 0;
    let pollAcc = 0;

    const codeHook = (handle, address, size, user_data) => {
        instCount++;
        if (process.env.DBG_PC) {
            const a = Number(address) & 0xFFFFFFFE;
            if ((a >= 0x8001080 && a <= 0x8001110)) {
                if (a === 0x8001080 || a === 0x8001088 || a === 0x800108c || a === 0x8001098 || a === 0x8001092 || a === 0x80010b6 || a === 0x80010f8 || a === 0x8001100) {
                    console.log(`[PC] 0x${a.toString(16)} inst=${instCount}`);
                }
            }
        }
        if (gwRxQueue.length > 0 && eth_is_rx_poll()) {
            uc.emu_stop();
            return;
        }
        tickAcc++;
        if (tickAcc >= TICK_EVERY) {
            tickAcc = 0;
            tick_n(TICK_EVERY);
            if (is_watchdog_reset_requested()) {
                stopRequested = true;
                uc.emu_stop();
                return;
            }
            if (has_pending_interrupt()) {
                uc.emu_stop();
                return;
            }
        }
        pollAcc++;
        if (pollAcc >= POLL_EVERY) {
            pollAcc = 0;
            if (dma_get_pending_count() > 0 || eth_is_tx_poll()) {
                uc.emu_stop();
                return;
            }
        }
    };
    uc.hook_add(Module.HOOK_CODE, codeHook, null);

    // Gateway networking
    let gwProcess = null;
    let gwWs = null;
    let gwDialSeq = 0;
    const gwRxQueue = [];
    const gwTxPending = [];
    let gwConnected = false;
    let gwRestarts = 0;

    const connectGateway = () => new Promise((resolve) => {
        let ws;
        let timedOut = false;
        gwDialSeq++;
        if (process.env.DBG_GW) console.log(`[GW] dial #${gwDialSeq} at ${Date.now()}`);
        try { ws = new WebSocket('ws://127.0.0.1:5099/api/network-gateway'); }
        catch (e) { resolve(null); return; }
        ws.binaryType = 'arraybuffer';
        ws.onclose = (ev) => { if (process.env.DBG_GW) console.log(`[GW] dial #${gwDialSeq} closed code=${ev.code}`); };
        const to = setTimeout(() => { timedOut = true; try { ws.close(); } catch (_) {} resolve(null); }, 4000);
        ws.onerror = () => { clearTimeout(to); resolve(null); };
        ws.onopen = () => {
            clearTimeout(to);
            if (timedOut) { try { ws.close(); } catch (_) {} resolve(null); return; }
            ws.onmessage = (ev) => {
                if (typeof ev.data === 'string') return;
                let buf;
                if (ArrayBuffer.isView(ev.data)) {
                    buf = new Uint8Array(ev.data.buffer, ev.data.byteOffset, ev.data.byteLength);
                } else if (ev.data instanceof ArrayBuffer) {
                    buf = new Uint8Array(ev.data);
                } else {
                    return;
                }
                gwRxQueue.push(buf);
                if (process.env.DBG_RX) console.log(`[RX] ws msg ${buf.length}B, queue=${gwRxQueue.length}`);
                if (process.env.DBG_PC2 && typeof uc !== 'undefined') {
                    let pcr = 0;
                    try { pcr = uc.reg_read_i32(Module.ARM_REG_PC); } catch (_) {}
                    console.log(`[GWF] guest PC=0x${(pcr >>> 0).toString(16)} rxQ=${gwRxQueue.length}`);
                }
            };
            ws.onclose = () => { if (gwWs === ws) { gwConnected = false; console.log('Gateway WebSocket disconnected'); } };
            gwWs = ws;
            gwConnected = true;
            console.log('Gateway WebSocket connected');
            while (gwTxPending.length > 0) {
                try { ws.send(gwTxPending.shift()); } catch (_) { gwTxPending.unshift(); break; }
            }
            resolve(ws);
        };
    });

    const startGateway = async () => {
        if (!useGateway) return;
        try {
            if (spawnGateway) {
                const gwPath = process.env.GW_PATH || path.join(import.meta.dirname, '..', '..', 'openhw-local-gateway', 'openhw-gw');
                gwProcess = spawn(gwPath, [], { stdio: 'pipe' });
                gwProcess.stdout.on('data', d => process.stdout.write(d));
                gwProcess.stderr.on('data', d => process.stderr.write(d));
                gwProcess.on('error', e => console.warn('Gateway error:', e.message));
                gwProcess.on('exit', c => console.log(`Gateway exited (code ${c})`));
            }
            const deadline = Date.now() + 8000;
            while (Date.now() < deadline) {
                const ws = await connectGateway();
                if (ws) return;
                await new Promise(r => setTimeout(r, 600));
            }
            console.warn('WebSocket timeout');
        } catch (e) {
            console.warn('Gateway startup failed:', e.message);
        }
    };

    // A DHCP reply is the only stateless frame worth keeping across a gateway
    // restart: Ethernet(14) | IPv4 | UDP dst port 68.
    const isDhcpReply = (buf) => {
        if (buf.length < 42) return false;
        if (buf[12] !== 0x08 || buf[13] !== 0x00) return false;
        if (buf[23] !== 17) return false;
        return buf[36] === 0 && buf[37] === 68;
    };

    const restartGateway = async () => {
        if (!spawnGateway) {
            // --connect mode: the gateway is external, ask it to reset its
            // gVisor session state via a control message, then reconnect.
            gwRestarts++;
            console.log(`\n[GW] requesting gateway session reset for round ${gwRestarts + 1}...`);
            if (gwWs && gwConnected) {
                try { gwWs.send('RESET'); } catch (_) {}
            }
            const deadline = Date.now() + 2000;
            while (gwConnected && Date.now() < deadline) await new Promise(r => setTimeout(r, 50));
            if (gwWs) try { gwWs.close(); } catch (_) {}
            gwWs = null;
            // Drop stale TCP frames from the dying session; keep only DHCP
            // replies (the firmware's next-round handshake must not see
            // old-session data).
            for (let i = gwRxQueue.length - 1; i >= 0; i--) {
                if (!isDhcpReply(gwRxQueue[i])) gwRxQueue.splice(i, 1);
            }
            await startGateway();
            return;
        }
        gwRestarts++;
        console.log(`\n[GW] restarting gateway for round ${gwRestarts + 1}...`);
        if (gwWs) try { gwWs.close(); } catch (_) {}
        gwWs = null;
        gwConnected = false;
        // Drop stale TCP frames from the dying session; keep only DHCP replies
        // (the firmware's next-round handshake must not see old-session data).
        for (let i = gwRxQueue.length - 1; i >= 0; i--) {
            if (!isDhcpReply(gwRxQueue[i])) gwRxQueue.splice(i, 1);
        }
        if (gwProcess) {
            const old = gwProcess;
            try { old.kill(); } catch (_) {}
            await new Promise(r => { if (old.exitCode !== null) r(); else old.once('exit', r); });
        }
        gwProcess = null;
        await startGateway();
    };

    // Round markers seen in UART (each round ends with "=== HTTP nnb ===")
    let gwRoundsSeen = 0;
    const checkGwRestart = (chunk) => {
        if (!useGateway || !chunk.includes('=== HTTP ')) return false;
        const markers = (chunk.match(/=== HTTP .* ===/g) || []).length;
        if (markers === 0) return false;
        gwRoundsSeen += markers;
        return true;
    };

    await startGateway();

    // Stdin -> UART RX
    const stdinQueue = [];
    if (process.stdin.isTTY) process.stdin.setRawMode(true);
    process.stdin.on('data', (chunk) => { for (const b of chunk) stdinQueue.push(b); });
    process.stdin.resume();
    if (process.stdin.isTTY) process.on('SIGINT', () => { process.stdin.setRawMode(false); process.exit(0); });

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

    const processEth = (uc) => {
        if (eth_is_tx_poll()) {
            const ta = eth_get_tx_desc_addr();
            const txDescAddr = eth_get_tx_desc_addr();
            if (process.env.DBG_TX) console.log(`[TX!] poll desc=0x${txDescAddr.toString(16)}`);
            if (txDescAddr !== 0) {
                let descAddr = txDescAddr;
                const seen = new Set();
                while (descAddr !== 0 && !seen.has(descAddr)) {
                    seen.add(descAddr);
                    const desc = uc.mem_read(BigInt(descAddr), 8);
                    const dv = new DataView(desc.buffer, desc.byteOffset, desc.byteLength);
                    const tdes0 = dv.getUint32(0, true);
                    const tdes1 = dv.getUint32(4, true);
                    if ((tdes0 & 0x80000000) === 0) break;
                    const bufAddr = tdes1 & 0xFFFFFFFC;
                    const bufSize = (tdes0 & 0x3FFF);
                    if (bufAddr !== 0 && bufSize > 0 && bufSize <= 2000) {
                        const pkt = new Uint8Array(uc.mem_read(BigInt(bufAddr), bufSize));
                        if (gwConnected && gwWs?.readyState === WebSocket.OPEN) {
                            if (process.env.DBG_TX) console.log(`[TX] ${bufSize}B -> ws`);
                            gwWs.send(pkt);
                        } else if (useGateway) {
                            if (process.env.DBG_TX) console.log(`[TX] ${bufSize}B queued (${gwConnected ? 'not-open' : 'disconnected'})`);
                            gwTxPending.push(pkt);
                        } else {
                            console.log(`ETH TX ${bufSize} byte(s) from 0x${bufAddr.toString(16)}`);
                        }
                    }
                    const ownClear = tdes0 & ~0x80000000;
                    const status = ownClear | 0x20000000;
                    const wb = new Uint8Array(4);
                    new DataView(wb.buffer).setUint32(0, status, true);
                    uc.mem_write(BigInt(descAddr), wb);
                    if (tdes0 & (1 << 22)) descAddr = tdes1 & 0xFFFFFFFC;
                    else descAddr = descAddr + 8;
                }
            }
            eth_clear_tx_poll();
            eth_tx_done();
        }
        if (eth_is_rx_poll()) {
            const rxDescAddr = eth_get_rx_desc_addr();
            let rxDelivered = 0;
            if (process.env.DBG_RXP) console.log(`[RXP] rx_poll=true queue=${gwRxQueue.length} desc=0x${rxDescAddr.toString(16)} inst=${instCount}`);
            if (rxDescAddr !== 0 && gwRxQueue.length > 0) {
                if (process.env.DBG_RX) console.log(`[RX] poll, queue=${gwRxQueue.length}, desc=0x${rxDescAddr.toString(16)}`);
                let descAddr = rxDescAddr;
                const seen = new Set();
                let attempts = 0;
                while (descAddr !== 0 && !seen.has(descAddr) && gwRxQueue.length > 0 && attempts < 1) {
                    attempts++;
                    seen.add(descAddr);
                    let desc;
                    try { desc = uc.mem_read(BigInt(descAddr), 8); } catch (e) { break; }
                    const dv = new DataView(desc.buffer, desc.byteOffset, desc.byteLength);
                    const rdes0 = dv.getUint32(0, true);
                    if ((rdes0 & 0x80000000) === 0) break;
                    const rdes1 = dv.getUint32(4, true);
                    const bufAddr = rdes1 & 0xFFFFFFFC;
                    const bufSize = (rdes0 & 0x3FFF);
                    if (bufAddr !== 0 && bufSize >= 60) {
                        const pkt = gwRxQueue.shift();
                        const len = Math.min(pkt.length, bufSize);
                        if (process.env.RX_HEX === '1') {
                            let hex = [];
                            for (let i = 0; i < len && i < 64; i++) hex.push(pkt[i].toString(16).padStart(2, '0'));
                            console.log(`[RXHEX len=${len}] ${hex.join('')}`);
                        }
                        try { uc.mem_write(BigInt(bufAddr), new Uint8Array(pkt.buffer, pkt.byteOffset, len)); } catch (e) { break; }
                        const rdes0_w = (1 << 28) | (1 << 27) | (len << 16);
                        const wb = new Uint8Array(4);
                        new DataView(wb.buffer).setUint32(0, rdes0_w, true);
                        try { uc.mem_write(BigInt(descAddr), wb); } catch (e) { break; }
                        rxDelivered = 1;
                    }
                    if (rdes1 & (1 << 29)) descAddr = rdes1 & 0xFFFFFFFC;
                    else descAddr = descAddr + 8;
                }
            }
            eth_clear_rx_poll();
            if (rxDelivered) {
                eth_rx_done();
                // Re-arm RX poll if more packets pending so next iteration gets a separate IRQ
                if (gwRxQueue.length > 0) {
                    const rda = eth_get_rx_desc_addr();
                    if (rda !== 0) eth_signal_rx_poll(rda);
                }
            }
        }
    };

    const processDma = () => {
        const count = dma_get_pending_count();
        for (let i = 0; i < count; i++) {
            const pending = dma_get_pending(0);
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
                            periph_write(peri_addr, chunk, val);
                        }
                    } else {
                        uc.mem_write(BigInt(dst), data);
                    }
                } else if (dir === 1) {
                    if (peripheral) {
                        for (let j = 0; j < size; j += 4) {
                            const chunk = Math.min(4, size - j);
                            const val = periph_read(peri_addr, chunk);
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
            dma_set_completed(stream, true);
        }
    };

    let lastUartLen = 0;
    let uartStableCount = 0;

    const processInterrupts = () => {
        while (!stopRequested) {
            const irq = get_next_pending_interrupt();
            if (irq <= -100) break;
            if (process.env.DBG_IRQ && (irq === 58 || irq === 61)) console.log(`[IRQ] ETH handler`);
            // console.log(`DEBUG: IRQ ${irq} at inst ${instCount}`);

            const savedAt = uc.reg_read_i32(Module.ARM_REG_SP);
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
            uc.mem_write(BigInt(savedAt - 32), frame);
            uc.reg_write_i32(Module.ARM_REG_SP, savedAt - 32);
            const handler_pc = read32(vector_table + 4 * (16 + irq));
            uc.reg_write_i32(Module.ARM_REG_LR, 0xFFFFFFF9);
            uc.reg_write_i32(Module.ARM_REG_PC, handler_pc);
            if (process.env.DBG_IRQF) {
                console.log(`[IRQF] irq=${irq} savedPC=0x${(pc & 0xFFFFFFFE).toString(16)} SP=0x${savedAt.toString(16)} r0=0x${r0.toString(16)} r1=0x${r1.toString(16)} r2=0x${r2.toString(16)} r3=0x${r3.toString(16)}`);
            }
            try {
                uc.emu_start(BigInt(handler_pc), 0n, 0n, 100000);
            } catch (e) {
                if (process.env.DBG_IRQF) console.log(`[IRQF!] ISR aborted: ${String(e).slice(0, 60)}`);
                // Handler crashed on BX LR (EXC_RETURN not supported)
            }
            const restoredR3 = uc.reg_read_i32(Module.ARM_REG_R3);
            const restoredPC = uc.reg_read_i32(Module.ARM_REG_PC) & 0xFFFFFFFE;
            if (process.env.DBG_IRQF) {
                console.log(`[IRQF] after-ISR r3=0x${(restoredR3 >>> 0).toString(16)} pc=0x${restoredPC.toString(16)}`);
            }
            if (process.env.DBG_IRQSR) {
                const hp = has_pending_interrupt();
                console.log(`[IRQSR] has_pending_after_ISR: ${hp}`);
            }
            // Restore context from where we saved it (handlers may modify SP)
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
            processEth(uc);
        }
    };

    let maxBatch = 500000;
    let smallBatch = false;
    let totalSteps = 0;
    const startTime = Date.now();

    while (!stopRequested) {
        const uartChunk = get_uart_output();
        if (uartChunk) {
            process.stdout.write(uartChunk);
            if (checkGwRestart(uartChunk)) await restartGateway();
            if (!smallBatch && uartChunk.includes('!CONN')) {
                smallBatch = true;
                console.log('[GW] round end detected, switching to small batches');
            }
            if (smallBatch && uartChunk.includes('Offer IP=')) {
                smallBatch = false;
                console.log('[GW] DHCP re-established, restoring large batches');
            }
        }
        while (stdinQueue.length > 0) uart_rx_byte(uartAddr, stdinQueue.shift());

        processDma();
        processEth(uc);
        tick();
        const curPc = uc.reg_read_i32(Module.ARM_REG_PC);
        try {
            uc.emu_start(BigInt(curPc | 1), 0n, 0n, smallBatch ? 1500 : maxBatch);
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
        processEth(uc);
        processInterrupts();
        totalSteps++;
        if (process.env.DBG_PC3) {
            const pc3 = uc.reg_read_i32(Module.ARM_REG_PC) & 0xFFFFFFFE;
            console.log(`[PC3] 0x${pc3.toString(16).padStart(8, '0')} n=${totalSteps} inst=${instCount}`);
        }
        if (process.env.SOAK_STATS && totalSteps % 2500 === 0) {
            const rssMB = (process.memoryUsage().rss / 1048576).toFixed(0);
            const line = `[SOAK] t=${((Date.now() - startTime) / 1000).toFixed(0)}s inst=${instCount} rxQ=${gwRxQueue.length} txQ=${gwTxPending.length} rounds=${gwRoundsSeen} rss=${rssMB}MB\n`;
            writeSync(1, line);
        }

        if (stopRequested || is_watchdog_reset_requested()) break;
        if (instCount >= BigInt(maxInst)) break;
        await new Promise(r => setImmediate(r));
    }

    if (gwWs) try { gwWs.close(); } catch (_) {}
    if (gwProcess) try { gwProcess.kill(); } catch (_) {}

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(2);
    const finalPc = uc.reg_read_i32(Module.ARM_REG_PC);
    const finalSp = uc.reg_read_i32(Module.ARM_REG_SP);

    const uartOut = get_uart_output();
    if (!uartOut.trim()) {
        process.stdout.write('\n');
    } else {
        console.log(`\n=== UART Output ===\n${uartOut}`);
    }

    try { if (process.stdin.isTTY) process.stdin.setRawMode(false); } catch (_) {}

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
    console.error('Fatal:', typeof e, String(e));
    process.exit(1);
});

