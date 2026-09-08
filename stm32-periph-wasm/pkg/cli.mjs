import { readFileSync } from 'fs';
import { spawn } from 'child_process';
import { createRequire } from 'module';
const require = createRequire(import.meta.url);
const yaml = require('js-yaml');
const path = require('path');
// Rust CPU backend (sole backend): the shared site emulator factory plus the
// current peripheral-model build. No Unicorn anywhere in this path.
import * as bindings from '../../site/vendor/stm32_periph_wasm.js';
import { createEmulator } from '../../site/emulator.js';

const parseHex = (v) => typeof v === 'number' ? v : parseInt(v, 16);

// usart_probe-style names (or hex) -> USART base for stdin UART injection.
const UART_BASE = {
    USART1: 0x40011000, USART2: 0x40004400, USART3: 0x40004800,
    UART4: 0x40004C00, UART5: 0x40005000, USART6: 0x40011400,
};
const parseUartAddr = (name, fallback) => {
    if (name == null) return fallback;
    const key = String(name).toUpperCase();
    if (UART_BASE[key] !== undefined) return UART_BASE[key];
    const n = parseInt(key, 16);
    return Number.isNaN(n) ? fallback : n;
};

async function main() {
    const args = process.argv.slice(2);
    const configPaths = args.filter(a => a.startsWith('--config=')).map(a => a.split('=')[1]);
    const posArgs = args.filter(a => !a.startsWith('--'));
    const maxInst = parseInt(posArgs[1] || process.env.MAX_INST || '1000000', 10);
    const showRegs = args.includes('--regs') || process.env.SHOW_REGS === '1';
    const useGateway = (args.includes('--gateway') || args.includes('--connect')) || process.env.ETH_GATEWAY === '1';
    const spawnGateway = args.includes('--gateway') && !args.includes('--connect');
    let uartAddr = parseHex(args.find(a => a.startsWith('--uart='))?.split('=')[1] || process.env.UART_ADDR || '0x40011000');

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

    const svdXml = readFileSync(new URL('../../monox/stm32f407.svd', import.meta.url), 'utf8');
    const wasmBytes = new Uint8Array(readFileSync(new URL('../../site/vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));

    let firmware;
    let vector_table = 0x08000000;
    let ram_size = 0x20000;
    const extra_ram = [];
    const ext_devices = {};

    if (config.regions) {
        // Config mode
        const memRegions = config.regions.map(r => ({ ...r, start: parseHex(r.start), size: parseHex(r.size) }));
        const romRegion = memRegions.find(r => r.load);
        if (!romRegion) { console.error('No region with load file found'); process.exit(1); }
        vector_table = parseHex(config.cpu?.vector_table || romRegion.start);
        const romFile = path.resolve(romRegion._dir || config._devices_dir, romRegion.load);
        firmware = new Uint8Array(readFileSync(romFile));
        console.log(`Loading firmware: ${romFile} (${firmware.length} bytes)`);

        // QSPI flash must be registered BEFORE init: the QUADSPI peripheral
        // binds its flash backend at construction time.
        if (config.devices && config.devices.qspi) {
            ext_devices.qspi = [];
            for (const d of config.devices.qspi) {
                const data = d.file ? new Uint8Array(readFileSync(path.resolve(config._devices_dir, d.file))) : new Uint8Array(d.size || 256);
                ext_devices.qspi.push({ peripheral: d.peripheral || 'QUADSPI', data });
                console.log(`Loaded QSPI flash (${data.length} bytes)`);
            }
        }

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

        // Non-ROM/RAM, non-model regions become plain extra RAM. Peripheral
        // (0x40000000+) and system (0xE0000000+) space belongs to the model.
        for (const r of memRegions) {
            if (r === romRegion) continue;
            if (r.start >= 0x40000000 && r.start < 0xB0000000) continue;
            if (r.start >= 0xE0000000 && r.start < 0xE1000000) continue;
            if (r.start <= 0x20000000 && r.start + r.size > 0x20000000) {
                ram_size = r.size;
                continue;
            }
            extra_ram.push({ addr: r.start, size: r.size });
        }
    } else {
        // Default fallback (no config)
        const firmwarePath = posArgs[0] || process.env.FIRMWARE;
        if (!firmwarePath) {
            console.error('Usage: node cli.mjs <firmware.bin> [max_instructions] [--config=path]');
            console.error('  or set FIRMWARE env var');
            process.exit(1);
        }
        firmware = new Uint8Array(readFileSync(firmwarePath));
        console.log(`Loading firmware: ${firmwarePath} (${firmware.length} bytes)`);

        const fwDir = firmwarePath.replace(/\\/g, '/').replace(/\/[^/]+$/, '');
        for (const fn of ['eeprom.bin', 'spi_flash.bin']) {
            try {
                const data = new Uint8Array(readFileSync(`${fwDir}/${fn}`));
                if (fn.startsWith('eeprom')) {
                    (ext_devices.i2c_eeprom ||= []).push({ peripheral: 'I2C1', address: 0x50, data });
                } else {
                    (ext_devices.spi_flash ||= []).push({ peripheral: 'SPI3', jedec_id: 0xef4016, data, cs: null });
                }
                console.log(`Loaded ext device: ${fwDir}/${fn} (${data.length} bytes)`);
            } catch (_) {}
        }

        // QSPI flash must be registered BEFORE init. For a qspi firmware we
        // use an adjacent qspi_flash.bin if present, else a default 256-byte
        // (blank) image so the indirect write/read round-trip still works.
        if (firmwarePath.toLowerCase().includes('qspi')) {
            try {
                const data = new Uint8Array(readFileSync(`${fwDir}/qspi_flash.bin`));
                (ext_devices.qspi ||= []).push({ peripheral: 'QUADSPI', data });
                console.log(`Loaded QSPI flash: ${fwDir}/qspi_flash.bin (${data.length} bytes)`);
            } catch (_) {
                (ext_devices.qspi ||= []).push({ peripheral: 'QUADSPI', data: new Uint8Array(256) });
                console.log('Loaded default 256-byte QSPI flash');
            }
        }
    }

    console.log(`Max instructions: ${maxInst}`);
    console.log('Booting Rust CPU backend...');

    // Config devices
    if (config.devices) {
        for (const [type, devs] of Object.entries(config.devices)) {
            for (const d of devs || []) {
                if (type === 'i2c_eeprom') {
                    const data = d.file ? new Uint8Array(readFileSync(path.resolve(config._devices_dir, d.file))) : new Uint8Array(d.size || 0);
                    (ext_devices.i2c_eeprom ||= []).push({ peripheral: d.peripheral, address: parseHex(d.addr), data });
                } else if (type === 'spi_flash') {
                    const data = d.file ? new Uint8Array(readFileSync(path.resolve(config._devices_dir, d.file))) : new Uint8Array(d.size || 0);
                    (ext_devices.spi_flash ||= []).push({ peripheral: d.peripheral, jedec_id: parseHex(d.jedec_id), data, cs: d.cs ?? null });
                } else if (type === 'usart_probe') {
                    uartAddr = parseUartAddr(d.peripheral, uartAddr);
                } else if (type === 'qspi') {
                    const data = d.file ? new Uint8Array(readFileSync(path.resolve(config._devices_dir, d.file))) : new Uint8Array(d.size || 256);
                    (ext_devices.qspi ||= []).push({ peripheral: d.peripheral || 'QUADSPI', data });
                }
            }
        }
    }

    // TX frames captured from the guest this step (sent to the gateway /
    // logged after the step so a round-boundary restart happens first).
    const stepTx = [];
    // Driver mode: eth_http is a POLLING firmware (its ETH_IRQHandler must
    // NOT run — it would stomp the driver's idx/len bookkeeping, and its
    // first-match descriptor scan starves an older-indexed frame whenever a
    // newer frame reuses d0, which loses the HTTP body at Rust-core speed).
    // Pure polling (driver-owned SRAM flags + idx rotation) is the proven
    // browser-gateway path. True IRQ firmware (eth_dhcp/eth_test/...) keeps
    // the ISR-owns-flags mode of the old pump CLI.
    const fwName = (configPaths[0] || posArgs[0] || process.env.FIRMWARE || '').toLowerCase();
    const pollingEth = fwName.includes('eth_http');
    const emu = await createEmulator({
        firmware, bindings, svdXml, wasmInit: wasmBytes,
        vector_table, ram_size, extra_ram, ext_devices, uart_addr: uartAddr,
        // Interrupt-driven delivery: the guest ETH_IRQHandler owns its SRAM
        // flags (the driver only signals the model + injects frames), exactly
        // like the old ISR-pump CLI. Covers both polling (eth_http) and
        // IRQ-driven (eth_dhcp/eth_test) firmware.
        enable_irqs: !pollingEth, irq_eth: !pollingEth,
        onTx: (pkt) => stepTx.push(pkt),
    });

    const regs0 = emu.getRegisters();
    console.log(`SP=0x${regs0.SP.toString(16)} PC=0x${regs0.PC.toString(16)}`);

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
        ws.onerror = () => { clearTimeout(to); if (process.env.DBG_GW) console.log(`[GW] dial #${gwDialSeq} ERROR`); resolve(null); };
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
        if (process.env.DBG_GW) console.log('[GW] old gateway dead, spawning fresh');
        await startGateway();
    };

    // Round markers seen in UART (each round ends with "=== HTTP nnb ===")
    let gwRoundsSeen = 0;
    let gwRestartPending = false;
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

    const sendTx = (pkt, meta) => {
        if (process.env.DBG_TX) console.log(`[TX] ${pkt.length}B${meta?.bufAddr !== undefined ? ` from 0x${meta.bufAddr.toString(16)}` : ''} -> ws ${[...pkt.subarray(0, 48)].map((b) => b.toString(16).padStart(2, '0')).join('')}`);
        if (gwConnected && gwWs?.readyState === WebSocket.OPEN) {
            gwWs.send(pkt);
        } else if (useGateway) {
            if (process.env.DBG_TX) console.log(`[TX] ${pkt.length}B queued (${gwConnected ? 'not-open' : 'disconnected'})`);
            gwTxPending.push(pkt);
        } else {
            console.log(`ETH TX ${pkt.length} byte(s)${meta?.bufAddr !== undefined ? ` from 0x${meta.bufAddr.toString(16)}` : ''}`);
        }
    };

    // Default batch budget. The old Unicorn CLI capped batches against the
    // ~40k-instruction WASM wedge; the Rust core has no such limit, but the
    // same cadence keeps gateway RX servicing prompt. MAX_BATCH overrides.
    let maxBatch = Number(process.env.MAX_BATCH) || 200000;
    let smallBatch = false;
    let totalSteps = 0;
    let instCount = 0;
    let dbgPrevSig = '';
    const startTime = Date.now();

    // eslint-disable-next-line no-constant-condition
    while (true) {
        // Feed queued gateway frames before stepping so RX is serviced promptly.
        while (gwRxQueue.length > 0) {
            const f = gwRxQueue.shift();
            if (process.env.DBG_RX) console.log(`[RX] inject ${f.length}B`);
            if (process.env.RX_HEX === '1') {
                let hex = [];
                for (let i = 0; i < f.length && i < 64; i++) hex.push(f[i].toString(16).padStart(2, '0'));
                console.log(`[RXHEX len=${f.length}] ${hex.join('')}`);
            }
            emu.injectFrame(f);
        }
        if (stdinQueue.length > 0) emu.sendUart(stdinQueue.splice(0));

        let r;
        try {
            r = emu.step(smallBatch ? 1500 : maxBatch);
        } catch (e) {
            console.error('Emulation error:', e.message || e);
            break;
        }
        instCount = r.instCount;
        if (process.env.DBG_RXDESC === '2') {
            // Per-step RX trace: flag/idx/len + all four rdes0 (only on change).
            const rr = (a) => emu.read32(a) >>> 0;
            const sig = [0x20000620, 0x20000628, 0x2000062c, 0x20000630, 0x20000638, 0x20000640, 0x20000648, 0x20000000, 0x20000654].map(rr).join(',');
            if (sig !== dbgPrevSig) {
                dbgPrevSig = sig;
                console.log(`[RXT step=${totalSteps} inst=${instCount}] f=${rr(0x20000620).toString(16)} idx=${rr(0x20000628)} len=${rr(0x2000062c)} sport=${rr(0x20000000).toString(16)} ack=${rr(0x20000654).toString(16)} d=${[0, 1, 2, 3].map((i) => rr(0x20000630 + i * 8).toString(16)).join('/')}`);
            }
        }
        const uartChunk = emu.drainUart() || '';
        if (uartChunk) {
            process.stdout.write(uartChunk);
            if (checkGwRestart(uartChunk)) {
                if (process.env.GW_RESTART) {
                    // GW_RESTART=1: kill+respawn the gateway per round (slow,
                    // ~0.7s/round). Default: no restart — gVisor opens a fresh
                    // session per connection and the firmware skips stale
                    // frames (TCP fl=18), so consecutive rounds work at speed.
                    gwRestartPending = true;
                    console.log(`[GW] round end detected (${gwRoundsSeen}), restart deferred to next-round TX`);
                }
            }
            if (!smallBatch && uartChunk.includes('!CONN')) {
                smallBatch = true;
                console.log('[GW] round end detected, switching to small batches');
            }
            if (smallBatch && uartChunk.includes('Offer IP=')) {
                smallBatch = false;
                console.log('[GW] DHCP re-established, restoring large batches');
            }
        }
        // Round boundary: the guest already printed its round marker and is
        // now TXing the next round's first frame. Restart the gateway BEFORE
        // the frame goes out, so the fresh stack sees the new transaction.
        if (gwRestartPending && stepTx.length > 0) {
            gwRestartPending = false;
            console.log(`[GW] restarting gateway on next-round TX (${stepTx[0].length}B)...`);
            await restartGateway();
        }
        for (const pkt of stepTx.splice(0)) sendTx(pkt, {});
        if (r.stopped) {
            console.log(`\n[STOP] guest stopped pc=0x${r.pc.toString(16)} fault=${JSON.stringify(emu.faultInfo())}`);
            break;
        }
        totalSteps++;
        if (process.env.SOAK_STATS && totalSteps % 2500 === 0) {
            const rssMB = (process.memoryUsage().rss / 1048576).toFixed(0);
            const line = `[SOAK] t=${((Date.now() - startTime) / 1000).toFixed(0)}s inst=${instCount} rxQ=${gwRxQueue.length} txQ=${gwTxPending.length} rounds=${gwRoundsSeen} rss=${rssMB}MB\n`;
            process.stdout.write(line);
        }

        if (instCount >= maxInst) break;
        await new Promise(r2 => setImmediate(r2));
    }

    if (gwWs) try { gwWs.close(); } catch (_) {}
    if (gwProcess) try { gwProcess.kill(); } catch (_) {}

    const elapsed = ((Date.now() - startTime) / 1000).toFixed(2);
    const regs = emu.getRegisters();
    if (process.env.DBG_RXDESC) {
        // eth_http layout (nm-verified): ETH_IRQ_FLAG 0x20000620,
        // RX_FRAME_IDX 0x20000628, RX_FRAME_LEN 0x2000062c, RX_DESC 0x20000630.
        const r = (a) => (emu.read32(a) >>> 0).toString(16).padStart(8, '0');
        console.log(`[RXDESC] flag=${r(0x20000620)} idx=${r(0x20000628)} len=${r(0x2000062c)}`);
        console.log(`[RXDESC] srcport=${r(0x20000000)} tgtport=${r(0x20000650)}`);
        for (let i = 0; i < 4; i++) {
            console.log(`[RXDESC] d${i} rdes0=${r(0x20000630 + i * 8)} rdes1=${r(0x20000630 + i * 8 + 4)}`);
        }
        // First 40 bytes of the d1 buffer (TCP ports at +34 if IP ihl=5).
        try {
            const fb = emu.uc.mem_read(BigInt(0x20000c60), 40);
            console.log(`[RXDESC] d1buf=${[...fb].map((b) => b.toString(16).padStart(2, '0')).join('')}`);
        } catch (e) { console.log(`[RXDESC] d1buf READERR ${e.message}`); }
    }

    const uartOut = emu.drainUart() || '';
    if (!uartOut.trim()) {
        process.stdout.write('\n');
    } else {
        console.log(`\n=== UART Output ===\n${uartOut}`);
    }

    try { if (process.stdin.isTTY) process.stdin.setRawMode(false); } catch (_) {}

    console.log(`\nDone: ${totalSteps} steps, ${instCount} instructions in ${elapsed}s`);
    console.log(`PC=0x${regs.PC.toString(16)} SP=0x${regs.SP.toString(16)}`);

    if (showRegs) {
        for (let i = 0; i <= 12; i++) {
            const reg = regs[`R${i}`] >>> 0;
            process.stdout.write(`R${i}=0x${reg.toString(16).padStart(8, '0')} `);
            if (i % 4 === 3) console.log();
        }
        console.log(`LR=0x${(regs.LR >>> 0).toString(16).padStart(8, '0')}`);
        console.log(`xPSR=0x${(regs.XPSR >>> 0).toString(16).padStart(8, '0')}`);
    }

    emu.close();
}

main().catch(e => {
    console.error('Fatal:', typeof e, String(e));
    process.exit(1);
});
