# AGENTS.md

This file is the working knowledge base for this repository. It documents the
architecture, build steps, how to run the various tests, the current state of
the webserver/network emulation effort, and exactly what is left to do.

It is meant to be read by both humans and AI coding agents so that a new session
can pick up where the last one left off without re-deriving everything.

---

## 1. Project overview

This repo emulates an **STM32F407** microcontroller. The core ideas:

- A **Unicorn** CPU emulator (`unicorn_arm.cjs`, Unicorn 2.1.4 compiled to WASM)
  executes the ARM Cortex-M4 (Thumb-2) firmware instructions.
- A **peripheral model** written in Rust, compiled to WASM with
  `wasm-bindgen` (`stm32_periph_wasm_bg.wasm`), simulates the on-chip
  peripherals (RCC, USART, GPIO, DMA, ETH, TIM, NVIC, etc.).
- The two are connected by **memory hooks**: every read/write to a hooked
  MMIO range is routed through `periph_read` / `periph_write` (the WASM model).
  Reads are "answered" by writing the modeled register value back into guest
  memory with `uc.mem_write` so the guest sees it.
- A **gateway / test driver** (JS) drives `uc.emu_start`, drains UART output,
  injects network packets, and services the ETH TX/RX protocol.

Native (non-WASM) build: a full SDL emulator (`stm32-emulator.exe`) also exists
in `src/`. The WASM peripheral model is the one used for headless testing.

---

## 2. Repository layout

```
stm32-emulator-main/
├── src/                    Native Rust emulator (SDL app, not the headless path)
├── stm32-periph-wasm/      The WASM peripheral model crate
│   ├── src/                Rust sources
│   │   ├── lib.rs          wasm_bindgen exports (periph_read/write, tick, eth_*, ...)
│   │   ├── system.rs       System state, UART buffer, ETH_TX_POLL/RX_POLL/DONE atomics
│   │   ├── peripherals/    mod.rs, eth.rs, usart.rs, rcc.rs, dma.rs, nvic.rs, ...
│   └── pkg/                Compiled JS bindings + all test drivers
│       ├── stm32_periph_wasm.js / _bg.wasm / .d.ts   built bindings
│       ├── unicorn_arm.cjs                            Unicorn 2.1.4 WASM build
│       ├── cli.mjs                                    Reference gateway (batch exec)
│       └── test_*.mjs, trace_*.mjs, cmp_speed*.mjs    Test drivers (see §5)
├── webserver/              The target firmware (Arduino sketch)
│   ├── webserver.ino       Source (DHCP + TCP + HTTP server)
│   └── build/              webserver.bin / .elf / .map
├── monox/stm32f407.svd     SVD XML used to build the register map
└── know.md                 Older notes (peripheral audit table, build snippets)
```

---

## 3. Build

### 3.1 Rebuild the WASM peripheral model

```powershell
cd stm32-periph-wasm
wasm-pack build --release        # wasm-pack 0.14.0 is installed
```

This regenerates `pkg/stm32_periph_wasm.js`, `pkg/stm32_periph_wasm_bg.wasm`,
etc. (wasm-bindgen must be compatible with the Rust/wasm-bindgen versions in
`Cargo.toml`.)

### 3.2 Build the firmware (Arduino)

```powershell
arduino-cli compile --fqbn "STMicroelectronics:stm32:GenF4:pnum=GENERIC_F407VGTX" `
  "C:\...\webserver" --output-dir "C:\...\webserver\build"
```

The F4 toolchain is at
`C:\Users\Danish\AppData\Local\Arduino15\packages\STMicroelectronics\tools\xpack-arm-none-eabi-gcc\14.2.1-1.1\arm-none-eabi\bin\` and includes
`objdump.exe` / `nm.exe` for disassembly and symbol lookup.

### 3.3 Native emulator

```powershell
cd stm32-emulator-main
cargo build --release
```

---

## 4. Core architecture (headless test path)

A minimal test (`test_webserver_net.mjs`) does the following:

1. Load `unicorn_arm.cjs` and `stm32_periph_wasm.js`, `init_svd()` with the SVD XML.
2. `uc.mem_map` FLASH (0x08000000), SRAM (0x20000000), full peripheral space
   (0x40000000, 0x70000000), and system space (0xE0000000, 0x1000000).
3. Write the firmware binary at 0x08000000; read SP/PC from the vector table
   (0x08000000 / 0x08000004) and set `SP` and `PC|1` (Thumb bit).
4. Register `HOOK_MEM_READ` / `HOOK_MEM_WRITE` over narrow MMIO ranges (see
   the `ranges` array in the test). Each read calls `periph_read(addr,size)`
   and writes the value back into memory so the guest sees it. Each write calls
   `periph_write(addr,size,value)`.
5. Register `HOOK_BLOCK` that calls `periph.tick()` (advances timers/state).
6. Loop `uc.emu_start(curPc|1, 0, 0, STEP)` (stepped execution), draining
   `get_uart_output()` and injecting network packets when the UART log reaches
   expected milestones.

### Hook ranges used

```js
[0x40000000,0x40007fff], [0x40011000,0x400117ff], [0x40020000,0x400223ff],
[0x40023800,0x40023bff], [0x40023c00,0x40024fff], [0x40026400,0x40026fff],
[0x40028000,0x400293ff], [0xe000e010,0xe000e020], [0xe000e100,0xe000e500],
[0xe000ed00,0xe000ef00]
```

- 0x40011000  = USART1 (SR at +0, DR at +4; SR always returns 0xC0 = TXE|TC).
- 0x40028000  = Ethernet MAC (MACMIIAR +0x14, MACMIIDR +0x18).
- 0x40029000  = Ethernet DMA (DMABMR +0, DMATPDR +0x04, DMARPDR +0x08,
  DMARDLAR +0x0C, DMATDLAR +0x10, DMASR +0x14, DMAOMR +0x18, DMAIER +0x1C).
- 0xE000E100+ = NVIC ISER/ICER/etc., 0xE000ED00 = SCB.

### The ETH poll/done protocol (Rust model <-> JS driver)

The Rust `eth.rs` + `system.rs` use three atomics:

- `ETH_TX_POLL` + `ETH_TX_DESC_ADDR` — set by the model when the guest writes
  DMATPDR (firmware wants to transmit).
- `ETH_RX_POLL` + `ETH_RX_DESC_ADDR` — set when the guest writes DMARPDR.
- `ETH_DONE` — set via `eth_tx_done()` / `eth_rx_done()`; the model's read of
  DMASR returns TS/RS bits reflecting done status.

JS-visible exports (see `stm32_periph_wasm.d.ts`):

- `eth_is_tx_poll()`, `eth_get_tx_desc_addr()`, `eth_clear_tx_poll()`
- `eth_is_rx_poll()`, `eth_get_rx_desc_addr()`, `eth_clear_rx_poll()`
- `eth_tx_done()`, `eth_rx_done()`, `eth_signal_rx_poll(desc_addr)`

The reference gateway `cli.mjs` uses batch execution (`maxBatch`, default
100000) with `HOOK_BLOCK` calling `tick()`, and `uc.emu_stop()` when DMA is
pending or an interrupt is pending; it then services TX/RX and resumes.

---

## 5. Test drivers in `pkg/`

| File | Purpose |
|---|---|
| `test_webserver_net.mjs` | **Main target.** Boots webserver firmware, injects DHCP Offer/Ack + TCP SYN/ACK + HTTP GET, expects to capture the 3 HTTP response TX packets. Currently stalls (see §7). |
| `test_webserver.mjs`, `test_webserver2.mjs` | Earlier webserver boot tests. |
| `test_hang.mjs` | Pre-populated (no-hook) instruction-count sweep: `emu_start(..., n)` for n = 1000..200000. Confirms where raw execution stops returning. |
| `bench_batch.mjs` | Batch-execution throughput + stall check (BATCH=20000). |
| `cli.mjs` | Reference gateway (config/YAML based, batch exec, ETH servicing, watchdog, interrupts). |
| `trace_*.mjs`, `trace2.mjs`, `trace_precise.mjs`, `trace_addrs*.mjs` | PC / address tracing helpers. |
| `cmp_speed*.mjs` | Throughput comparisons of execution strategies. |
| `minimal_test.mjs`, `test_esm.mjs`, `test_require.cjs`, `test_unicorn.cjs`, `test_svd_run.cjs` | Sanity checks for the bindings. |

### How to run the main test

```powershell
cd stm32-periph-wasm\pkg
node test_webserver_net.mjs
```

Notes:
- Redirect output to a file when debugging (`*> out.txt`) and inspect it — the
  pipe/console path can hide buffered output on kill.
- `writeSync(1, ...)` from `node:fs` flushes immediately and is the reliable way
  to observe progress when a run hangs and gets killed.
- The in-script timeout is 120s; the shell timeout in this environment defaults
  to 120s too, so pass a larger `-TimeoutSeconds` when expecting long runs.

---

## 6. Webserver firmware facts

### Expected full boot UART log

```
\r\n=== Web Server ===\r\n
ETH clock ON\r\n
DMA reset\r\n
MAC RE+TE\r\n
MAC addr set\r\n
link up\r\n
DMA ST+SR\r\n
RX descriptors ready\r\n
Ready\r\n
DHCP Discover\r\n
Wait DHCP Offer\r\n
Offer IP=<a.b.c.d>\r\n
DHCP Ack IP=<a.b.c.d> OK\r\n
(Arp...?) 
TCP fl=.. port=..\r\n SYN-ACK\r\n Client ACK\r\n
REQ:...\r\n  -> then 3 HTTP response TX packets
Done\r\n
```

### Firmware global addresses (from `nm` on webserver.elf)

```
SRV_SEQ          0x20000000
CLIENT_CONNECTED 0x20000008
CLIENT_PORT      0x2000000c
CLIENT_MAC       0x20000010
CLIENT_IP        0x20000018
MY_IP            0x20000020
RX_FRAME_IDX     0x20000024
RX_FRAME_LEN     0x20000028
ETH_IRQ_FLAG     0x2000002c
TX_DESC          0x20000030   (tx_desc[0][0]=flags|len, [1]=data ptr)
RX_DESC          0x20000040   (rx_desc[i][0], rx_desc[i][1]=rx_buf[i])
TX_PKT           0x20000060   (tx_pkt buffer)
RX_BUF           0x20000660   (rx_buf[0])
RX_BUF_SIZE      1536
```

Reset vector: SP=0x20020000, reset PC=0x08000185 (Thumb). The reset handler
zeroes .bss (0x20000008..0x20001e60), copies .data, then calls `main`.

### How the firmware drives the NIC

- `eth_send_packet(data,len)` (webserver.ino:84):
  `tx_desc[0][0]=0x80000000|(1<<28)|(1<<27)|(len&0x3FFF); tx_desc[0][1]=data;`
  then `DMATDLAR=tx_desc; DMATPDR=1;` then polls
  `eth_irq_flag & 1` up to 5,000,000 iterations (bit 0 = TX done), clears it and
  returns 1 on success, 0 on timeout.
- `eth_recv_packet(buf,len)` (webserver.ino:92): polls `eth_irq_flag & 2`
  (bit 1 = RX event); on hit, returns `rx_buf[rx_frame_idx]` /
  `rx_frame_len`, re-arms `rx_desc[i][0]=0x80000000|ETH_MAX_PKT`, writes
  DMARPDR=1.
- `ETH_IRQHandler` (webserver.ino:103): reads DMASR; bit 0 (TS) sets
  `eth_irq_flag|=1`, bit 6 (RS) sets `eth_irq_flag|=2`, and scans rx_desc to
  find a CPU-owned frame for rx_frame_idx/rx_frame_len.
- Firmware polls `eth_irq_flag` in **SRAM**, not DMASR, for TX/RX completion.
  So the JS driver can drive progress by writing `ETH_IRQ_FLAG` in guest SRAM
  directly (see §8), which is the approach in `test_webserver_net.mjs`.

### Firmware boot / link-up

- `eth_init()` (webserver.ino:70): RCC AHB1ENR ETHMAC bit 25 -> "ETH clock ON",
  DMABMR|=1 + delay_ms(2) -> "DMA reset", MACCR RE|TE, MACA0HR/LR -> "MAC addr
  set", then up to 100 iterations of `eth_phy_read(0,1)&0x4` (link) with
  `delay_ms(10)` between tries -> "link up", DMAOMR ST|SR -> "DMA ST+SR".
- `eth_phy_read` (webserver.ino:64): writes MACMIIAR (MBUSY set) and polls
  MACMIIAR bit 0 until the model clears it, then returns MACMIIDR. The Rust
  model must clear MBUSY on write (fixed in `eth.rs`, ~line 197-209).
- `delay_ms(n)` (webserver.ino:62) is an inlined nop loop: 4000 nops per ms.

---

## 7. CURRENT STATE: the hang (what we know)

### Symptom

`uc.emu_start(...)` fails to return. It is reproducible and tied to a fixed
point in the firmware's early execution, NOT to a specific step count, NOT to
the hook mem-write pattern.

### Evidence collected this session

1. **Tiny steps (STEP=3) progress through boot** then stall:
   UART reaches `"...DMA reset\r\nMAC RE+TE\r\nMAC addr"` (≈64 chars), PC stops
   at 0x80001d4 (USART_DR store in `uart_putchar`). After that, no progress
   markers for 45s.
2. **STEP=100** (the original test): no output at all in 60s — first
   `emu_start` doesn't return. (In an earlier session the same file *did* reach
   `"DHCP Dis"` then stall — i.e. non-deterministic between runs.)
3. **Batch exec (BATCH=20000)**: batch 0 returns (20ms), batch 1 hangs.
4. **No-hook, pre-populated register memory** (`test_hang.mjs` / inline sweep):
   `emu_start(..., n)` returns cleanly for n = 1000..30000 (PC lands at
   0x8000b28-0x8000b2c = the delay_ms loop), then **hangs for n=40000+**.
5. Using Unicorn's built-in timeout (microseconds arg) aborts with
   `Aborted(). Build with -sASSERTIONS for more info` and
   `qemu: qemu_thread_create: Not supported` — the timeout path in this WASM
   build is broken, and the machine is left unusable afterwards.

### Interpretation

- The hang reproduces **without any JS hooks**, so it is not the
  `uc.mem_write`-inside-read-hook pattern.
- It occurs at a **fixed instruction budget (~35-40k instructions)**,
  regardless of how the budget is sliced (3, 100, 20000 instructions per call).
  The firmware is in/near `setup()`'s `delay_ms(2)` (PC 0x8000b28..0x8000b2c,
  pure nops) and the following eth_init/MAC phase.
- This points to a bug/limitation in this particular **Unicorn 2.1.4 WASM
  build**: something about translating/executing that region (e.g. a large
  translation cache, a specific Thumb block, or an internal TCG state) wedges
  the WASM instance. The broken timeout path (`qemu_thread_create: Not
  supported`) is strong evidence the WASM port is incomplete.

### Options already tried

- Stepped execution (STEP 3/10/100/500/5000) — stalls at same point.
- Continuous execution with pre-populated registers (no hooks) — stalls at
  ~40k instructions.
- Batch execution (20000/step) — batch 1 stalls.
- Unicorn built-in timeout — aborts (broken in this build).
- `uc.close()` after a wedged run — throws "memory access out of bounds".

### Options not yet fully explored

- **Block-stepping**: drive execution one basic block at a time via
  `HOOK_BLOCK` + `uc.emu_stop()` (the pattern `cli.mjs` uses for DMA). Test if
  single-block steps avoid the wedge.
- **Fresh Unicorn build**: the WASM port may be the problem; try a newer/native
  (Node addon) Unicorn, or recompile with assertions to locate the abort.
- **Reduce firmware code path**: build a minimal firmware that exercises the
  same region to isolate whether a specific instruction sequence (e.g. the
  `mov.w r3,#8000` + nop loop, or a Thumb-2 wide instruction) triggers it.
- **Check WASM heap growth**: `uc.mem_read`/`mem_write` malloc/free per call;
  heap growth mid-run is a suspect for the wedge (Emscripten `_resize_heap`).

---

## 8. NEXT PHASE — what is left to do

### Goal
Get the webserver firmware to complete: DHCP Discover -> Offer -> Request ->
Ack -> TCP handshake -> HTTP GET -> 3-chunk HTTP response emitted over UART/TX,
and captured by the test.

### Execution-strategy fix (blocker, do first)
1. Try block-stepping via `HOOK_BLOCK` + `uc.emu_stop()` (cli.mjs pattern) and
   see if it runs past the 40k-instruction wedge.
2. If not, try a different Unicorn build (newer wasm, or native binding), or
   reproduce with a trimmed firmware to pinpoint the offending instruction.
3. When a reliable stepping mode is found, re-validate full boot UART output.

### Network flow implementation (after the blocker)
Once execution is reliable, finish `test_webserver_net.mjs`:

1. **TX short-circuit**: on DMATPDR write (0x40029004), call
   `periph.eth_tx_done()` + `periph.eth_clear_tx_poll()` and set guest SRAM
   `ETH_IRQ_FLAG |= 1` (bit 0 = TX done) so `eth_send_packet`'s 5M-iteration
   poll exits immediately. Capture the outgoing packet from the TX descriptor
   (`tx_desc[0][0]&0x3FFF` = len, `tx_desc[0][1]` = data pointer, read via
   `r32`/`uc.mem_read`) and log it (needed to learn the DHCP XID/server IP the
   firmware uses).
2. **RX injection** (already scaffolded in `injectPacket`): write the Ethernet
   frame into `RX_BUF + idx*1536`, set `rx_desc[idx][0] = (len&0x3fff)<<16`
   (CPU owns, length), set `RX_FRAME_IDX`/`RX_FRAME_LEN`, then
   `ETH_IRQ_FLAG |= 2`. Keep a single RX descriptor index since the firmware's
   `ETH_IRQHandler` is never invoked (no real interrupts delivered).
3. **DHCP sequence**: inject Offer (msgType=2) when UART shows
   "Wait DHCP Offer"; inject Ack (msgType=5) when UART shows "Offer IP=".
   Match the DHCP XID (firmware uses fixed `dhcp_xid = 0x87654321`).
4. **TCP handshake + HTTP** (already scaffolded): SYN when UART shows
   "Waiting for client", ACK when UART shows "SYN-ACK", GET+FIN when UART shows
   "REQ:". Track the client's seq/ack so the firmware's `srv_seq`/`srv_ack`
   bookkeeping stays consistent.
5. **Capture response**: the firmware splits the HTTP response into 3 TCP
   frames; each `eth_send_packet` TX must be captured from the TX descriptor.
   Stop when 3 chunks collected or UART shows "Done\r\n".
6. Optionally call `periph.eth_rx_done()` / `eth_signal_rx_poll(...)` to keep
   the Rust model's DMASR state consistent (currently not strictly needed since
   the firmware polls `eth_irq_flag` in SRAM).

### Validation checklist
- Full boot UART matches §6 exactly.
- All 5 injections fire in order (offer, ack, syn, tcpAck, get) — each prints an
  `[INJECT ...]` line.
- HTTP response text contains `HTTP/1.0 200 OK` and `Hello from STM32F407`
  (see the assertions at the end of `test_webserver_net.mjs`).

---

## 9. Gotchas / lessons learned

- The shell kills runs at its timeout; node buffers stdout on kill. Use
  `writeSync(1, ...)` for anything you must see, and redirect to a file.
- `uc.mem_write`/`mem_read` accept `BigInt` addresses in this binding; passing
  the hook-provided address directly works.
- Register reads from the reset vector: PC must be OR'd with 1 (Thumb). SP and
  PC come from 0x08000000/0x08000004.
- `periph_read` may return `undefined` for some addresses; guard before
  `mem_write`.
- Never start emu from a hardcoded 0x08000190 — that's mid-bss-loop. Use the
  reset vector value (0x08000185).
- After a wedged emu_start, the instance is unusable; re-create the uc and
  re-init rather than trying to recover.
- The step throughput degrades sharply over long runs (translation cache
  growth); prefer short runs and checkpointing to one long run.

---

## 10. Linux port (updated 2026-08-08) — this machine

Windows/AGENTS-doc paths above no longer apply on this Linux box. Current
facts:

### Toolchain (all installed)
- Rust + `wasm32-unknown-unknown`, `wasm-pack` 0.14.0 (`~/.local/bin`).
- Arduino core `STMicroelectronics:stm32` 3.0.0 (installed via arduino-cli;
  provides the xpack-arm-none-eabi-gcc toolchain below).
- Go toolchain (used to build the gateway).

### Build commands

```bash
# WASM peripheral model — MUST use --target nodejs (default bundler emits ESM
# that require() can't load on Node 22)
cd stm32-periph-wasm && wasm-pack build --release --target nodejs

# Firmware (bare-metal Makefile; toolchain from Arduino core)
TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-" \
  make -C eth_http          # also eth_dhcp, eth_test

# Gateway binary (sources are in openhw-local-gateway/; binary NOT committed)
cd openhw-local-gateway && go build -mod=vendor -o openhw-gw .
```

### Running the end-to-end test (proven working on Linux)

```bash
# 1) local HTTP server the NAT forwards to (127.0.0.1:8092)
node /tmp/opencode/http_server.js &

# 2) run emulator + gateway in one shot
cd stm32-periph-wasm/pkg
node cli.mjs ../../eth_http/eth_http.bin 10000000 --gateway --config=../../eth_http/config.yaml
#   RX_HEX=1 same command dumps the first 64 B of each injected RX frame
```

Expected round-1 UART: `=== HTTP ... ===` → DHCP Discover/Offer/Ack →
`TCP 010.150.211.085:8092` → `TCP SYN` → `TCP fl=12` (SYN-ACK) →
`TCP connected` → `Hello from openhw HTTP server` (the HTTP/1.1 200 body)
→ `TCP FIN` → `!CONN` → loop restart, DHCP renews OK.

### Multi-round support (fixed in cli.mjs)
The Go (gVisor-tap-vsock) gateway keeps the round-1 TCP session alive and
retransmits old server data at the new src port after the firmware restarts
(firmware prints `TCP fl=18` for those and ignores them). cli.mjs now
**auto-restarts the gateway** when a round-end marker (`=== HTTP ... ===`) is
seen in streamed UART, before the next round's DHCP Discover goes out.

Three fixes were needed to make consecutive rounds work:
1. **WS connect race**: the 1.5s dial timeout could fire while the gateway's
   HTTP upgrade was still completing, creating a dead second connection that
   stole the DHCP reply path. Now: 4s dial timeout + 600ms retry spacing.
2. **Round-boundary batching**: once `!CONN` is seen, batches drop to 1500
   instructions so the round-2 DHCP TX is processed after the restart, not
   into the dying gateway.
3. **RX queue wipe**: `restartGateway` used to clear `gwRxQueue`, destroying
   the round-2 DHCP Offer that had already arrived on the old connection.
   The queue now survives restarts (DHCP replies are stateless; stale TCP
   frames are filtered by the firmware's `fl!=0x12` check).

Verified: 200M-instruction run completes 3+ consecutive clean rounds
(DHCP → `fl=12` → HTTP body → `!CONN` → next round), each with its own
fresh gateway. `DBG_TX=1` / `DBG_RX=1` env flags add TX/RX frame traces.
Note: restart only works in `--gateway` mode (self-spawned); with
`--connect` the stale-session round may still fail.

Full 200M soak (2026-08-08, `SOAK_STATS=1`): 200,000,151 instructions in
870.8 s, **1012 TCP connections, 0 TCP fail, 0 SYN-ACK timeouts**. RSS
grew linearly 153→214 MB (~0.06 MB/round; not runaway, but not a
plateau — ~4 MB/min if a future run needs memory hygiene). First fully
completed 200M run; earlier 320 s/250 s `timeout` attempts were killed
mid-run, which is why soaks were incomplete before. Run command:
`SOAK_STATS=1 node cli.mjs ../eth_http/eth_http.bin 200000000 --gateway --config=../../eth_http/config.yaml`
(~14.5 min wall time; budget 20 min in the shell).

### XPSR restore fix (2026-08-08) — TCP fail after ISR pump
~1 round in ~2 failed at the ACK send with `TCP fail`, and `DBG_FLAG`
showed the ISR correctly OR-ing `eth_irq_flag`=1, yet the guest never
re-polled at 0x8001008. Root cause: `processInterrupts` (cli.mjs) saves
r0-r3/r12/lr/pc/sp + XPSR into a 32-byte frame at `savedAt-32`, runs the
guest `ETH_IRQHandler` (which aborts at the unsupported `bx lr`
EXC_RETURN), then restores — but the restore **omitted XPSR**. The
aborted ISR leaves its own condition flags; the guest resumes at
`0x8001004: beq.w 0x8001306` (the TX countdown's "exhausted → TCP fail"
branch) and that `beq` reads the stale Z flag from the ISR, jumping to
the fail path even though the countdown r3 was still ~5M.

Fix (cli.mjs):
```js
// Restore context from where we saved it (handlers may modify SP)
const savedFrame = uc.mem_read(BigInt(savedAt - 32), 32);
const savedSv = new DataView(savedFrame.buffer, savedFrame.byteOffset, savedFrame.byteLength);
uc.reg_write_i32(Module.ARM_REG_XPSR, savedSv.getUint32(0, true));
// ... then r0-r3, r12, lr, pc|1, sp
```
After the fix: 45 consecutive rounds with **0 TCP fail** (previously ~1
per round). The pattern to remember: any restore of a saved guest
context must include APSR/XPSR if the resume PC is a condition-code-
sensitive instruction. (Diagnosis used `DBG_FLAG=1` + `DBG_IRQF=1`:
`[FLAG] wr 0x20000620 = 1 pc=0x80009bc` shows the ISR flag landing while
`[IRQF] after-ISR r3=0x4 pc=0x-8` shows the machine left mid-abort.)

### Changes committed with this port
- `eth_http/eth_http.ino`: TCP src port now randomized per connect attempt
  (`49152 + ((tcp_attempt++*7) + port) % 2048`) — avoids stale-frame
  collisions between rounds.
- `stm32-periph-wasm/pkg/cli.mjs`: gateway path via `GW_PATH` env or
  repo-relative `openhw-local-gateway/openhw-gw`; `RX_HEX=1` frame dump.
- `stm32-periph-wasm/pkg/test_svd_run.cjs`: repo-relative firmware/SVD
  paths, `PROT_ALL` (the `PERM_ALL` from the docs doesn't exist).
- `stm32-periph-wasm/Cargo.toml`: added `[workspace]` so wasm-pack/cargo
  work from that dir.
- `openhw-local-gateway/`: fully tracked (sources + vendored deps) after
  removing its nested `.git`; the built `openhw-gw` binary, gw logs, and
  `*.bak` are ignored.

### Notes / gotchas on Linux
- The AGENTS.md §7 execution wedge was NOT reproduced under cli.mjs batch
  stepping (10M-instruction runs are fine).
- The repo's `webserver/` dir and `pkg/test_webserver_net.mjs` don't exist
  here; `eth_http` is the actual web-client firmware used for verification.
- Node buffers stdout: redirect to a file for long runs. The in-script
  timeout is 120 s; raise it for slow runs (10M instructions ≈ 5.6 s).
