# MCP server

`mcp/server.mjs` exposes the emulator as [Model Context
Protocol](https://modelcontextprotocol.io) tools, so an MCP host (Claude
Code, Claude Desktop, or any MCP client) can boot firmware, step
execution, read UART, poke pins, inject analog values, and inspect
registers interactively — firmware debugging driven by an agent instead
of a hand-written script.

Built on the official `@modelcontextprotocol/sdk` over a stdio transport.

## Install

The SDK and `zod` are **optional peer dependencies** — using this package
as an emulator library stays completely dependency-free (a plain
`npm i stm32f4-emulator` installs exactly one package), and only the MCP
server needs them:

```bash
npm install stm32f4-emulator @modelcontextprotocol/sdk zod
```

Running the server without them exits with a message telling you exactly
what to install, rather than a module-resolution stack trace.

From a clone of this repo, `npm install` picks them up as devDependencies:

```bash
npm install
npm run mcp            # or: node mcp/server.mjs
npm run test:mcp       # protocol round-trip smoke test
```

## Client configuration

Claude Desktop / Claude Code `mcpServers` entry:

```json
{
  "mcpServers": {
    "stm32f4": {
      "command": "node",
      "args": ["/absolute/path/to/stm32 F4/mcp/server.mjs"]
    }
  }
}
```

Installed as a package, the `stm32f4-mcp` bin is on `PATH`:

```json
{
  "mcpServers": {
    "stm32f4": { "command": "stm32f4-mcp" }
  }
}
```

## Tools

| Tool | What it does |
|---|---|
| `list_firmwares` | List the bundled firmware keys `load_firmware` accepts |
| `load_firmware` | Boot a firmware (`firmware`, optional `enable_irqs`), replacing any active session |
| `step` | Run up to N instructions; returns PC, instruction count, stopped flag |
| `read_uart` | Drain UART output printed since the last call |
| `send_uart` | Send text to the firmware over UART RX |
| `read_pin` | Read a GPIO pin's output and input levels |
| `write_pin` | Drive a GPIO input level into the guest |
| `set_adc_channel` | Force an ADC channel to a 12-bit value (omit `value` to clear) |
| `read_registers` | R0-R12, SP, LR, PC, XPSR as hex |
| `read_memory` | Read a 32-bit word at any address (flash, SRAM, MMIO) |
| `attach_component` | Attach a `led`/`button`/`pwm`/`potentiometer`; returns an id |
| `read_component` | Read an attached component's state |
| `control_component` | `press`/`release` a button, or `set` a potentiometer value |
| `reset` | Close the session and drop attached components |

Components are the same classes documented in
[components.md](components.md) — the MCP layer is a thin wrapper over the
public library API, not a parallel implementation.

## One session at a time

The server keeps a single active emulator. `load_firmware` closes any
previous session first, and `reset` clears it.

Switching firmware within one server process is allowed but **not fully
clean**: the wasm peripheral model holds process-lifetime global state
that a second `init()` does not reset (see the "one firmware per process"
gotcha in [components.md](components.md) and AGENTS.md §9). `load_firmware`
returns a note when it replaces a session. For a guaranteed-clean boot,
restart the server process.

## Example session

```
list_firmwares                             -> ["eth_http", "blinky", ...]
load_firmware  {firmware: "blinky"}        -> loaded
attach_component {type: "led", port: "A", pin: 5}  -> {id: "led1"}
step {instructions: 100000}                -> {pc: "0x80001ee", ...}
read_uart                                  -> "=== Blinky ===\r\nLED: GPIOA PA5..."
read_component {id: "led1"}                -> {on: true}
```
