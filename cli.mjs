#!/usr/bin/env node
// stm32f4-emu — headless CLI to load and run an STM32F4 firmware image.
//
//   stm32f4-emu <firmware> [--inst N] [--format auto|bin|hex|elf] [--verbose]
//
// Loads a .bin/.elf/.hex firmware, boots it in the emulator, and streams the
// guest UART to stdout. Peripheral register accesses are traced to stderr when
// --verbose is given.
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { dirname, resolve } from 'node:path';
import * as bindings from './site/vendor/stm32_periph_wasm.js';
import { createEmulator } from './site/emulator.js';
import { parseIntelHex, parseElf } from './site/loaders.js';

const require = createRequire(import.meta.url);
const unicornFactory = require('./site/vendor/unicorn_arm.cjs');
const __dirname = dirname(fileURLToPath(import.meta.url));
const svdXml = readFileSync(resolve(__dirname, 'site/vendor/stm32f407.svd'), 'utf8');
const wasmBytes = new Uint8Array(readFileSync(resolve(__dirname, 'site/vendor/stm32_periph_wasm_bg.wasm')));
const pkg = JSON.parse(readFileSync(resolve(__dirname, 'package.json'), 'utf8'));

const USAGE = `stm32f4-emu — run an STM32F4 firmware image in the emulator

Usage:
  stm32f4-emu <firmware> [options]

Arguments:
  <firmware>            path to a firmware image (.bin, .elf, or .hex)

Options:
  -n, --inst <N>        instruction budget to run (default 20000000)
  -f, --format <fmt>    firmware format: auto|bin|hex|elf (default auto)
  -v, --verbose         trace peripheral register reads/writes to stderr
  -h, --help            show this help
  -V, --version         show version

Examples:
  stm32f4-emu build/firmware.bin
  stm32f4-emu app.elf --inst 5000000 --verbose
`;

function fail(msg) {
    process.stderr.write(msg + '\n\n' + USAGE);
    process.exit(2);
}

function detectFormat(buf, format) {
    if (format !== 'auto') return format;
    if (buf[0] === 0x7F && buf[1] === 0x45 && buf[2] === 0x4C && buf[3] === 0x46) return 'elf';
    let i = 0;
    while (i < buf.length && (buf[i] === 0x20 || buf[i] === 0x09 || buf[i] === 0x0D || buf[i] === 0x0A)) i++;
    if (buf[i] === 0x3A) return 'hex'; // ':'
    return 'bin';
}

async function main() {
    const args = process.argv.slice(2);
    let firmwarePath = null;
    let inst = 20000000;
    let format = 'auto';
    let verbose = false;
    for (let i = 0; i < args.length; i++) {
        const a = args[i];
        if (a === '-h' || a === '--help') { process.stdout.write(USAGE); process.exit(0); }
        if (a === '-V' || a === '--version') { process.stdout.write(`stm32f4-emu ${pkg.version}\n`); process.exit(0); }
        if (a === '-v' || a === '--verbose') { verbose = true; continue; }
        if (a === '-n' || a === '--inst') {
            const v = args[++i];
            inst = Number(v);
            if (!Number.isFinite(inst) || inst <= 0) fail(`invalid --inst value: ${v}`);
            continue;
        }
        if (a === '-f' || a === '--format') {
            format = args[++i];
            if (!['auto', 'bin', 'hex', 'elf'].includes(format)) fail(`invalid --format: ${format}`);
            continue;
        }
        if (a.startsWith('-')) fail(`unknown option: ${a}`);
        if (firmwarePath) fail('multiple firmware paths given');
        firmwarePath = a;
    }
    if (!firmwarePath) fail('no firmware path given');

    let raw;
    try {
        raw = new Uint8Array(readFileSync(firmwarePath));
    } catch (e) {
        fail(`cannot read firmware '${firmwarePath}': ${e.message}`);
    }

    const fmt = detectFormat(raw, format);
    let firmware, extra_mem = [];
    try {
        if (fmt === 'elf') {
            const elf = parseElf(raw);
            if (!elf.flash) fail('ELF contains no FLASH segment (expected a segment at 0x08000000)');
            firmware = elf.flash;
            extra_mem = elf.extraMem || [];
        } else if (fmt === 'hex') {
            const hex = parseIntelHex(Buffer.from(raw).toString('latin1'));
            if (!hex.flash) fail('Intel HEX contains no FLASH records (expected data in the 0x08000000 range)');
            firmware = hex.flash;
        } else {
            firmware = raw;
        }
    } catch (e) {
        fail(`failed to parse ${fmt} firmware: ${e.message}`);
    }

    let emu;
    try {
        emu = await createEmulator({
            firmware, bindings, unicorn: unicornFactory, svdXml, wasmInit: wasmBytes,
            extra_mem, verbose,
        });
    } catch (e) {
        fail(`emulator failed to load firmware: ${e.message}`);
    }

    const STEP = 100000;
    let remaining = inst;
    let lastInst = 0;
    try {
        while (remaining > 0) {
            const take = Math.min(STEP, remaining);
            const r = emu.step(take);
            remaining -= (r.instCount - lastInst);
            lastInst = r.instCount;
            const u = emu.drainUart();
            if (u && u.length) process.stdout.write(u.toString());
            if (r.stopped) break;
        }
        const tail = emu.drainUart();
        if (tail && tail.length) process.stdout.write(tail.toString());
    } finally {
        try { emu.close(); } catch {}
    }
}

main().catch((e) => {
    process.stderr.write(`stm32f4-emu: ${e.message}\n`);
    process.exit(1);
});
