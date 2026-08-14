#!/usr/bin/env node
// MCP server exposing the STM32F407 emulator as tools an MCP client
// (Claude Code / Claude Desktop / any MCP host) can drive: load firmware,
// step execution, read/write UART, poke and watch GPIO pins, inject ADC
// values, and inspect CPU registers.
//
// One active emulator session at a time — load_firmware/reset replaces it.
// This mirrors the hard "one firmware per process" constraint documented in
// docs/components.md: createEmulator() instances are not safe to reuse
// across different firmware in the same process, so we close the old
// instance before creating a new one and surface a warning that a fresh
// server process is the only fully clean way to switch firmware.
import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';
import { createSTM32F407, FIRMWARES, LED, Button, Pwm, Potentiometer } from '../index.mjs';

let session = null;      // { emu, firmware, components: Map }
let componentSeq = 0;

const text = (s) => ({ content: [{ type: 'text', text: typeof s === 'string' ? s : JSON.stringify(s, null, 2) }] });
const err = (s) => ({ content: [{ type: 'text', text: s }], isError: true });

function requireSession() {
    if (!session) throw new Error('no firmware loaded — call load_firmware first');
    return session;
}

const server = new McpServer({ name: 'stm32f4-emulator', version: '0.1.0' });

server.registerTool('list_firmwares', {
    description: 'List the firmware images bundled with the emulator that load_firmware accepts.',
    inputSchema: {},
}, async () => text(Object.keys(FIRMWARES)));

server.registerTool('load_firmware', {
    description: 'Boot a bundled firmware image, replacing any active session. Use list_firmwares for valid names.',
    inputSchema: {
        firmware: z.string().describe('a firmware key from list_firmwares, e.g. "blinky"'),
        enable_irqs: z.boolean().optional().describe('run guest IRQ handlers between batches (needed for interrupt-driven firmware; must stay off for the ETH demos)'),
    },
}, async ({ firmware, enable_irqs = false }) => {
    if (!FIRMWARES[firmware]) return err(`unknown firmware '${firmware}' (have: ${Object.keys(FIRMWARES).join(', ')})`);
    const replaced = session !== null;
    if (session) { try { session.emu.close(); } catch {} }
    const emu = await createSTM32F407({ firmware, enable_irqs });
    session = { emu, firmware, components: new Map() };
    componentSeq = 0;
    return text({
        loaded: firmware,
        enable_irqs,
        note: replaced
            ? 'Replaced a previous session. Peripheral state is process-global in the wasm model, so switching firmware in one process can misbehave (see docs/components.md) — restart this server for a fully clean boot.'
            : undefined,
    });
});

server.registerTool('step', {
    description: 'Run the CPU for up to N instructions. Returns the program counter, total instruction count, and whether the machine stopped.',
    inputSchema: { instructions: z.number().int().positive().default(100000) },
}, async ({ instructions }) => {
    const { emu } = requireSession();
    const r = emu.step(instructions);
    return text({ pc: '0x' + (r.pc >>> 0).toString(16), instCount: r.instCount, stopped: r.stopped });
});

server.registerTool('read_uart', {
    description: 'Drain and return any UART output the firmware has printed since the last call.',
    inputSchema: {},
}, async () => text(requireSession().emu.drainUart() || '(no output)'));

server.registerTool('send_uart', {
    description: 'Send text to the firmware over UART RX (as if typed into a serial console).',
    inputSchema: { text: z.string() },
}, async ({ text: s }) => {
    const { emu } = requireSession();
    emu.sendUart([...s].map((c) => c.charCodeAt(0)));
    return text(`sent ${s.length} byte(s)`);
});

server.registerTool('read_pin', {
    description: 'Read a GPIO pin: the level the guest is driving out, and the input level it sees.',
    inputSchema: {
        port: z.string().describe('port letter, e.g. "A"'),
        pin: z.number().int().min(0).max(15),
    },
}, async ({ port, pin }) => {
    const p = requireSession().emu.pin(port, pin);
    return text({ port, pin, output: p.read(), input: p.readInput() });
});

server.registerTool('write_pin', {
    description: 'Drive a GPIO input level into the guest (as an external device or button would).',
    inputSchema: {
        port: z.string().describe('port letter, e.g. "A"'),
        pin: z.number().int().min(0).max(15),
        level: z.boolean(),
    },
}, async ({ port, pin, level }) => {
    requireSession().emu.pin(port, pin).write(level);
    return text(`P${port.toUpperCase()}${pin} <- ${level ? 'high' : 'low'}`);
});

server.registerTool('set_adc_channel', {
    description: 'Force an ADC channel to a 12-bit value (0-4095), simulating an analog sensor. Omit `value` to clear the override.',
    inputSchema: {
        peripheral: z.string().default('ADC1'),
        channel: z.number().int().min(0).max(18),
        value: z.number().int().min(0).max(4095).optional(),
    },
}, async ({ peripheral, channel, value }) => {
    const { emu } = requireSession();
    if (value === undefined) {
        emu.clearAdcChannel(peripheral, channel);
        return text(`${peripheral} ch${channel} override cleared`);
    }
    emu.setAdcChannel(peripheral, channel, value);
    return text(`${peripheral} ch${channel} <- ${value}`);
});

server.registerTool('read_registers', {
    description: 'Read the ARM CPU registers (R0-R12, SP, LR, PC, XPSR) as hex.',
    inputSchema: {},
}, async () => {
    const r = requireSession().emu.getRegisters();
    return text(Object.fromEntries(Object.entries(r).map(([k, v]) => [k, '0x' + (v >>> 0).toString(16)])));
});

server.registerTool('read_memory', {
    description: 'Read a 32-bit word from guest memory (flash, SRAM, or a peripheral register).',
    inputSchema: { address: z.number().int().describe('absolute address, e.g. 0x40020014 for GPIOA_ODR') },
}, async ({ address }) => {
    const v = requireSession().emu.read32(address);
    return text({ address: '0x' + (address >>> 0).toString(16), value: '0x' + (v >>> 0).toString(16), decimal: v });
});

server.registerTool('attach_component', {
    description: 'Attach a virtual component to the running machine and return its id for read_component. Types: led (port+pin), button (port+pin), pwm (timer+channel), potentiometer (peripheral+channel).',
    inputSchema: {
        type: z.enum(['led', 'button', 'pwm', 'potentiometer']),
        port: z.string().optional().describe('led/button: port letter, e.g. "A"'),
        pin: z.number().int().min(0).max(15).optional().describe('led/button: pin number'),
        timer: z.string().optional().describe('pwm: timer name, e.g. "TIM2"'),
        channel: z.number().int().optional().describe('pwm: channel 1-4; potentiometer: ADC channel'),
        peripheral: z.string().optional().describe('potentiometer: ADC peripheral, default ADC1'),
        activeLow: z.boolean().optional(),
    },
}, async (a) => {
    const { emu, components } = requireSession();
    const id = `${a.type}${++componentSeq}`;
    let comp;
    switch (a.type) {
        case 'led':
            if (a.port === undefined || a.pin === undefined) return err('led requires port and pin');
            comp = new LED(emu, a.port, a.pin, { activeLow: a.activeLow ?? false });
            break;
        case 'button':
            if (a.port === undefined || a.pin === undefined) return err('button requires port and pin');
            comp = new Button(emu, a.port, a.pin, { activeLow: a.activeLow ?? true });
            break;
        case 'pwm':
            if (!a.timer) return err('pwm requires timer');
            comp = new Pwm(emu, a.timer, a.channel ?? 1);
            break;
        case 'potentiometer':
            if (a.channel === undefined) return err('potentiometer requires channel');
            comp = new Potentiometer(emu, a.peripheral ?? 'ADC1', a.channel);
            break;
    }
    components.set(id, { type: a.type, comp });
    return text({ id, type: a.type });
});

server.registerTool('read_component', {
    description: 'Read the current state of an attached component (led: on/off, pwm: freq/duty, potentiometer: value).',
    inputSchema: { id: z.string() },
}, async ({ id }) => {
    const entry = requireSession().components.get(id);
    if (!entry) return err(`no component '${id}'`);
    const { type, comp } = entry;
    if (type === 'led') return text({ id, type, on: comp.value });
    if (type === 'pwm') return text({ id, type, freq: comp.freq, duty: comp.duty });
    if (type === 'potentiometer') return text({ id, type, value: comp.value });
    return text({ id, type, note: 'buttons are write-only; use control_component to press/release' });
});

server.registerTool('control_component', {
    description: 'Act on an attached component: press/release a button, or set a potentiometer value.',
    inputSchema: {
        id: z.string(),
        action: z.enum(['press', 'release', 'set']),
        value: z.number().optional().describe('required for action=set on a potentiometer'),
    },
}, async ({ id, action, value }) => {
    const entry = requireSession().components.get(id);
    if (!entry) return err(`no component '${id}'`);
    const { type, comp } = entry;
    if (action === 'set') {
        if (type !== 'potentiometer') return err(`'set' only applies to a potentiometer, not ${type}`);
        if (value === undefined) return err("action 'set' requires value");
        comp.value = value;
        return text({ id, value: comp.value });
    }
    if (type !== 'button') return err(`'${action}' only applies to a button, not ${type}`);
    action === 'press' ? comp.press() : comp.release();
    return text({ id, action });
});

server.registerTool('reset', {
    description: 'Close the active emulator session and clear all attached components.',
    inputSchema: {},
}, async () => {
    if (!session) return text('no active session');
    try { session.emu.close(); } catch {}
    const was = session.firmware;
    session = null;
    return text(`closed session (${was})`);
});

await server.connect(new StdioServerTransport());
