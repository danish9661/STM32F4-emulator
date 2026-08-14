// Smoke test for the MCP server: spawns mcp/server.mjs as a subprocess and
// drives it through the real MCP stdio protocol using the SDK's own client
// (not hand-crafted JSON-RPC), so the handshake, schema validation, and
// tool dispatch are all exercised end to end.
// Usage: node mcp/test_mcp.mjs  (exit 0 = PASS)
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';
import { fileURLToPath } from 'node:url';

// fileURLToPath, not URL.pathname: pathname percent-encodes spaces (and
// mangles drive letters on Windows), which breaks the spawned path.
const serverPath = fileURLToPath(new URL('./server.mjs', import.meta.url));
const transport = new StdioClientTransport({ command: process.execPath, args: [serverPath] });
const client = new Client({ name: 'stm32f4-mcp-smoke', version: '0.1.0' });
await client.connect(transport);

const textOf = (r) => r.content.map((c) => c.text).join('\n');
const checks = [];
const check = (name, ok, detail = '') => {
    checks.push({ name, ok });
    console.log(`${ok ? 'ok  ' : 'FAIL'} ${name}${detail ? ' — ' + detail : ''}`);
};

const tools = await client.listTools();
const names = tools.tools.map((t) => t.name).sort();
check('lists tools', names.length >= 10, names.join(','));

const fw = textOf(await client.callTool({ name: 'list_firmwares', arguments: {} }));
check('list_firmwares includes blinky', fw.includes('blinky'));

await client.callTool({ name: 'load_firmware', arguments: { firmware: 'blinky' } });

const led = JSON.parse(textOf(await client.callTool({
    name: 'attach_component', arguments: { type: 'led', port: 'A', pin: 5 },
})));
check('attach_component returns an id', typeof led.id === 'string', led.id);

let uart = '';
for (let i = 0; i < 40; i++) {
    await client.callTool({ name: 'step', arguments: { instructions: 100000 } });
    const out = textOf(await client.callTool({ name: 'read_uart', arguments: {} }));
    if (out && out !== '(no output)') uart += out;
    if (uart.includes('tick 1')) break;
}
check('firmware boots and prints over UART', uart.includes('=== Blinky ==='), JSON.stringify(uart.slice(0, 40)));
check('firmware reaches tick output', uart.includes('tick '));

const ledState = JSON.parse(textOf(await client.callTool({ name: 'read_component', arguments: { id: led.id } })));
check('read_component reports LED state', typeof ledState.on === 'boolean', `on=${ledState.on}`);

const regs = JSON.parse(textOf(await client.callTool({ name: 'read_registers', arguments: {} })));
check('read_registers returns a PC', typeof regs.PC === 'string' && regs.PC.startsWith('0x'), regs.PC);

const odr = JSON.parse(textOf(await client.callTool({ name: 'read_memory', arguments: { address: 0x40020014 } })));
check('read_memory reads GPIOA_ODR', typeof odr.value === 'string', odr.value);

await client.callTool({ name: 'set_adc_channel', arguments: { peripheral: 'ADC1', channel: 3, value: 1234 } });
check('set_adc_channel accepted', true);

const pinInfo = JSON.parse(textOf(await client.callTool({ name: 'read_pin', arguments: { port: 'A', pin: 5 } })));
check('read_pin reports output level', typeof pinInfo.output === 'boolean', `output=${pinInfo.output}`);

const bad = await client.callTool({ name: 'read_component', arguments: { id: 'nope1' } });
check('unknown component is an error', bad.isError === true);

await client.callTool({ name: 'reset', arguments: {} });
const afterReset = await client.callTool({ name: 'step', arguments: { instructions: 1000 } });
check('step after reset errors', afterReset.isError === true);

await client.close();

const failed = checks.filter((c) => !c.ok);
console.log(failed.length === 0 ? 'PASS' : `FAIL (${failed.length}/${checks.length})`);
process.exit(failed.length === 0 ? 0 : 1);
