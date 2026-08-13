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
20000 — capped below the §7 wedge threshold) with a code hook calling
`tick_n(delta)`, and `uc.emu_stop()` when TX/RX/DMA is pending or an
interrupt is pending; it then services TX/RX and resumes.

---

## 5. Test drivers in `pkg/`

> **Peripheral test reference:** `site/PERIPHERAL_TESTS.md` documents what
> every peripheral test (site/test_*.mjs) does, its expected UART output
> markers, and pass conditions.

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

> **STATUS (2026-08-10): the wedge does NOT reproduce on Node 22.22 (V8).**
> Fresh characterization with the exact repro paths — pure nop loops, firmware
> boot, mem hooks, no hooks, theory-café matrix in /tmp/opencode/wedge*.cjs,
> hang2.cjs, soak.cjs — shows count-based `emu_start` returning cleanly at
> n = 1000..1,000,000 both with and without memory hooks. Real cli.mjs soaks
> with `MAX_BATCH=500000` (303 no-gateway rounds + 2000+ gateway rounds,
> 150M instructions) complete without a single wedge; comprehensive_test
> (ISR pump) at MAX_BATCH=500000 also clean. The ONE guaranteed instance
> killer that is still reproducible is passing the **timeout argument** to
> `emu_start` (aborts with `qemu_thread_create: Not supported`, machine left
> unusable) — nothing in the repo passes it.
> Ruling: the old wedge was build/env-specific (likely an older Node/V8 WASM
> engine bug in the same Unicorn 2.1.4 build; the vendored `unicorn_arm.cjs`
> is byte-compatible with the official AlexAltea/unicorn.js v2.1.4 arm
> release — no newer build exists). `cli.mjs` default `maxBatch` raised
> 20000 → 200000 (env `MAX_BATCH`); measured ~2.96–3.2 MIPS (100M in 33.8s,
> 150M in 46.3s) vs ~2.1 MIPS at the old cap. History below is preserved
> for context.

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
- **2026-08-09: confirmed the wedge is instruction-count-per-call based, not
  region-based.** Under cli.mjs, an empty-RX-queue recv-wait spin (guest
  polling `eth_irq_flag` in SRAM) runs the full `emu_start` budget with no
  stop condition firing; a 500000-inst batch wedges the instance
  permanently (mid-soak, at ~90-150 rounds). The same soak with
  `maxBatch=20000` completes 600+ rounds cleanly. Fix: cli.mjs caps the
  main-loop and ISR-pump batch budgets at 20000 (env `MAX_BATCH`).

### Options already tried

- Stepped execution (STEP 3/10/100/500/5000) — stalls at same point.
- Continuous execution with pre-populated registers (no hooks) — stalls at
  ~40k instructions.
- Batch execution (20000/step) — batch 1 stalls.
- Unicorn built-in timeout — aborts (broken in this build).
- `uc.close()` after a wedged run — throws "memory access out of bounds".

### Options not yet fully explored

- **Block-stepping**: drive execution one basic block at a time via
  `HOOK_BLOCK` + `uc.emu_stop()` — obsolete since the wedge no longer
  reproduces on Node 22.22 (any `emu_start` budget returns; see STATUS).
- **Fresh Unicorn build**: the WASM port may be the problem; try a newer/native
  (Node addon) Unicorn, or recompile with assertions to locate the abort.
  Checked 2026-08-10: the vendored build IS the current release — the
  official unicorn.js v2.1.4 arm bundle (2026-06-19) is byte-comparable;
  no newer unicorn.js exists.
- **Reduce firmware code path**: build a minimal firmware that exercises the
  same region to isolate whether a specific instruction sequence (e.g. the
  `mov.w r3,#8000` + nop loop, or a Thumb-2 wide instruction) triggers it.
- **Check WASM heap growth**: `uc.mem_read`/`mem_write` malloc/free per call;
  heap growth mid-run is a suspect for the wedge (Emscripten `_resize_heap`).

---

## 8. NEXT PHASE — what is left to do

> **SUPERSEDED (2026-08-12):** this Windows-era plan (webserver.ino +
> `test_webserver_net.mjs`) is historical. The Linux port (§10) achieved the
> same goal through the gateway path: `cli.mjs` + `openhw-gw` (real gVisor
> stack) runs `eth_http` through DHCP → TCP → HTTP end-to-end, verified by
> `scripts/verify_ethernet.sh` and 100M-instruction soaks (§10). The
> `webserver/` dir and `test_webserver_net.mjs` do not exist in this repo.
> The text below is kept for context only.

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

# Firmware (bare-metal Makefile; toolchain from Arduino core).
# NOTE: the Makefiles use `TOOLCHAIN ?=` — the env-var override above works,
# and the committed eth_*.bin can be STALE relative to the .ino (the old
# eth_dhcp.bin was built from an older source: it read rx_frame_len from
# rdes0 bits [13:0] instead of [29:16], so DHCP Offer RX silently failed).
# Rebuild + re-verify after any .ino change:
TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-" \
  make -C eth_http          # also eth_dhcp, eth_test
# Configs for all three exist: eth_http/config.yaml, eth_dhcp/config.yaml,
# eth_test/config.yaml (each sets its own `load:` firmware in the ROM region;
# in config mode the positional firmware arg is IGNORED).

# Gateway binary (sources are in openhw-local-gateway/; binary NOT committed)
cd openhw-local-gateway && go build -mod=vendor -o openhw-gw .
```

### Running the end-to-end test (proven working on Linux)

```bash
# 1) local HTTP server the NAT forwards to (127.0.0.1:8092) — REQUIRED.
#    If it is down, the gVisor replies RST-ACK and the firmware prints
#    "TCP fl=14" (0x14 = ACK|RST) instead of the normal "TCP fl=12" SYN-ACK.
node /tmp/opencode/http_server.js &

# 2) run emulator + gateway in one shot
cd stm32-periph-wasm/pkg
node cli.mjs ../../eth_http/eth_http.bin 10000000 --gateway --config=../../eth_http/config.yaml
#   RX_HEX=1 same command dumps the first 64 B of each injected RX frame
```

All three ethernet firmwares pass: eth_http (44+ rounds, 0 TCP fail),
eth_dhcp (repeated DHCP SUCCESS), eth_test (TX completed, "ETH Test: done").

### Regression script
`scripts/verify_ethernet.sh [max_inst]` runs all three firmwares and
asserts the success markers (TCP connected / DHCP SUCCESS / ETH Test:
done) plus 0 `TCP fail` for eth_http. Starts the 127.0.0.1:8092 HTTP
server itself if it isn't up. Exit 0 = all pass.

Expected round-1 UART: `=== HTTP ... ===` → DHCP Discover/Offer/Ack →
`TCP 010.150.211.085:8092` → `TCP SYN` → `TCP fl=12` (SYN-ACK) →
`TCP connected` → `Hello from openhw HTTP server` (the HTTP/1.1 200 body)
→ `TCP FIN` → `!CONN` → loop restart, DHCP renews OK.

### Multi-round support (fixed in cli.mjs)
The Go (gVisor-tap-vsock) gateway keeps a TCP session alive briefly after the
firmware FINs and may retransmit old server data at a new src port after the
firmware restarts (firmware prints `TCP fl=18` for those and ignores them).
Historically cli.mjs **auto-restarted the gateway** when a round-end marker
(`=== HTTP ... ===`) was seen in streamed UART, before the next round's DHCP
Discover went out.

**2026-08-09: per-round restart is now OFF by default** (opt-in via
`GW_RESTART=1`). gVisor opens a fresh session per connection and the firmware
skips stale `fl=18` frames, so consecutive rounds work at full speed without
any restart. Verified 100M-instruction soak: **604 TCP connected, 0 TCP
fail, 0 `fl=18`, 0 timeouts, 47.9s (~12.6 rounds/s)** — beats the old
586-round/78s restart-mode record. The old restart mode measured ~1.3
rounds/s (each kill+spawn+reconnect costs ~0.7s). When `GW_RESTART=1`, the
restart is **deferred to the next-round TX** (the first frame after the
`=== HTTP` marker — usually the DHCP Request): restarting on the UART marker
alone was racy (the round-2 DHCP Discover could already be TX'd into the
dying gateway, and the firmware never retransmits the Discover → permanent
"Wait DHCP Offer" stall).

Legacy fixes that made consecutive rounds work (still in place):
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
Note: in `--connect` mode (external gateway) `restartGateway` cannot kill
the gateway process; instead cli.mjs sends a `RESET` control message
(WebSocket text frame) and reconnects. The gateway tears down the room,
closes the pipe, and nils its shared gVisor stack (`globalVN`), so the
next connection builds a fresh session table. Verified 2026-08-08: two
consecutive 15M-instruction `--connect` runs against the SAME gateway
process (the second running into 74 stale sessions from the first):
74/77 TCP connected, **0 TCP fail, 0 SYN-ACK timeouts** both runs
(150 `RESET requested` lines in the gateway log). `globalVN` is guarded
by `globalVNMutex` (handleProxy reads it under RLock) to avoid a nil-deref
race with the reset. Note: a stale gVisor stack is abandoned, not freed —
idle goroutines linger until the gateway exits (acceptable for `--connect`).

Connect-mode soak (2026-08-08, `SOAK_STATS=1`): two consecutive 100M-
instruction `--connect` runs against the SAME gateway process
(**1055 TCP rounds total**, 526 + 529 connected, **0 TCP fail, 0 SYN-ACK
timeouts**, 525 RESETs logged in run 1's gateway session). RSS ~193MB at
end of each run — the abandoned-stack leak stays bounded per run and does
not accumulate across runs (each RESET nils globalVN, so memory resets
with the room).

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

### Throughput work (2026-08-08) — 1.0 → 1.25 MIPS, +48% rounds/s
Measured on 20M-instruction `--connect` runs (eth_http, fresh gateway):

| Config | Steps | Rounds | Wall | MIPS | rounds/s |
|---|---|---|---|---|---|
| Baseline (old cli.mjs) | ~2600 | 101 | ~20.0s | ~1.00 | 5.05 |
| TICK_EVERY=1000 (no poll split) | 1939 | 67 | 11.05s | 1.81 | 6.06 |
| TICK_EVERY=5000 (no poll split) | 2473 | 95 | 13.9–14.14s | 1.44 | 6.8 |
| **POLL_EVERY=1000 + TICK_EVERY=5000 (final)** | **3061** | **118** | **15.8–16.0s** | **1.25** | **7.46** |

Final config reproducible: 118 rounds, 0 TCP fail, 0 timeouts (two runs).

100M soak with final config (2026-08-08, `SOAK_STATS=1`, fresh gateway):
**586 TCP connected, 0 TCP fail, 0 SYN-ACK timeouts** in 78.29 s (≈1.28
MIPS, 7.48 rounds/s) — up from the pre-speed 2x100M soak (526+529 rounds,
~1.0 MIPS). A repeat soak on a gateway polluted with 300+ stale sessions
failed at the first SYN with `TCP fl=10` + recv-wait stall (the documented
environmental flake) — restart the gateway (`kill <pid>`, relaunch
`openhw-gw -port 5099`) before long soaks.

**2026-08-09 update** (wedge fix + no-restart default, see §7): the same
100M soak now runs **604 TCP connected, 0 TCP fail, 0 `fl=18`, 0 timeouts
in 47.9s (~12.6 rounds/s)** with per-round restart OFF. The old restart-mode
figures above (7.48 rounds/s at ~1.3 MIPS) predate the `maxBatch=20000`
cap; restart-mode today measures ~1.27 rounds/s (each kill+spawn+reconnect
≈ 0.7s), so it is only for pathological stale-session cases (`GW_RESTART=1`).

Changes that produced it:
- **`tick_n(delta)` export in Rust** (`src/lib.rs`): `INSTRUCTION_COUNT.fetch_add(delta)` + `sys().tick()`. Safe to batch because timers are instruction-count-delta driven (`elapsed_ticks = (now-last_tick)/prescaler`) and eth.rs consumes `eth_take_done()` atomics. WASM rebuilt with `wasm-pack build --release --target nodejs`.
- **codeHook (cli.mjs)** does only `instCount++` per instruction plus the queue-stop, and batches the expensive WASM calls:
  - every `TICK_EVERY=5000` inst: `tick_n(5000)`, watchdog check, `has_pending_interrupt()` → stop.
  - every `POLL_EVERY=1000` inst: `dma_get_pending_count() > 0 || eth_is_tx_poll()` → stop.
  - every instruction (cheap, all-JS): `gwRxQueue.length > 0 && eth_is_rx_poll()` → stop (prompt RX injection).
- **smallBatch flip-back**: `smallBatch=true` on `!CONN` (round boundary, 1500-inst batches while the gateway restarts) flips back to `maxBatch=20000` when a UART chunk contains `Offer IP=` (DHCP re-established).
- **maxBatch=200000 default** (env `MAX_BATCH` override): 10x the old 20k
  cap (raised 2026-08-10 — the wedge no longer reproduces on Node 22.22;
  soaks at 500k batches are clean, see §7). A dead/unreachable gateway now
  just burns bigger batches in the DHCP wait instead of hanging.
- Removed the second main-loop `tick()` after processEth; removed unused `gwRestartWarned`.

Two failures along the way (both avoided in the final design):
1. **Unconditional queue-stop deadlock**: `if (gwRxQueue.length > 0) emu_stop()` froze the guest at 0x8000902 (recv-loop, 2 inst/step). Injection requires `eth_is_rx_poll()` armed (guest re-arms DMARPDR every 1024 loop iterations ≈ 9k inst at pc=0x800090c, `.ino` line 103), and processEth clears the poll at empty-queue boundaries — so a stop without the poll armed stalls forever. Queue-stop must AND `eth_is_rx_poll()`.
2. **Sticky RX poll** (clear only on delivery, else re-signal): 1 round / 163.74s regression (stall at PC=0x8000954 after "RESPONSE:"). Reverted to clear-always.

Gotchas: with `maxBatch=500000` and only-conditional stops, a dead/unreachable gateway (ws dial timeout, stale `WebSocket timeout` in the log) burns the whole budget in the DHCP wait with ~40-500k-inst batches and 0 rounds — always verify the gateway is listening (`ss -tlnp | grep <port>`) before trusting a 0-round result. The 95-round run from the intermediate config is NOT a code regression — it was a dead gateway. The `maxBatch=20000` cap (2026-08-09) makes even that case safe: empty-queue recv-waits now wedge-free (see §7).

### CAN bus peer / arbitration (2026-08-10)
The CAN model is now a real two-node bus (CAN1 + CAN2) instead of a
register file that completes TX instantly:

- **Staging**: a TXRQ mailbox write (TIR bit 0, not in init mode) pushes a
  `CanFrame` onto a global staged queue in `system.rs`
  (`CAN_STAGED` + `can_stage_tx`/`can_take_staged`/`can_restage`). TSR gets
  TXRQ + CODE bits at stage; TXOK/TME/RQCP are deferred to arbitration.
- **Arbitration**: `Can::arbitrate_bus(sys)` runs at every
  `WasmSystem::tick` (after peripheral ticks): lowest arbitration ID wins
  (ties: lower node, then mailbox index; single staged frame wins alone —
  so single-node TX behavior is unchanged). Winner's node gets
  `complete_tx` (TXOK bit 8+i, TME bit 16+i, RQCP bits 31/27/23). Losers
  are re-staged and complete on the next free round. TXRQ bits stay set
  (historic observable behavior; comprehensive_test checks TSR bit 0).
- **Delivery**: the winning frame broadcasts to EVERY node (including the
  transmitter — real CAN self-ACK traffic) whose filter banks pass it;
  under BTR bit 30 (LBKM loopback) it goes only to the sender. RX FIFO
  selection per FFA1R; 3 mailboxes per FIFO at the real addresses
  (0x1B0/0x1C0/0x1D0 FIFO0, 0x1E0/0x1F0/0x200 FIFO1 — `rx: [Mailbox; 6]`),
  FMP++/FULL/FOVR semantics; RFOM write-1 releases (FMP--, FULL cleared).
- **Filters**: 28 global banks (CAN2 maps local bank i to global 14+i via
  `filter_off`; the fm1r/fs1r/ffa1r/fa1r registers store bits shifted by
  `filter_off` and read back shifted). Mask/list modes, 32/16-bit scale
  (16-bit = two standard-frame entries), masks in bank word 2b+1.
- **Fixes along the way**: the minimal (no-SVD) register list in
  `Peripherals::new_wasm` was missing CAN1/CAN2 entirely — added, so CAN
  firmware works in browser/`new_wasm` builds too; RF0R/RF1R writes no
  longer clobber FMP/FULL/FOVR (only RTOM/bit4 and RFOM/bit5 are writable,
  matching the real w1c semantics).
- **`can_test/` firmware** (`make -C can_test`, polling — no interrupts):
  phase 1 BTR-LBKM loopback TX id 0x123 "CANLOOP!" verified end-to-end
  (id + payload constant check), phase 2 stages 0x200 (CAN2) and 0x300
  (CAN1) back-to-back and asserts both nodes drain 2 frames each and both
  TME bits set. `node site/test_can.mjs` asserts the full log.
- **Unit tests** (`can.rs` tests, 4): lowest-ID wins + broadcast +
  loser-retry, loopback self-delivery only, filter-gated delivery (mask
  mode), FIFO fill to 3 + FOVR + RFOM release.
- IRQ side effects preserved: TMEIE fires on TME bits 16..18 (plus legacy
  CODE bits 24..26 at stage time); RX FMPIE0/1 fire per FIFO; CAN2 base
  63 (TX=63, RX0=64, RX1=65, SCE=66). comprehensive_test's CAN1 section
  (TME0 + IRQ19 ISR) still passes.

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
- The AGENTS.md §7 execution wedge WAS reproduced under cli.mjs (2026-08-09):
  an empty-RX-queue recv-wait spin with no stop condition firing makes one
  `emu_start` batch run 40k+ instructions, wedging the Unicorn WASM instance.
  Fixed by capping `maxBatch` at 20000 (env `MAX_BATCH` override); the
  pump's ISR `emu_start` is likewise capped. On Node 22.22 the cap is
  unnecessary (see §7 STATUS); the default is now 200000.
- The repo's `webserver/` dir and `pkg/test_webserver_net.mjs` don't exist
  here; `eth_http` is the actual web-client firmware used for verification.
- Node buffers stdout: redirect to a file for long runs. The in-script
  timeout is 120 s; raise it for slow runs (10M instructions ≈ 5.6 s).

---

## 11. Browser demo + npm package (2026-08-09) — site/ and publishable library

### In-browser demo (`site/`)
- `site/index.html` + `site/app.js`: **single-page console**. Dark terminal
  UI: UART terminal, preset dropdown (eth_http/eth_dhcp/eth_test/blinky,
  `?fw=<name>` URL param), custom firmware upload (`.bin`, Intel `.hex`,
  `.elf` — RAM segments preloaded via `extra_mem`, symbols from symtab —
  and `.map` files for symbols only), Run/Stop/Reset buttons (Reset sends
  the gateway `RESET` control message when connected), a **gateway URL**
  field (WebSocket to openhw-gw `/api/network-gateway`; when connected all
  TX frames go to the real gVisor stack and RX frames are injected from it;
  otherwise netsim is the fallback), live GPIO pin grid for banks A–E
  (MODER/ODR/IDR read every frame via `read32`), key peripheral registers
  (ETH DMASR/MACCR, USART1 SR, RCC AHB1ENR), and a packet viewer.
- Serve with `python3 -m http.server 8123 --directory site` (SVD + wasm are
  fetched at runtime — file:// won't work). Browser build of the peripheral
  model lives in `site/vendor/` (`wasm-pack build --release --target web
  --out-dir ../site/vendor`); the Node build stays in `stm32-periph-wasm/pkg`.
  `unicorn_arm.js` (browser IIFE -> global `MUnicorn`) vs `unicorn_arm.cjs`
  (require) are two copies of the same module.
- `site/loaders.js`: `parseIntelHex`, `parseElf` (PT_LOAD segments split
  into flash/RAM + `extra_mem` preload list + symtab symbols), `parseMap`.
  Verified: hex/elf boot the blinky firmware through `createEmulator`.
- `site/emulator.js` is the **universal factory** (no imports — caller passes
  bindings/unicorn/svdXml/firmware): memory hooks over the whole peripheral
  space, codeHook with `tick_n` batching (TICK_EVERY=5000) + poll checks
  (POLL_EVERY=1000), TX capture + RX injection, optional `extra_mem`
  preload, and no interrupt pump (the firmware polls `eth_irq_flag` in SRAM;
  the driver writes bits 0/2 directly).
- `site/netsim.js`: canned DHCP Offer/Ack + TCP SYN-ACK + HTTP response
  (141B, fl=0x19) + bare ACK; no real stack. `site/test_flow.mjs` is the Node
  harness that asserts the whole flow (boot, DHCP, TCP connected, HTTP body,
  !CONN, >= 2 rounds) — `node site/test_flow.mjs`, PASS on exit 0.
- Gateway protocol (same as cli.mjs `--connect`): WS `ws://host:port/api/network-gateway`,
  binary frames = raw Ethernet both directions, text `RESET` = clear gVisor
  session. Browser smoke (CDP): /tmp/opencode/gw_smoke.mjs — connects,
  resets, asserts DHCP→TCP→HTTP through the real stack (passed: 3 rounds).
  Note: GitHub Pages is https, which blocks plain `ws://` — gateway mode
  needs the page served over http://.

### Non-ethernet demo (`blinky/` + `?fw=blinky`) — 2026-08-09
- `blinky/` is a bare-metal firmware with **no ETH/DMA/interrupts**: UART
  banner + `tick N LED=ON/OFF` prints + PA5 toggling every 100 ms via
  `GPIOA_MODER`/`GPIOA_ODR`. Built with the same Makefile pattern as
  `eth_test/` (startup.c `_start` does NOT zero .bss — keep globals static-
  initialized or explicit). Boot it in the console page with `?fw=blinky`;
  the GPIO panel's PA5 pin is read live from the emulated ODR
  (`read32(0x40020014) & 0x20`) every rAF — proves the peripheral model
  round-trips GPIO writes.
- `site/test_blinky.mjs` asserts boot banner + `tick 0/1` prints + ≥2 ODR
  toggles (~10M inst, 1.4 s). CDP smoke: /tmp/opencode/blinky_smoke.mjs
  (single-page console with `?fw=blinky`; chrome on port 9223).
- Gotcha: `delay_ms(n)` here is the `4000 nops/ms` convention — each
  `delay_ms(100)` is ~2.4M emulated instructions (6 inst/iter), so a full
  tick is ~5M inst (~2 s wall at 2.5 MIPS). Don't expect real-time blink
  rates in the emulator.

### The HTTP 000b bug (fixed 2026-08-09) — buffer-clobber race
Symptom: response consumed but `=== HTTP 000b ===`, one ACK with
ack=0x10000001, "TCP FIN" + "!CONN". Root cause (found via objdump of the
inlined tcp_recv in `loop`): the guest's `eth_recv_packet` (eth_http.ino:101)
re-arms `DMARPDR=1` at the END of consume, BEFORE tcp_recv parses the frame
(disasm: `bl 80008e0` -> r6=pl, then sp/dp/fl reads from `pkt`). With two
queued frames, the queue-stop fired mid-processing and `processEth` injected
the second frame into the SAME `rx_buf[0]`, so the guest parsed
pl=54 (old len) with the response's fl byte (0x19) -> td=0, fl&1=1 ->
ack=0x10000000+0+1, "TCP FIN", return 0. Fix: `rxInjectIdx` now rotates
`(idx+1) % rxDescs` (site/emulator.js) so consecutive injections land in
different RX descriptor slots. Verified: test_flow.mjs PASS (087b + body),
and a CDP-driven headless-Chrome smoke test completes 2 rounds.

### Interrupt pump — opt-in per firmware (fixed 2026-08-09)
The guest-IRQ pump (`processInterrupts` in site/emulator.js, ported from
cli.mjs) now runs **only when `enable_irqs: true`** (app.js enables it for
`rx_interrupt_test` + `rx_crypto_test` + `comprehensive_test` +
`eth_irq_test`). OFF by default — the polling ETH firmware (eth_http) is
corrupted by it: the driver signals TX/RX done by writing SRAM `irq_flag`
+ model DMASR, and a guest `ETH_IRQHandler` run on top re-reads DMASR and
re-scans `rx_desc`, stomping `rx_frame_idx/len` (observed: response body
followed by raw RX-buffer/TX-packet garbage and a mojibake `=== HTTP «75b
===`). In cli.mjs there is only ONE irq_flag writer (the ISR), so the pump
there is safe; emulator.js must never combine both.
**Interrupt-driven ETH firmware (`eth_irq_test/`, 2026-08-09)**: the proper
way to run the pump + ETH together. The firmware enables NVIC ETH IRQ 61
(ISER1 bit 29) + DMAIER (TSE|RSE|NIE); `ETH_IRQHandler` reads DMASR TS/RS,
sets its own SRAM `eth_irq_flag`, scans/re-arms RX descriptors, and
write-1-clears DMASR. The driver runs with `irq_eth: true` (emulator.js
opt, app.js `IRQ_ETH_FIRMWARES`): processEth skips the SRAM
irq_flag/rx_frame_idx/rx_frame_len writes — it only signals the model
(eth_tx_done/eth_rx_done) and injects frames. Verified: TX PING -> ISR
sets flag -> RX PONG -> echo -> "ETH IRQ Test: done" (Node test +
headless-Chrome smoke). Also fixed the RX descriptor format: drivers used
to write `(1<<28)|(1<<27)|(len<<16)` — the FS/LS marker bits at 28/27 fall
inside the frame-length window [29:16], so a guest ISR reading
`(rdes0>>16)&0x3FFF` saw `0x183C` instead of 60. Real F407 keeps FS/LS in
the low status word; drivers now write `len<<16` (eth_http tolerates
either). Firmware RX wait loops should re-arm `DMARPDR=1` periodically
(every 0x3FF iters like eth_http) so the model's poll desc addr tracks
DMARDLAR.
While wiring the pump, fixed a latent `rx_crypto_test` firmware bug: it
numbered USART1 as IRQ 38 (NVIC bit 6, vector slot 16+38) but the model
(and the real F407) pends USART1 as IRQ 37 — the ISR never ran. Fixed
startup.c (handler now in the IRQ 37 slot) + main.c (ISER1 bit 5). Also
fixed its Makefile (`TOOLCHAIN ?=`, output names `rx_crypto_test.*` —
previously built `comprehensive_test.*` and left the .bin stale). Rebuilt
and re-ran `tools/make_firmware.mjs`. Tests: `node site/test_rx_interrupt.mjs`
(PASS both RX firmware), `site/test_flow.mjs`, `site/test_blinky.mjs`.
`rx_crypto_test/run.mjs` is a legacy standalone harness (HOOK_INTR-based),
not the createEmulator path.

### npm package (prepare-only, dry-run verified)- Root `package.json` (`stm32f4-emulator`), `index.mjs` (Node API:
  `createSTM32F407({firmware})` + `decodeFirmware`/`createNetSim`/
  `createEmulator` re-exports), `tools/make_firmware.mjs` (regenerates
  `site/firmware.js` base64 blobs from `eth_*/eth_*.bin`; runs on `prepack`).
- `npm pack --dry-run` verified: 16 files, 1.2 MB tarball (includes
  site/vendor wasm + SVD + both unicorn copies). NOT published. Consumer test
  (install tarball into /tmp/opencode/pkgtest, `consumer.mjs`) completes a
  full HTTP round — exit 0. Exports map covers `.`, `./emulator`,
  `./netsim`, `./firmwares`, `./vendor`, `./site`.
- **Gotcha**: wasm-pack writes `site/vendor/.gitignore` containing `*` —
  delete it after each rebuild or git/npm pack will silently drop all vendor
  assets (tracked: ~4.8 MB). `*.tgz` is gitignored.

### Gotchas
- **Boot failures are now visible** (2026-08-11): app.js adds an
  `unhandledrejection` listener (the page previously showed only a silent
  "Uncaught (in promise)" + stuck `booting…` status), and emulator.js wraps
  `uc.mem_map` with labeled errors incl. `uc.mem_regions()` on failure.
  Intermittent `uc_mem_map failed with code 1` (ARG) on the 1.75GB periph
  range was traced to **system memory pressure on this 16GB box (~1.2GB
  free)** — it killed Chrome (renderer OOM) and corrupted long-lived
  instances; a fresh headless-Chrome instance passes 60+ consecutive
  ?fw= navigation boots and 8/8 same-page click-boots with the JS heap
  bounded at 15-17MB. If boots start failing with mem_map errors, restart
  Chrome (fresh `--user-data-dir`) — the page code is not at fault.
  `performance.memory` only counts the JS heap, NOT the wasm heaps
  (unicorn ~40MB + rust per boot) that actually accumulate.
- Headless-chrome `--virtual-time-budget` + `--dump-dom` throttles rAF to a
  few frames — it will NOT complete a run. Use the CDP driver
  (script: /tmp/opencode/site_smoke.mjs) for browser verification.
- The browser demo keeps 1 `emu.step()` per rAF; a step is up to 100k
  instructions, so a full round completes in a few frames.
- **`<input>.value` strips CR/LF (HTML spec)** — the UART RX input box cannot
  send `\n`/`\r`, so newline-terminated RX firmware (rx_interrupt_test,
  rx_crypto_test) can't be driven from the UI box. The CDP smoke
  (`/tmp/opencode/rx_smoke.mjs`) sends control chars via `window.__emu
  .sendUart([...])` — app.js exposes `window.__emu`/`window.__bindings`
  debug handles for this (verified 2026-08-09: both RX firmwares reach
  `CRC=EFE8B569` / `INT CRC matches polling` in the browser in ~0.4 s).
- `rx_interrupt_test`'s interrupt pump also services the model's SysTick
  pending (Arduino core configures it); each service runs the real
  SysTick_Handler, which aborts on `pop {pc}` with the fake EXC_RETURN LR —
  cheap, but the pump should never be enabled for ETH firmware (see the
  interrupt-pump section).

---

## 12. I2S audio + LTDC display + DMA depth (2026-08-10) — WAV DMA, scanout sink

### I2S/SAI audio (WAV-backed, DMA-driven)
- `audio_load_wav(wav_bytes)` (lib.rs export) parses RIFF/WAVE (fmt PCM16 +
  data chunks) into a global sample source. I2S/SAI DR reads consume the
  next sample (or `generate_i2s_audio()` when no WAV was loaded); DR writes
  push into a capture FIFO drained via `audio_take_capture()`.
- **SPI1/I2S1 share one silicon register block at 0x40013000**; the map
  registers it as `Spi::new` (wins over `I2s::new`). The I2S routing lives
  in `spi.rs`: when `i2scfgr & 1` (I2SMOD) is set, DR (0x0C) reads take the
  audio path and writes push capture samples. Do NOT add a separate I2S
  peripheral to the map for 0x40013000.
- `audio_test/` firmware: I2S2? — no: SPI1/I2S1 block. I2SCFGR=1 (I2SMOD),
  DMA1 Stream0 PERIPH->MEM from I2S1_DR (0x4001300C), 64 16-bit samples,
  checksum 0x93C40 over samples `(i*300+7)&0xFFFF`; then TX writes 16 words
  1000..1015 to DR. Marker "RX n=64 sum=93C40" (8-digit hex, NO `0x` prefix
  in the marker — matches printf `%08X`) + "TX n=16 OK". Harness:
  `node site/test_audio.mjs` loads a generated Buffer WAV via
  `bindings.audio_load_wav` right after `createEmulator` returns (before
  the run loop starts).
- DMA details that made it work:
  - **PINC + PSIZE**: `DmaTransfer` carries `pinc` (sidestream wander) and
    `p_size` (peripheral width). `dma_periph_read(addr,size,pinc,psize)`
    chunks at PSIZE and re-reads the SAME address when PINC=0 — a fixed
    16-bit DR FIFO now yields a contiguous sample stream (previously
    4-byte chunks with zero padding). JS call sites (emulator.js
    processDma + pkg/cli.mjs): `dma_periph_read(peri_addr, size,
    (pending[7]||0)===1, pending[8]||4)`; the transfer vector is now
    9 elements [dir, stream, src, dst, size, peri_addr, peripheral,
    pinc, p_size].
  - **Completion latching**: TCIF(1<<4)/HTIF(1<<3) are NO LONGER set at
    CR-EN write. The JS driver calls `dma_set_completed` when the transfer
    finishes; the guest's LISR/HISR read latches them into stream status
    via `dma_check_completion(i)` (atomically consumed, IFCR w1c clears).
    Needed because firmware polls LISR then RE-READS it after masking.

### LTDC scanout + browser sink
- Model (`ltdc.rs`): when GCR (0x18) has LTEN (bit 0) + L1CR (0x84) has
  LEN (bit 0), `scan_tick` advances a 2-px-per-call scanline driven by the
  real geometry regs (SSCR 0x08, BPCR 0x0C, AWCR 0x10: line span =
  hsw+1+hbp+W-1, frame = vsw+1+vbp+H-1 lines). At LIPCR (0x40) the
  `ltdc_lif` flag (LIF bit 0) is set and IRQ 88 pends; at frame end the
  F flag (bit 1) is set and `frame_count` increments. Exports:
  `ltdc_get_scanline`, `ltdc_get_frame_count`. 1 tick = 1 inst, so a
  64×32 frame costs ~82px×49 lines... scans at 2px/tick (41 ticks/line).
- `ltdc_test/` firmware: framebuffer PINNED at 0x20002000 via macro (the
  .fb linker section was unused — don't rely on it), 64×32 ARGB8888 with
  pixel(x,y)=0xFF000000|x<<16|y<<8|(x+y); SSC R = 8-1, BPCR 9-1, AWCR
  (H-1, W-1), LIPCR=48, IER=0xF; layer0: CR 0x84 (needs ctrl bit),
  PFCR 0x94 = 0 (ARGB8888), **WHPCR 0x88 / WVPCR 0x8C = (w-1)|((h-1)<<16)
  (must be set — JS dimensions come from them)**, CFBAR 0xAC,
  CFBLR 0xB0 = pitch<<16|linebytes, CFBLNR 0xB4 = H. Waits on ISR (0x38)
  bit 1 (F flag). Checksum 0xFC7F1F9E. Harness `site/test_ltdc.mjs`:
  asserts markers, extra 300 steps after "pixels OK" so frames accumulate,
  `ltdc_get_frame_count() >= 2`, per-pixel spot checks.
- Browser: `site/index.html` LTDC panel (canvas 320×160 pixelated,
  `#ltdcInfo`); `site/app.js` `renderLtdc()` runs in the rAF loop after
  refreshStats — reads GCR/L1CR/PFCR/CFBAR/CFBLR/CFBLNR/WHPCR/WVPCR,
  supports pf 0 (ARGB8888) + 2 (RGB565), reads rows from guest RAM with
  pitch, cache key includes frame count (repaints only on new frames),
  draws via an offscreen canvas + drawImage. Note: width falls back to
  CFBLR line bytes if WHPCR is 0 (unconfigured layer).
- Browser smoke: `/tmp/opencode/ltdc_smoke.mjs` (headless Chrome CDP via
  global WebSocket, site served on 8123) asserts `#ltdcInfo` text
  "layer0 64×32 ARGB8888 @ 0x20002000". `?fw=ltdc_test` in the console.

### cargo test parallelization (fixes for process-global races)
- The core problem: `ExtDevices` is `unsafe impl Send+Sync` and holds
  Rc<RefCell> links; Rust 2024 runs lib tests in parallel THREADS, so
  shared mutable state across tests raced (RefCell "already borrowed" @
  spi.rs:80, CAN "TXOK0 after free round", SPI "jedec 0xFF", audio
  "wav_parse/capture").
- Fixes (all in the model crate):
  - `test_dummy_system()` (system.rs): now builds `WasmSystem` directly
    with an EMPTY ExtDevices (no `register_software_spis`) — it no longer
    calls `WasmSystem::new()` (global read) → full isolation.
  - `spi.rs::sys_with_flash` must NOT use test_dummy_system (unregistered
    spis would crash); uses `WasmSystem::new()` directly.
  - can.rs tests: `static CAN_TEST_LOCK: Mutex<()>` locked in all 4 tests
    (global CAN_STAGED queue is shared).
  - system.rs audio_tests: `static AUDIO_TEST_LOCK` locked in all 4 tests
    (global AUDIO_SOURCE/AUDIO_CAPTURE).
- Verified: `cargo test` 15/15 green across 8 consecutive runs
  (previously flaky). NVIC gained a pub accessor `irq_pending(irq)`
  (raw pending, regardless of ISER) used by the ltdc tests.

### Regressions this session
- cargo 15/15 · site tests: blinky, eth_irq, can, audio, ltdc, flow,
  rx_interrupt (rx_crypto_test) all PASS · comprehensive_test via
  pkg/cli.mjs: 42 PASS, "FAIL: 00000000" (0-count counter, not a
  failure) · browser LTDC smoke PASS. `node site/test_audio.mjs` +
  `node site/test_ltdc.mjs` + `node tools/make_firmware.mjs` (24
  firmwares, includes audio_test + ltdc_test) + `?fw=audio_test` /
  `?fw=ltdc_test` dropdown entries.

---

## 13. Protocol taps (SPI/I2C) + DCMI frame feed (2026-08-11)

Protocol-agnostic chip-side taps: the JS driver watches byte traffic on a
peripheral and can answer it, without any device protocol knowledge in the
Rust model. All register-before-`init()`.

### Exports (both `pkg` Node build and `site/vendor` browser build)

| Export | Purpose |
|---|---|
| `spi_tap(peripheral, cs?)` | Register a tap on an SPI peripheral (e.g. `'SPI1'`, `'PA4'`). While the CS pin is LOW (real GPIO ODR gating via `register_cs_callbacks` + `sel_state`), every byte the master writes to DR is queued as an event. |
| `spi_push_miso(peripheral, bytes)` | Queue bytes the tapped device answers on master DR reads (0xFF when empty). |
| `spi_take_events(peripheral)` | Drain the event queue (u32 per event). Byte events: `v & 0xFF`. CS events: bit31 set, bit30 = 1 HIGH / 0 LOW. Events are pushed per byte and per CS edge (edge-triggered, change-only). |
| `i2c_register_slave(peripheral, address)` | Register a tap slave (e.g. `'I2C1'`, `0x3C`). Pushed into `ExtDevices.i2c_taps`, picked up by `find_i2c_devices` like an eeprom. |
| `i2c_take_tx(peripheral)` | Drain all bytes the master wrote to the tapped slave since the last call. |
| `i2c_push_rx(peripheral, bytes)` | Queue bytes answered on master reads (in order; 0xFF when empty). |
| `dcmi_feed_frame(w, h, pixels)` | Feed a complete frame (YUV/RGB raw bytes) into the DCMI; `consumes_js_fed_frame_with_line_and_frame_flags` cargo test covers it. |

### I2C bus semantics you MUST respect in a smoke/driver
The model mirrors real F407 register semantics — a raw DR poke does NOT
route bytes. Correct master flow (verified end-to-end):
`CR1 START` (bit 8) → `DR = (addr<<1)|rw` → read SR1 (0x14, latches ADDR)
→ read SR2 (0x18, AddrSent→Active) → DR data writes now route to the tap.
The AddrSent→Active transition lives ONLY in the SR2 read path
(i2c.rs:107-118); there is no DR-write arm for AddrSent. SR1 is 0x14,
SR2 is 0x18 (not 0x08/0x0C — those are OAR1/OAR2).

### SPI CS gating
Bytes only route while CS is asserted (pin low). No CS pin (`cs` omitted)
means always selected. CS edge events fire on ODR change only (the
callback tracks `prev`), so the initial CS state does not spam the queue.

### wasm-bindgen glue change (Node build) — gotcha
The current wasm-bindgen emits a **synchronous** nodejs glue: `require()`
instantiates the wasm immediately (reads `_bg.wasm` from `__dirname`,
calls `__wbindgen_start()`); there is NO `default` export anymore (an
`init()` exists but is not needed). `site/emulator.js` now guards with
`typeof bindings.default === 'function'` and skips init when absent.
The browser build (`site/vendor`) still exports `__wbg_init as default`
(ESM). cli.mjs never called `default` so it is unaffected.

### wasm-pack out-dir clean — gotcha
`wasm-pack build --release --target web --out-dir ../site/vendor`
DELETES the out-dir contents, including the manually-placed
`stm32f407.svd` + `unicorn_arm.js`/`unicorn_arm.cjs` that
`site/index.html`/`app.js` fetch at runtime, and writes a `.gitignore`
containing `*` (which would silently untrack vendor assets). After any
vendor rebuild: restore the three files from `pkg/unicorn_arm.{js,cjs}`
and `monox/stm32f407.svd`, then `rm site/vendor/.gitignore`.

### Removed ext_devices
`display.rs`, `lcd.rs`, `touchscreen.rs`, `usart_probe.rs` were deleted
(unreferenced; superseded by the tap approach). `ext_devices/mod.rs` now
carries `spi_taps`/`i2c_taps` vecs; `find_spi_devices`/`find_i2c_devices`
append them to the device list so CS/addr routing is shared with the
flash/eeprom devices.

### Verification (2026-08-11)
- `cargo test --release`: 16/16 (incl. `firmware_flow_via_gpio_cs`,
  `consumes_js_fed_frame_with_line_and_frame_flags`).
- Node smoke (pkg): SPI tap bytes `x12,x34` + CSL event + MISO readback
  `0xAB`; I2C TX `0xde,0xad`, I2C RX `0xbe 0xef` in order; DCMI feed no
  throw. Script pattern in `/tmp/opencode/tap_verify5.cjs`.
- Site suite 16/16 exit 0 (current, §15): flow, blinky, audio, ltdc, can,
  dma (comprehensive_test, `FAIL: 00000000` = known 0-count artifact),
  eth_irq, exti, flash, rx_interrupt, spi_flash, oled, tft, buzzer,
  audio_play, rtc. (At the time of §13 it was 11/11 — the four device
  tests + rtc came later.)

---

## 14. Peripheral devices: OLED / TFT / buzzer / speaker (2026-08-11)

Four JS-visible "peripheral devices" driven through the existing protocol-tap
machinery (§13) + model registers. All of them are **opt-in per firmware**
via `ext_devices.{oled,tft,buzzer,speaker}` (browser: `DEVICE_FIRMWARES` in
app.js; Node: the `ext_devices` arg of `createEmulator`).

| Device | Transport | Parser location | Firmware | Node test |
|---|---|---|---|---|
| SSD1306 OLED 128×64 | I2C (I2C1 @ 0x3C) | `site/emulator.js` `processOled` | `oled_test/` | `site/test_oled.mjs` |
| ILI9341 TFT 240×320 RGB565 | SPI (SPI2, CS PB12, DC PB11) | `site/emulator.js` `processTft` | `tft_test/` | `site/test_tft.mjs` |
| Buzzer (TIM PWM) | none — reads TIM regs | `site/emulator.js` `processBuzzer` | `buzzer_test/` | `site/test_buzzer.mjs` |
| Speaker (I2S capture) | none — drains model capture | `site/emulator.js` `processSpeaker` | `audio_play_test/` | `site/test_audio_play.mjs` |

### I2C tap START/STOP boundaries (Rust, i2c.rs)
`i2c_register_slave` already pushed per-byte events; this session added
transaction boundaries so multi-byte command/data groups parse correctly:
- **START** (CR1 bit 8 rising edge, i2c.rs:147): pushes `(1<<31)|(1<<30)`
  and clears `active_device` (next DR write = address byte).
- **STOP** (CR1 bit 9, i2c.rs:157): pushes `1<<31`; Active/AddrSent state
  resets (real w1c-ish behavior preserved: the firmware's START→addr→SR1/SR2
  latch flow in §13 is untouched).
- JS consumers: `ev & 0x80000000` = edge event; `ev & 0x40000000` = START
  (for I2C: begin a new group — the next byte is a control byte); plain STOP
  ends the group. Same bit layout as the SPI CS events.

### processOled (SSD1306, page-addressed framebuffer)
- On START, the next byte is a **control byte**: 0x00 = command group,
  0x40 = data group (`needControl` latch).
- Command state machine: page 0xB0-0xB7, col low 0x00-0x0F, col high
  0x10-0x1F, arg commands (0x20/0x21/0x22/0x81/0x8D/0xA8/0xD3/0xD5/0xD9/
  0xDA/0xDB take 1 arg); single-byte commands (0xAE/0xAF/0x40/0xA4/0xA6)
  ignored.
- Data bytes: 8 vertical bits per column into `fb[(page*8 + bit)*128 + col]`
  (column-major, row-major `fb[bit*128+col]` in the renderers/tests).
- `frame` counts event batches. Exposed as `emu.oled = { fb, frame() }`.

### processTft (ILI9341, RGB565)
- CS asserted (edge) resets the transaction; `dc` bit = (ev>>29)&1.
- Commands with args: 0x2A/0x2B (4), 0x36/0x3A/0x35/0x53 (1), 0xC0 (2),
  0xC1 (1), 0xC5 (2), 0xC7 (1), 0xE0/0xE1 (15), 0xF6 (3). Only 0x2A/0x2B
  args are interpreted (window); everything else is consumed.
- 0x2C enters pixel-write mode: two bytes (big-endian RGB565) per pixel,
  `fb[(y*240 + x)*2]` big-endian (renderTft decodes `(fb[p]<<8)|fb[p+1]`).
  `frame` increments per row wrap; window wrap returns to (x0,y0).
- Exposed as `emu.tft = { fb, w, h, frame() }`.

### processBuzzer (TIM PWM probe)
Reads TIM2 (default; TIM3/4/5 via `ext_devices.buzzer.tim`) CR1/CCER/PSC/ARR/
CCR1 directly from the modeled registers: `freq = 84MHz/((psc+1)(arr+1))`,
`duty = ccr/(arr+1)`, active when CR1 bit 0 (CEN) + CCER bit 0 (CC1E).
`change` counts freq/duty transitions (note changes). Exposed as
`emu.buzzer = { freq, duty, change }` (getters).

### processSpeaker (I2S TX capture drain)
`audio_take_capture()` (Rust, I2S1 DR writes, see §12) drained each
`processDevices` pass into a Float32 ring (`emu.takeSpeakerSamples()`).
Browser `renderSpeaker` (app.js:479) schedules chunks through WebAudio
(BufferSource chain, `audioNextTime`). `audioCtx` is created lazily and
closed/recreated per boot (autoplay policy: created on first samples).

### Browser UI
`site/index.html` aside panels: `#oledCanvas` (128×64, putImageData) +
`#oledInfo`, `#tftCanvas` (240×320) + `#tftInfo`, `#buzzerInfo`,
`#speakerInfo`. `renderDevices()` (app.js:506) runs in the rAF loop;
renderers are frame-cache-keyed (no repaint when nothing changed).
`?fw=oled_test|tft_test|buzzer_test|audio_play_test` in the console page.

### Firmwares (all bare-metal, same Makefile pattern as blinky/)
- `oled_test/`: SSD1306 init sequence, text page ("F407 OLED"), empty rows,
  solid bottom bar; prints "OLED init done" / "OLED draw done".
- `tft_test/`: ILI9341 init, four color quadrants (R/G/B/W 0xF800/0x07E0/
  0x001F/0xFFFF), "TFT init done" / "TFT draw done".
- `buzzer_test/`: TIM2 CH1 PWM 294 Hz @ 50% duty, two note changes
  (on→off→on), "BUZZER note A" / "BUZZER done".
- `audio_play_test/`: I2S1 TX (SPI1 block 0x40013000, I2SMOD set) writes a
  256-sample sine table repeatedly (continuous tone).

### Tests
- Node: `node site/test_oled.mjs` (textPixels > 30 on page 0, barPixels =
  128*8 on page 7, empty pages 1/2/4/6), `site/test_tft.mjs` (quadrant
  colors + done markers), `site/test_buzzer.mjs` (294 Hz / 50% / 2 changes),
  `site/test_audio_play.mjs` (samples drain non-zero).
- Browser smoke (`/tmp/opencode/devices_smoke.mjs`): boots all five
  device presets via `?fw=` navigation on headless Chrome (port 9223, site
  on 8123), asserting OLED lit=1452 + bar=1024, TFT quadrant pixels
  `#f80000,#00fc00,#0000f8,#f8fcf8` (RGB565: red 0xF800 → #f80000, green
  0x07E0 → #00fc00, blue 0x001F → #0000f8, white → #f8fcf8), buzzer
  "294 Hz, duty 50%", speaker "playing", RTC `10:45:30 ... temp=27.50 C`.
  NOTE: it restarts Chrome between every boot — the 1.75GB periph
  `mem_map` fails with UC_ERR_NOMEM once wasm heaps accumulate (see §11).

### Gotchas
- **TFT framebuffer byte order**: `processTft` stores RGB565 **big-endian**
  (`fb[off] = hi, fb[off+1] = lo` — bytes as transmitted), so `renderTft`
  and `test_tft` must decode `(fb[p]<<8)|fb[p+1]`, NOT little-endian.
- **I2C group boundaries**: without the START/STOP events, command args run
  into data groups and the framebuffer shifts. The firmware MUST use
  START/STOP per group (the Arduino-style SSD1306 drivers do).
- **audio_play_test boot order**: `createEmulator` must be created before
  the run loop; the capture FIFO only fills while the guest writes DR.
- **Timing**: `buzzer_test` and the I2C/SPI parsers are step-count-driven —
  the smoke boots each firmware and waits for the terminal marker before
  reading device state (no fixed wall-clock sleeps).
- **Reboots**: boot failures were silent before 2026-08-11 (see §11 Gotchas
  — unhandledrejection + labeled mem_map). Device panel state resets per
  boot via `oledCacheKey/tftCacheKey/buzzerCacheKey = ''` in `boot()`.

---

## 15. DS3231 RTC device + I2C register-file taps (2026-08-11)

A pointer-addressed register-file I2C device (DS3231 RTC semantics), the
fifth virtual peripheral. Unlike the SSD1306/TFT (pure protocol taps
parsed in JS), the RTC's read path pre-pushes bytes at address-match —
racing a pure-JS parser — so it needed a **device-side tap in Rust**.

### Rust (`i2c_regfile.rs`, ext_devices/)
- `I2cRegFile`: register file with a 1-byte pointer. First write byte of
  each transaction = register pointer (phase `Addr`), subsequent bytes land
  at `ptr++`; reads return `regs[ptr++]`. The pointer **persists across
  address matches** (real DS3231: reads continue from the last-accessed
  register); `reset()` (i2c.rs calls it per address match) only re-arms the
  pointer-byte phase for the NEXT write transaction. Out-of-range pointers
  clamp `% size`; `get(offset)`/`set(offset,v)` for JS pokes.
- Registered like the eeprom: `ExtDevices.i2c_regfiles` vec + entry in
  `find_i2c_devices` (`i2c-regfile` device name) so the address ACK/routing
  machinery is shared.
- Exports: `i2c_register_regfile(peripheral, address, size, init)` (must be
  called before init), `i2c_regfile_get(peripheral, offset) -> u8`,
  `i2c_regfile_set(peripheral, offset, value)` (JS-side poke, e.g. ambient
  temperature changes from outside the guest).
- Unit tests (3, in-module): pointer-write-then-read starts at the pointer;
  data writes auto-increment from the pointer (reads then continue after the
  last written register); out-of-range pointer clamps.

### Firmware `rtc_test/` (bare-metal, I2C1 master, same driver as oled_test)
- Writes pointer 0x00 + 7 BCD bytes (sec 30/min 45/hr 10/dow 3/day 15/mon 7/
  yr 26) in ONE transaction (auto-increment), then a pointer-only transaction
  + streaming read of 7 bytes (pointer persistence), verifies all 7
  ("RTC verify OK"), then reads the temp pair at 0x11/0x12 and prints
  "RTC temp=27.50". Markers: `RTC set done` / `RTC read done` /
  `RTC time=10:45:30 DOW=3 15/07/26` / `RTC test done`.
- I2C read flow the model requires: START → `DR=(addr<<1)|1` → wait ADDR →
  read SR1 (latch) → read SR2 (Active + RXNE armed, byte prefetched) →
  per byte: wait SR1 bit 5 (RXNE), read DR (returns prefetched byte and
  prefetches the next — the last read prefetches one harmless extra byte).

### emulator.js / app.js / index.html
- `ext_devices.regfile[]` is the generic list; `ext_devices.rtc` is shorthand
  (regfile at 0x68) that ALSO enables `emu.rtc` = live BCD-decoded time
  (`{sec,min,hour,dow,day,mon,year}`), `temp` (signed MSB + (LSB>>6)*0.25),
  `change` counter. Seed via `init` (20-byte &[u8]: BCD time 0x00-0x06, temp
  MSB/LSB 0x11/0x12; the guest overwrites time, temp stays read-only).
- `site/index.html`: dropdown entry + `#rtcInfo` panel ("RTC DS3231 (I2C)");
  app.js `DEVICE_FIRMWARES.rtc_test` with `RTC_INIT` seed, `renderRtc()`
  (frame-cache-keyed, `rtcCacheKey` reset in boot()).

### Tests
- `cargo test --release`: 19/19. Site suite 16/16: flow, blinky, audio,
  ltdc, can, dma, eth_irq, exti, flash, rx_interrupt, spi_flash, oled, tft,
  buzzer, audio_play, **rtc**.
- `node site/test_rtc.mjs`: asserts the 7 markers + `emu.rtc.time` BCD
  decode (10:45:30 dow3 15/07/26) + `temp === 27.5` + `change === 1`.
- Browser smoke (`/tmp/opencode/devices_smoke.mjs`, 5 devices): RTC panel
  shows `time 10:45:30 DOW=3 15/07/26 temp=27.50 C` + `RTC verify OK` in the
  UART. NOTE: the smoke now **restarts Chrome between every boot** — the
  1.75GB periph `mem_map` fails with UC_ERR_NOMEM once wasm heaps accumulate
  after ~4 boots on this memory-starved box (see §11).

### Gotchas
- Regfile test expectations: after a pointer+data write transaction, reads
  continue from the register AFTER the last written one (pointer advanced),
  NOT from the original pointer — the first test iteration got this wrong.
- The `rtc` device config registers the regfile AND enables `emu.rtc`;
  passing only `regfile` in a test yields `emu.rtc === null` (silently).
- BCD: `bcd2n`/`bin2bcd` in JS/firmware; the DS3231 dow register is raw
  (not BCD) — stored as 3 = 0x03, BCD decode yields 3 either way.

### Web demo UX pass (2026-08-12)
- **UART RX terminal input fixed**: the input box strips CR/LF (HTML spec),
  so newline-terminated RX firmwares could not be driven from the UI. The
  keydown handler now appends the terminator itself: Enter → sends the line
  + `\r` (0x0D), Shift+Enter → `\n` (0x0A), empty Enter → bare `\r`.
  Verified in-browser: rx_interrupt_test gets "Hello\r" and prints
  `CRC=F1AFE56C`. Button renamed "Send RX" → "Send".
- **Gateway status counters**: the status label is now refreshed on every
  frame (`refreshGwLabel` in onTx/onmessage) — it previously only updated at
  connect/disconnect, so a run showed "0 TX / 0 RX" forever despite traffic.
  Counters reset per boot. **Gateway RX frames now appear in the packet
  viewer** (`addFrame('rx', buf)` in onmessage — netsim replies already did).
- **Save log button**: downloads `uart-<fw>.txt` (Blob of the terminal
  buffer).
- **can_test preset added** to the dropdown (firmware bundle is now 30
  firmwares; `tools/make_firmware.mjs` gained the can_test entry).
- Browser smoke for all of it: `/tmp/opencode/webux_smoke.mjs` (can_test
  done-marker, rx_interrupt CRC via the UI input box, save button);
  `/tmp/opencode/gw_web_smoke.mjs` (boots eth_http in the page with the real
  gateway: assert TCP connected + HTTP body + !CONN + live TX/RX counts +
  gateway RX frames in the viewer). Gateway smoke needs openhw-gw on 5099
  (relaunch with `setsid nohup ./openhw-gw &` — `pkill -f openhw-gw`
  self-matches the shell, use the full path pattern or kill by pid) and the
  local HTTP server on 8092 (`node /tmp/opencode/http_server.js`).

---

## 16. DOOM (doomgeneric F407 port) — 2026-08-13

A full DOOM 1 shareware (doom1.wad) runs on the emulated STM32F407: boot →
title/attract → menu → New Game → episode → skill → E1M1 gameplay with the
320×200 CMAP256 framebuffer rendered through a guest-exported BGRA palette.
Deterministic: `node site/test_doom.mjs` PASSes 3/3 with identical numbers.

### The port (`doom/`)
- doomgeneric (github.com/ozkl/doomgeneric) with a `f407` platform target:
  `main_f407.c` = `doomgeneric_Tick()` + `DG_SleepMs(15)` busy-wait loop.
- Memory (link.ld): flash 0x08000000, SRAM: stack at top + `.abi` NOLOAD
  section pinned at 0x20002000; `.data`/`.bss` at 0xC0000000 (16MB EXTRAM);
  `__heap_start` after bss. Runtime layout: ZONE = 0xC0100000 (6MB),
  heap = 0xC0700000 (9MB). WAD image is loaded by the JS driver at
  0xB8000000 (8MB) via `extra_mem` — the firmware never reads the WAD from
  flash; it sees a "file" through `doom_wad_name`/`doom_file_exists` shims
  in platform.c that redirect to 0xB8000000 (lumps are accessed via
  `W_AddFile` → fread on the memory image).

### Guest↔driver ABI (doom/f407/doomplatform.h) at 0x20002000
- `+0x00` u32 keyWr (JS writes), `+0x04` u32 keyRd (guest writes),
  `+0x08` 256-byte ring, **2 bytes per event** (keycode byte, `0x80|pressed`
  byte), indices in bytes mod 256.
- `+0x110` palette 1024 B, **BGRA** u8 per entry (guest writes at boot),
  `+0x510` u32 DG_ScreenBuffer address (guest writes in DG_Init).
- Driver: `sendKey(code, pressed)` writes the 2 bytes then `write32(keyWr)`.
  Read-back path: `emu.read32` (`uc.mem_read`).
- **Key consumption pacing**: `I_GetEvent` (engine/i_input.c) breaks its
  drain loop on the FIRST key-UP, so the guest consumes at most one
  (down, up) pair per frame; frames run ~1 per 2.5 batches (the 15 ms
  `DG_SleepMs` = 60k nops ≈ 360k inst dominates). Sent keys therefore need
  pacing: held keys should be sent down-only (no UPs — they don't break the
  drain), momentary keys as (D,U) pairs a few batches apart.
- Keycodes are identity (`TranslateKey`); Enter=0x0D, Esc=0x1B,
  arrows 0xAC/0xAE/0xAD/0xAF, strafe 0xA0/0xA1, use 0xA2, fire 0xA3,
  F-keys 0x80..0x8B.

### Boot / menu flow (verified in this port)
- UART markers: `Z_Init` → `W_Init` → `adding doom1.wad` →
  `D_CheckNetGame` ("startskill 2" prints unconditionally, NOT in-game) →
  `HU_Init`/`ST_Init` → `I_InitGraphics` (last boot print). Boot ≈ 7-10M
  instructions.
- Title: attract loop TITLEPIC (pagetic=TICRATE*170 → demo1 → CREDIT …).
- **Menu navigation with doom1.wad = retail: New Game goes to an EPISODE
  select first** (EpiDef), then the skill menu. The skill menu selects by
  CURSOR, not number keys: Enter(menu) → Enter(New Game) → Enter(episode 1)
  → Down, Down (skill 3) → Enter(start). Full working sequence in
  site/test_doom.mjs (change-gated on `menuactive` at 0xC00166F8).
- In-game state: `gamestate` at 0xC00153AC == 0 (GS_LEVEL),
  `menuactive` at 0xC00166F8 == 0. Debug symbols (nm): gamestate c00153ac,
  menuactive c00166f8, pagetic c001439c, demosequence c00143bc,
  advancedemo c0014390, currentMenu c0016820, itemOn c0016824, MainDef
  c000bbd0, EpiDef c000bcbc, NewDef c000bca4, gameaction c001588c,
  gametic c000f350.

### Two firmware-side bugs fixed in this port (both REBUILD `make -C doom`)
1. **Garbage "fresh" SRAM**: this Unicorn WASM build's `mem_write` copies
   data through a stackAlloc buffer; a later `mem_map` can reuse those
   stack-touched heap pages, so newly-mapped guest RAM was seeded with
   firmware-transfer leftovers (doom.bin's 288 KB triggers it). **Fix in
   site/emulator.js: zero SRAM right after the RAM mem_map** (applies to
   every firmware, not just doom).
2. **SHA-1 TCI wedge**: the engine's `W_Checksum` SHA-1 `Transform`
   (engine/sha1.c) aborts the TCI interpreter (`tcg_qemu_tb_exec_arm`,
   tci.c:1272). Patched `W_Checksum` (engine/w_checksum.c) to
   `memset(digest, 0, …)` — only used for net-game connect, harmless.
3. **_sbrk / zone overlap crash**: `_sbrk` used `__heap_start`
   (0xC0100000) which equals DOOM_ZONE_ADDR — malloc'd framebuffer
   overlapped the zone, corrupting Z_* headers and `lumphash` chains
   (crash in `W_CheckNumForName` → strncasecmp). platform.c `_sbrk` now
   starts at DOOM_HEAP_ADDR (0xC0700000).

### Verification
- `node site/test_doom.mjs` — boots, drives the full menu, holds W + turns,
  asserts: boot markers, palette populated, fb at 0xC0700008, changes ≥ 20,
  `menuActive === 0`, `phase === 'play'`, keyRd > 0. 80M-inst cap, 200k
  batches, ~45 s wall, exit 0.
- `site/doom.html` + `site/doom.js` — browser demo (serve site/, open
  /doom.html): canvas 640×400 (2× 320×200, BGRA→RGBA per frame), WASD +
  arrows + Ctrl/Space/Shift + F-keys, held keys re-asserted down-only each
  rAF step, click = fire with pointer lock. Links from index.html footer.
- **Browser pacing (2026-08-13)**: 1 `emu.step()` per rAF was ~13 tics/s
  (each game frame ≈ 450k inst: tick ≈90k + DG_SleepMs(15) ≈360k). doom.js
  now runs up to `STEP_BUDGET = 6` steps per rAF within `MS_BUDGET = 16` ms
  wall → near-realtime (~16.5 MIPS in headless Chrome). `emu.step()`
  returns `{pc, stopped, instCount}` where **instCount is CUMULATIVE
  session-wide** (module counter in emulator.js HOOK_CODE) — assign
  `instTotal = res.instCount`, NEVER `+=` (summing yields fake
  MIPS 33419/41568 readings).
- **Fast path (2026-08-13)**: emulator.js gained `minimalPolls: true` +
  `blockCounting: true` (createEmulator opts, doom.js + test_doom pass both).
  `minimalPolls` skips the per-instruction tick_n/watchdog/flash/dma/eth wasm
  calls entirely; `blockCounting` replaces HOOK_CODE with HOOK_BLOCK
  (instCount += size/2 per block — exact for Thumb-2 2/4-byte widths, valid
  ONLY with minimalPolls since it has no tick/poll logic). Measured on Node:
  stock 7.6 MIPS → minimalPolls 8.6 → blockCounting 12.3 (+63%). The
  instCount meter in block mode over-reports ~1.39× the emu_start budget
  (the guest's straight-line 60k-nop delay loop is one giant TB that
  overshoots the per-step count; the MIPS meter reads ~1.4× true). Do NOT
  use blockCounting for ETH/DMA firmware (no rx-poll stop → recv-wait
  wedges, the §7 class of stall).
- **Stats meter**: `#stats` line shows `MIPS: x.x · FPS: n · x.xM inst`,
  updated every 500 ms. FPS counts **framebuffer changes/sec** (fnv1a of
  the 64KB fb, change-gated re-render) — reads ~0 on static views (title,
  menus, pushing into the spawn wall) by design, real numbers when the
  view moves. Smoke asserts FPS ≥ 5 while holding W + ArrowLeft.
- **Canvas fills the viewport via `fitCanvas()` (JS)**: CSS-only sizing
  failed twice (`width:min(100vw,calc(100vh*1.6))` resolved to ~931px in
  a 980px viewport; `max-width/max-height:100%` caps but never grows, so
  the canvas stayed at intrinsic 640×400). fitCanvas now **stretches the
  canvas edge-to-edge** to the `#screenWrap` rect (explicit px
  `canvas.style.width/height` — the user asked for full-fill, accepting
  the aspect distortion instead of centered 16:10 bars); called at boot
  start, after `createEmulator`, and on window resize. Headless quirk:
  dpr=1.25.
- CDP smoke: `/tmp/opencode/doom_smoke2.mjs` — boots in headless Chrome
  (fresh `--user-data-dir` per run; cache-bust via `?v=N` — the doom.js
  module is cached), dispatches the menu sequence via synthetic
  KeyboardEvents, asserts gamestate=0/menu closed, canvas fills the
  viewport (w ≥ 600, aspect ≈1.6), stats `/MIPS: \d/` with FPS ≥ 5 during
  W+turn, and ≥5000 non-black canvas pixels (measured 63080). Uses a
  python http.server on 8123.
- `doom1.wad` (shareware, 4.2 MB) is copied into `site/` for the browser
  page; the node harness reads `/tmp/opencode/wad/doom1.wad`.
- `tools/make_firmware.mjs` has the `doom` entry; `site/firmware.js`
  regenerated (31 firmwares, doom.bin 277 KB).
