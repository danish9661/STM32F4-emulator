# AGENTS.md

## 0. Working agreement (added 2026-09-06, at user request)

- **Never end a turn on a bare launch.** Every background job must be an
  end-to-end detached chain (`setsid nohup ... &` + log file) that runs to
  a verdict file without intermediate turns: build → run → analyze →
  branch, all inside one script.
- **Stay on-turn with chained blocking waits** (`sleep ~110` + check,
  repeat in the same turn, inside the ~120 s tool timeout) instead of
  ending the turn while a result is minutes away.
- **Scratch lives in `.pw-scratch/`** (repo-local; /tmp is quota-starved).
  Delete PNGs/scratch that is no longer needed; keep logs + scripts.
- **Do not touch the user's background processes** (esp32 emu etc.) and
  never `pkill` without asking first (a stray pkill pattern once killed
  the tool call itself).
- **One refresh per shipped fix only**: bump `?v=` on every edited
  served file (doom.js, worker, app.js, firmware.js) so a single
  hard-refresh always reaches the current build; record the number in
  `__doomVer`.

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
- **FreeRTOS interrupt pump: task-context `portYIELD()` mid-`str` PC.** In
  `site/emulator.js` `processInterrupts`, a task that yields via a `PENDSVSET`
  write to `0xE000ED04` is stopped by `memWriteHook` **mid-`str`**, so
  `uc.reg_read_i32(PC)` is the **store's own address**, not the next
  instruction. The exception frame must save the **following** instruction's
  address (advanced by `thumbInstLen`), matching real Cortex-M (exceptions are
  taken *after* the instruction completes). Saving the frozen PC makes the
  resumed task re-execute the `str PENDSVSET`, re-pend PendSV, and — for the
  highest-priority task — deadlock the whole scheduler (`TASK1`/`TASK2`/SysTick
  never run). The `icsrYieldPc`/`deliveringIsr` plumbing in
  `processInterrupts`/`memWriteHook` enforces this; never "simplify" it back.
  Regression test: `site/probe_freertos.mjs` (wired into `npm test`); the
  firmware is `freertos_test/` (TIM3 ISR → `xSemaphoreGiveFromISR(xTimSem)` →
  `vHighTask` pends on the semaphore). `vHighTask` **arms TIM3 itself** (it owns
  the peripheral that gates its wakeup, so it does not depend on another task
  arming the timer). The probe is intentionally quiet: it prints only a final
  summary plus `PROBE PASS`/`PROBE FAIL`, and stops early once every success
  marker is observed — all earlier `[DIAG]`/`[MODELDUMP]`/`[syms]`/`[parse]`/
  `[N]` debug output was removed.
- **The "huge MMIO hook range breaks TIM3" diagnosis was wrong.** TIM3 (and
  other APB1 peripherals at 0x4000xxxx) ARE correctly hooked by the single
  `[0x40000000,0xB0000000]`+`[0xE0000000,0xE1000000]` range used for BOTH
  `mmap` and `hook_add` in `site/emulator.js`. The FreeRTOS scheduler deadlock
  was purely the context-switch PC bug above, not the hook range. Keep the
  single huge range — a prior split into per-region `periphHookRanges`/
  `periphMapRanges` dropped EXTI/SYSCFG/DBGMCU and regressed `test_exti`/
  `test_dma`/`test_rx_interrupt`.
- **Deeper FreeRTOS coverage is NOT needed (decision 2026-08-22).**
  `probe_freertos.mjs` already exercises all three context-switch entry
  points the interrupt pump handles: **task-context yield** (`vHighTask`'s
  `xSemaphoreTake` block → resume), **ISR-context yield** (`TIM3_IRQHandler`
  `xSemaphoreGiveFromISR` + `portYIELD_FROM_ISR`), and **SysTick-context
  yield** (`vTaskDelay` expiry unblocks TASK1/TASK2 via the SysTick ISR's
  own PendSV path) — plus **preemption** (HIGH/TASK1/TASK2/IDLE all
  observed) and a **binary-semaphore give-from-ISR**. That guards the
  specific emulator defect (mid-`str` exception-return PC). Deeper
  primitives (inter-task queues, mutex/priority-inheritance, task deletion,
  a second concurrent ISR) would mostly test *guest* FreeRTOS library code
  on the emulated CPU, not new emulator behavior, and add firmware/
  maintenance cost for marginal protection. Revisit only if `processInterrupts`
  is reworked or a real FreeRTOS app using those primitives is targeted.
  Recorded in `docs/progress-and-future.md` (FreeRTOS to-do).
- After a wedged emu_start, the instance is unusable; re-create the uc and
  re-init rather than trying to recover.
- The step throughput degrades sharply over long runs (translation cache
  growth); prefer short runs and checkpointing to one long run.
- **2026-08-15: sequential `createEmulator()` instances in one process now
  work** (this reverses the 2026-08-14 "one firmware per process" rule).
  Two causes, both fixed: (1) `SYS` in `lib.rs` was a `OnceLock`, so every
  `init()` after the first was silently dropped and instance 2 ran on
  instance 1's peripheral tree — it is now an `AtomicPtr` to a leaked
  `Box`; (2) `system.rs` globals accumulated across instances —
  `createEmulator()` now calls `reset_state()` before registering devices.
  Regression test: `site/test_multi_instance.mjs` (three orderings of
  blinky/rtc/buzzer plus a 7-instance run), in `npm test`.
  Two things to keep in mind when touching this:
  - Call **exactly one** of `init_svd()` / `init()` per instance. Both
    install a system and the last wins, so `init()` after `init_svd()`
    silently replaces the SVD map with the hardcoded one. Under the old
    `OnceLock` that was a harmless no-op, and `emulator.js` did exactly
    that — which is why the first attempt at this fix looked like it
    regressed `test_exti`/`test_audio`. Those two remain the canaries.
  - Instances are still not safe **concurrently**: one active system per
    process, and a new `init` detaches the old. Close before re-creating.

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
Also a fixed **Peripherals** panel (`#periph`, `refreshPeriph`) showing
ETH DMASR/MACCR, USART1 SR/BRR, RCC AHB1ENR, GPIOA ODR, and a **Memory
Watch** panel (`#watchList`): type any hex address (+ optional label),
Add, and it shows a live 32-bit readout each rAF; a per-row poke input
writes a hex value via `emu.write32`. `refreshWatch()` runs in the rAF loop
next to `refreshPeriph`. Verified by headless-Chrome CDP smoke
(`/tmp/opencode/watch_smoke.mjs`): boots `?fw=blinky`, adds a watch on
GPIOA ODR (0x40020014), observes it toggle `0x20`↔`0x00` as PA5 blinks.

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
- **Key consumption pacing (input model, rewritten 2026-08-13)**: `I_GetEvent`
  (engine/i_input.c) breaks its drain loop on the FIRST key-UP, so the guest
  consumes at most one (down, up) pair per drain (~1 per 2 frames — the 15 ms
  `DG_SleepMs` = 60k nops ≈ 360k inst dominates). doom.js therefore gates the
  keyUP on **guest consumption**: `sendKey` returns the ring byte position of
  the DOWN; a `sentPos` Map (code->D position) + `upPending` Map remember
  released keys; every rAF `flushUpPending()` writes the UP only when the
  guest's ring cursor `keyRd` has advanced past that D (reads `KEYRD =
  ABI+0x04`). Consequences: (1) each tap delivers exactly one (D,U) pair the
  guest can drain atomically; (2) a held key's UP is deferred until the guest
  consumed the D, so holds turn continuously; (3) NO per-rAF re-assert of held
  keys — the old re-assert loop spammed ~7 downs per tap (menu cursor
  jumped several items per tap, and a held Enter auto-started games). The
  ring therefore always contains strictly ordered D...U pairs, and with
  `keyRd` gating, an UP can never be written before its DOWN.
- Keycodes are identity (`TranslateKey`); Enter=0x0D, Esc=0x1B,
  arrows 0xAC/0xAE/0xAD/0xAF, strafe 0xA0/0xA1, use 0xA2, fire 0xA3.
  **F-keys are `KEY_F1..F12 = 0x80+0x3B..0x80+0x46 = 0xBB..0xC6`** (engine
  doomkeys.h; the old 0x80..0x8B mapping was wrong — browser F2/F3/F6/F9
  did nothing). doom.js: F1=0xBB, F2=0xBC, F3=0xBD, F6=0xC0, F9=0xC3,
  F10=0xC4, F11=0xC5, F12=0xC6. Menu keys: save=F2, quick-save=F6,
  load=F3, quick-load=F9, confirm='y' (0x79), forward=Enter, activate=Esc.
  doom.js keydown/keyup pass **raw ASCII `a`..`z`** (0x61..0x7A) through
  when the key is not in DOM_TO_DOOM — required for the save-menu name
  entry ('a' is strafe!) and the 'y' confirm prompts.

### Save/load (2026-08-14) — EXTRAM staging + localStorage mirror
- **Save area**: `DOOM_SAVE_ADDR 0xC0080000` (EXTRAM), 2 slots ×
  `0x40000` (256 KB — matches vanilla `SAVEGAMESIZE` cap checked by
  `ftell` in G_DoSaveGame; slot size == cap so a buffer-overrun save
  can't overflow into the zone at 0xC0100000; the 0xC0080000→0xC0100000
  gap is exactly 512 KB).
- **Firmware shims** (doom/f407/platform.c): savegame files are
  `*/.savegame/doomsavN.dsg` (N = slot digit) → `_open` routes fd 0x7f00
  reads/writes to the EXTRAM area. Commit = newlib `rename` (disassembled
  `_rename_r` @0x0801c330 = `_link_r` + `_unlink_r`) → `_link(old,new)`
  when the OLD name parses as a savegame: sets `saveFlag=1`, `saveSlot`,
  `saveSize`, and prints `SAVE ok slot=%d bytes=%d` (rename's return is
  UNCHECKED by the engine — the flag IS the success signal). Load
  (`_open` read path): returns -1 immediately if the driver's `saveMap`
  bit for the slot is clear (M_ReadSaveStrings fopen()s ALL 6 slots at
  boot-menu time — no saveMap → NULL → EMPTYSTRING), else sets
  `saveFlag=2`+`saveSlot` and busy-waits on `saveReady`, then prints
  `LOAD ok slot=%d bytes=%d`.
- **ABI additions** (after clockMs @+0x518): saveFlag +0x51C (1=save
  written, 2=load request), saveSize +0x520, saveReady +0x524
  (driver→guest), saveSlot +0x528, saveMap +0x52C (bit N = slot N has a
  save). `DOOM_SAVE_FD 0x7f00`; `is_savegame_name` (".dsg"),
  `save_slot_of` (parses "doomsav"+digit; temp.dsg/recovery.dsg → slot 0).
- **`_write` gotcha**: the save stream is written through the SAME
  `_write`; the fd==DOOM_SAVE_FD branch must come FIRST — a missing
  branch routes the 25 KB save archive to the UART (looks like raw
  mobj/level data garbage on the terminal). `_read`/`_write`/`_lseek`/
  `_close`/`_link`/`_unlink` all honor the fd branch.
- **doom.js** `processSaves()` (runs every step + in the pump): flag=1 →
  read EXTRAM blob, chunked `btoa` into `localStorage['doom-save-N']`,
  OR `saveMap` bit, clear flag; flag=2 → `atob` → mem_write, set
  saveSize or 0, `saveReady=1`. boot() restores saveMap from
  localStorage. (Node harness has no processSaves — it asserts the ABI.)
- **Node harness flow** (site/test_doom.mjs): F6 (0xC0) at i≥200 →
  Enter when menuactive → 'a' (0x61) + Enter → asserts `SAVE ok slot=0`
  in uart + flag=1. Pass condition: `SAVE ok slot=0` present.
- **Key-ring overflow (harness bug)**: the guest consumes ~1 (D,U) pair
  per game frame; spamming W(D) every 200k-inst iteration (plus arrow/
  fire taps) wraps the 256-byte ring mod 256 and clobbers unconsumed
  events — the F6 was consumed but the queued Enter/'a'/Enter were
  overwritten (keyRd froze, menu=1 cm=SaveDef, sse=0, qss=-2 forever).
  Fix: hold W = re-assert D every 25 iters, arrow+fire taps every 40.
  Browser doom.js is unaffected (held keys = one D, U on release).
- **Nightmare confirm**: the skill menu opens at item 2; two ArrowDowns
  land on item 4 (NIGHTMARE) → the game pops a "ARE YOU SURE?" prompt
  needing 'y' (0x79) — the save smoke handles it (waits for
  messageToPrint @0xC001682C != 0 → taps 'y'). One ArrowDown = skill 3,
  no prompt.
- **Attract note (smoke2)**: demo1 playback runs with gamestate=0
  (GS_LEVEL) and demoseq @0xC00143BC ≥ 1 — `gamestate==3` only appears
  during the ~170s title pic, so smoke waits on `gs==3 || demoseq>=1`.
- **Hidden-tab test is env-flaky** (~50%): a TCI wedge can strike
  anywhere in the first seconds of E1M1 play (main thread blocks 10s+;
  Runtime.evaluate never returns). Reproduced with the OLD firmware +
  new doom.js and vice versa — NOT a code regression, it is the §7-class
  Chrome-only TCI issue (batch/region sensitive; the firmware layout
  shift from any rebuild moves the trigger). doom.js has a `lastPump`
  guard so the rAF loop never steps back-to-back with the pump's burst
  (gap-free melt-crossing is the documented wedge trigger). If a smoke
  fails with `evaljs timeout`, kill all chrome (`pkill -9 chrome`) —
  stale instances hold CDP 9334 and the smoke drives a wedged page.

### Boot / menu flow (verified in this port)
- UART markers: `Z_Init` → `W_Init` → `adding doom1.wad` →
  `D_CheckNetGame` ("startskill 2" prints unconditionally, NOT in-game) →
  `HU_Init`/`ST_Init` → `I_InitGraphics` (last boot print). Boot ≈ 7-10M
  instructions.
- Title: attract loop TITLEPIC (pagetic=TICRATE*170 → demo1 → CREDIT …).
- **Menu navigation with doom1.wad = retail: New Game goes to an EPISODE
  select first** (EpiDef), then the skill menu. The skill menu selects by
  CURSOR, not number keys: Enter(menu) → Enter(New Game) → Enter(episode 1)
  → Down (skill 3) → Enter(start). NOTE: the skill menu **opens at item 2**
  (`NewDef.lastOn = hurtme`, m_menu.c:323) — ONE ArrowDown lands on skill 3;
  a second Down lands on item 4 (NIGHTMARE), which pops the "ARE YOU
  SURE?" confirm prompt needing 'y' (0x79). Full working sequence in
  site/test_doom.mjs (change-gated on `menuactive` at 0xC00166F8).
- In-game state: `gamestate` at 0xC00153AC == 0 (GS_LEVEL),
  `menuactive` at 0xC00166F8 == 0. **In-game turning is verified by reading
  `players[0].mo->angle`: mo = read32(0xC00153B4), angle = read32(mo+32) —
  the Strife-fork mobj_t has extra `snext`/`sprev` fields so `angle` is at
  offset 32, NOT 24** (p_mobj.h:201; thinker 12 + x,y,z 12 + snext 4 +
  sprev 4). Dumping `mo` via a double-deref (`read32(read32(0xC00153B4))`)
  lands on `thinkercap` (0xC0018488), a linked-list sentinel — always read
  the mobj AT the players[0].mo value.
  Debug symbols (nm): gamestate c00153ac,
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
  /doom.html): canvas backing store 320×200 = the native framebuffer
  (BGRA→RGBA per frame; CSS scales it up, see the quarter-size bug below),
  WASD +
  arrows + Ctrl/Space/Shift + F-keys, **held keys are held (D once, U gated
  on guest consumption — no re-assert spam)**, click = fire with pointer
  lock. Links from index.html footer.
- **Browser pacing (2026-08-13)**: 1 `emu.step()` per rAF was ~13 tics/s
  (each game frame ≈ 450k inst: tick ≈90k + DG_SleepMs(15) ≈360k). doom.js
  runs up to `STEP_BUDGET = 12` steps per rAF within `MS_BUDGET = 16` ms
  wall, paced by the realtime lock (§ Realtime lock below — guest clock
  derived from FRAMECOUNT, 60000-inst steps) → measured **~24 MIPS / ~24 FPS**
  in headless Chrome (blockCounting mode, smoke run: 216M inst, 135.4° turn
  in a 4s W+ArrowLeft hold).  `emu.step()` returns `{pc, stopped,
  instCount}` where **instCount is CUMULATIVE session-wide** (module counter
  in emulator.js HOOK_CODE) — assign `instTotal = res.instCount`, NEVER `+=`
  (summing yields fake MIPS 33419/41568 readings).
- **Melt-wipe stall fix (2026-08-13) — the smoke's "menu never opens / W key
  did not move the player" root cause**: the level/title transitions run the
  melt wipe, and its driver loop (d_main.c D_Display, ~line 318) is
  `do { do { nowtime = I_GetTime(); tics = nowtime - wipestart; I_Sleep(1); }
  while (tics <= 0); wipestart = nowtime; done = wipe_ScreenWipe(wipe_Melt,
  ..., tics); } while (!done);` — it can advance only ONE melt iteration per
  clock advance, and the melt needs ~40 iterations (~15 count-up + 25 scroll
  dy=8 rows at height 200). The page wrote the wall clock to the ABI CLOCKMS
  slot **once per rAF**, and headless Chrome rAF throttles to ~2 Hz, so each
  transition froze the guest for **~20 s** (diag4 caught PC spinning in
  `wipe_doMelt`'s inner column loop 0x0800373A-0x08003756 with FRAMECOUNT
  frozen at 32 while CLOCKMS advanced). Worse, the old guest-side
  `DG_SleepMs` did an **absolute** `g_abi.clockMs = s_msClock` write that
  raced the page's absolute write — the clock flapped between tiny guest
  values (5, 16, 47…) and page values (~3000+), so `tics` went negative and
  the wait loop could spin past its budget. Fix, both sides:
  1. **site/doom.js**: `emu.write32(CLOCKMS, performance.now())` now runs
     before **every** `emu.step()` in the pacing loop (was: once per rAF).
  2. **doom/f407/platform.c**: `DG_SleepMs` now advances the ABI clock
     **relatively** (`g_abi.clockMs += ms`) instead of an absolute write —
     monotonic, never flaps the driver's value, and still self-advances the
     clock in the Node harness (which has no page writer). With both, the
     wipe's wait loop resolves in one iteration and each melt runs `tics`
     iterations per call — a transition melt completes in ~1-2 steps.
  Verified: browser smoke PASS (W-move 442.8 units / turn 126.9° / audio
  42525 samples / 24.0 MIPS / 27 FPS), `node site/test_doom.mjs` PASS (74
  fb changes, audio peak 1). Diagnosis path: /tmp/opencode/doom_diag4.mjs
  (PC probe via `e.uc.reg_read_i32(11)` — **ARM_REG_PC=11, NOT 15** (15 is
  D1); reg_read_i32(15) silently returns 0).

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
  use the `blockCounting` flag for ETH/DMA firmware (that hook is
  count-only — no rx-poll stop → recv-wait wedges, the §7 class of stall).
- **Per-BLOCK hook is now the DEFAULT for all firmware (2026-08-14).**
  The restriction above was a property of the count-only `blockHook`, not
  of HOOK_BLOCK itself. `blockHookFull` does everything `codeHook` did —
  rx-poll stop, `tick_n`, watchdog, flash erase, DMA/TX poll — just once
  per basic block, scaled by `size/2`. Why it is safe: the tick/poll
  thresholds are 5000/1000 instructions, so accumulating per ~10-instruction
  block trips them within one block of where per-instruction counting would.
  Escape hatch: `perInstHook: true` restores the old HOOK_CODE path.
  * **Rationale, measured**: a hook whose body returns IMMEDIATELY is just
    as slow as one doing the full accounting (15.09 vs 15.14 MIPS) — the
    WASM→JS **crossing itself** is the cost, not the work, so the fix is to
    cross less often. Removing the hook entirely gives 17.70, i.e. ~16% is
    on the table and the per-block hook captures most of it.
  * **The CPU hook, not the two-WASM-module boundary — but that depends on
    the firmware, so measure before concluding.** The Unicorn↔Rust
    peripheral path (`periph_read`/`periph_write`) fires ~42 times per 3M
    instructions on blinky (0.001%) — but **2.25M times on an eth_http soak,
    3.75% of instructions, ~642 per HTTP round**. So for compute-bound
    firmware (doom, blinky) the second WASM module is nowhere near the hot
    path, while for I/O-bound firmware it carries real traffic. An earlier
    version of this note generalised the blinky figure and claimed the
    boundary never matters; that was wrong.
  * **`memReadHook` reuses scratch buffers** instead of allocating a
    `Uint8Array` per MMIO read (safe: `uc.mem_write` copies synchronously).
    Worth it on I/O firmware — eth_http avoids ~2.25M allocations per 60M
    inst. NOTE: an earlier claim of "3-5% on blinky" for this change was
    measurement noise and should not be repeated — blinky issues only 42
    MMIO reads per 3M instructions, so it cannot have moved the needle
    there. The change is sound but its magnitude is unquantified.
  * **Gain, honestly**: +10% on blinky. On a 200M-inst eth_http netsim soak
    the MIPS meter shows 11.30 → 17.03, but that is INFLATED by block
    over-counting (~1.32× here). Measured by work actually completed it is
    **869 → 999 HTTP rounds/sec, ≈ +15%**. Always compare block-mode
    against per-inst mode by rounds/sec or wall time, never by the MIPS
    readout.
  * **Do NOT chase the last ~6% by removing the hook (measured trap).**
    Hookless runs at 17.70 MIPS vs the per-block hook's 16.61, so it looks
    like free headroom — but for IO firmware the hook's **rx-poll stop is
    what lets the driver service RX promptly**. Without it, RX is only
    serviced between `emu_start` batches and the guest burns its budget
    spinning in recv-wait. Measured on the eth_http soak, by HTTP
    rounds/sec (60M inst, equal length):
    | mode | MIPS | rounds/sec |
    |---|---|---|
    | per-block (default) | 16.01 | **947** |
    | per-inst (old) | 10.47 | 809 |
    | hookless, batch 5000 | 16.54 | 476 |
    | hookless, batch 20000 | **20.11** | **143** |
    The hookless/batch-20000 config posts the HIGHEST MIPS and does **7x
    less actual work**. This is the clearest example of why MIPS is the
    wrong metric for this emulator — always measure rounds/sec or wall
    time for a fixed workload.
  * **Soak-validated** (the check the old warning demanded): 200M inst,
    **11692 rounds, 0 `TCP fail`, 0 stalls**, plus npm test 7/7, all eight
    peripheral tests, test_doom, and 5 deterministic test_flow runs.
    Harness: `scratchpad/eth_soak.mjs` (`PER_INST=1` for the baseline) —
    note `cli.mjs` installs its OWN HOOK_CODE and does not exercise
    emulator.js, so it can neither validate nor benefit from this.
- **Stats meter**: `#stats` line shows `MIPS: x.x · FPS: n · x.xM inst`,
  updated every 500 ms. FPS counts **framebuffer changes/sec** (fnv1a of
  the 64KB fb, change-gated re-render) — reads ~0 on static views (title,
  menus, pushing into the spawn wall) by design, real numbers when the
  view moves. Smoke asserts FPS ≥ 5 while holding W + ArrowLeft.
- **Quarter-size render bug (fixed 2026-08-14) — the game drew into the
  top-left quarter of the canvas.** `renderFb` blits with
  `ctx.putImageData(img, 0, 0)`, which writes pixels **1:1 and ignores all
  scaling** (CSS size and transforms alike). The canvas backing store was
  640×400 while the DOOM framebuffer (and `img`) is 320×200, so three
  quarters of the buffer stayed black and CSS then scaled that mostly-empty
  buffer to fit — a 1280×860 window showed an ~422×264 game floating in an
  844×528 element. **Fix**: canvas `width`/`height` attributes are now
  320×200 (native), CSS does the upscale, `image-rendering: pixelated`
  keeps it crisp — 4× the visible area, and a smaller buffer to boot.
  Diagnosis note: `fitCanvas()` was NOT at fault (measured wrap 1280×528
  and canvas CSS 844×528 — both correct); the giveaway was that the visible
  game was exactly half the canvas in each dimension. If you ever change
  `DOOMGENERIC_RESX/RESY`, the canvas attributes must follow.
- **Canvas sizing via `fitCanvas()` (JS, contain-fit since 2026-08-13)**: the
  canvas is sized to the **largest 16:10 box inside `#screenWrap`**
  (`scale = min(wrap.w/640, wrap.h/400)` — 640/400 here is just the 16:10
  ASPECT reference for the CSS box, independent of the 320×200 backing
  store above; using 320/200 would give the identical result) and centered by
  the wrap's flexbox — aspect is always exactly 1.6 (verified: 1467×917 at
  1920×1080 and 884×553 at 1000×900, equal side gaps in both), with black
  bars only where the window isn't 16:10. History: CSS-only sizing failed
  twice (`width:min(100vw,calc(100vh*1.6))` resolved to ~931px in a 980px
  viewport; `max-width/max-height:100%` + `width:auto` never grows past
  intrinsic 640×400); the intermediate JS version stretched edge-to-edge
  (distorted aspect + looked right-aligned against the black wrap). Called
  at boot start, after `createEmulator`, and on window resize. Headless
  quirk: dpr=1.25.
- **Page layout (2026-08-13, game-first)**: doom.html is now a full-window
  gaming layout: one compact `#topbar` (title + status + stats + held-keys +
  buttons), `#screenWrap` with `flex:1` (the game fills ALL remaining
  vertical space), an always-visible **`#guide`** controls strip below the
  screen (Move W/S/A/D + arrows · Strafe Shift · Fire Ctrl · Use Space ·
  Menu Enter/Esc · Save F2/F6 · Load F3/F9 · F1/F10/F11/F12), then a small
  `#uart` terminal. The old tall header/help/button stack wasted ~half the
  window. `<meta http-equiv="Cache-Control" content="no-cache…">` +
  `?v=N` cache-bust on the script tags — a stale cached doom.html/doom.js
  is a known cause of "wrong aspect / not covering / no guide" reports
  after an update (hard-refresh Ctrl+Shift+R if the page looks old).
- CDP smoke: `/tmp/opencode/doom_smoke2.mjs` — boots in headless Chrome
  (fresh `--user-data-dir` per run; cache-bust via `?v=N` — the doom.js
  module is cached), drives the menu with **REAL CDP `Input.dispatchKeyEvent`
  keys** (synthetic `window.dispatchEvent` KeyboardEvents do NOT reach
  doom.js listeners — the old smoke was a false positive), waiting on each
  intermediate menu (`currentMenu` == EpiDef/NewDef pointers at 0xC0016820),
  asserts the skill menu opens at item 2 (NewDef.lastOn=hurtme), in-game
  (gamestate=0/menu closed), **player angle at mo+32 actually rotates
  ≥ 10° during a 4s W+ArrowLeft hold (measured 110.7°)**, canvas fills the
  viewport (w ≥ 600), stats `/MIPS: \d/` with FPS ≥ 5 during W+turn
  (measured 24.4 MIPS / 16 FPS), and ≥5000 non-black canvas pixels
  (measured 63719). Uses a python http.server on 8123.
- `doom1.wad` (shareware, 4.2 MB) is copied into `site/` for the browser
  page; the node harness reads `/tmp/opencode/wad/doom1.wad`.
- `tools/make_firmware.mjs` has the `doom` entry; `site/firmware.js`
  regenerated (31 firmwares, doom.bin 277 KB).

### Speed + audio corrections (2026-08-14) — MEASURED, supersedes claims below

Investigated a "speed issue and audio" report. Three things below this line
turned out to be wrong; all numbers here are measured, not estimated
(diagnostics drove boot→menu→E1M1 and sampled guest globals directly).

- **The low-detail toggle was a NO-OP.** `site/doom.js` wrote the engine's
  `detailLevel` global (0xC0016814) directly. `detailshift` — and the
  `colfunc`/`spanfunc` pointers that actually make low detail cheaper — are
  recomputed ONLY inside `R_ExecuteSetViewSize()`, which runs only when
  `R_SetViewSize()` has set `setsizeneeded`. Measured: writing `detailLevel`
  left inst/frame bit-identical (1145k both ways) with `detailshift` stuck
  at 0, i.e. the page shipped a default-ON speed control that did nothing.
  **Fix**: new ABI slot `DETAIL_ADDR` (+0x530); `apply_detail_request()` in
  platform.c (called from `DG_DrawFrame`, a safe end-of-frame boundary) sets
  `detailLevel` and calls `R_SetViewSize()` **and** `R_ExecuteSetViewSize()`.
  Probed: `setsizeneeded` is never consumed by `D_Display` in this build
  (stays 1 across frames while rendering continues, with `screenvisible=1`
  and `nodrawers=0`), so the guest must execute it itself.
  Result: 1145k → **918k inst/frame (-20%)**, 19.2 → **24.0 fps (+25%)**.
- **"Any machine above ~3.5 MIPS holds exactly 35 fps" (old doom.js header)
  was wrong by ~10x.** At ~918k inst/frame (low detail) 35 fps needs
  **~32 MIPS**; high detail needs ~40 MIPS, and moving+turning peaks near
  1.7M inst/frame (~60 MIPS). The core delivers **~20-23 MIPS**, so the page
  runs ~22-24 fps. Verified NOT fixable by driver tuning: step size is flat
  from 60k to 600k inst (16.6-17.0 MIPS), removing the per-block counting
  hook buys only ~6% (`noCountHook` option added to emulator.js), and
  rebuilding the firmware `-O2` instead of `-Os` changed inst/frame by <1%
  (reverted; it only cost 28 KB). The ceiling is the Unicorn WASM core.
- **The mixer clipped ~45% of all nonzero samples.** The scale constant has
  been wrong in BOTH directions: the original 16129 was blamed for "weak and
  muffled" audio, but the real cause was the per-frame `/ active` divisor
  (8 live channels = exactly the "~12% of full scale" that was reported);
  "fixing" it by swapping 16129 → 1905 (assuming vol maxed at 15) then
  over-amplified by **8.47x**. `I_StartSound` receives the engine's INTERNAL
  volume, "ranging from 0-127" (engine/s_sound.c:92) — NOT the 0..15 menu
  setting — so the true per-channel max is 127*127 = 16129.
  **Fix**: fixed scale `(32768<<8)/16129`, no `/active` divisor, sum+clamp
  like a normal mixer. Measured over the same E1M1 run:

  | | before | after |
  |---|---|---|
  | clipped samples | 1396 (**45% of nonzero**) | **0** |
  | distinct 16-bit levels | 61 | **204** |
  | crest factor | 3.39 (squashed) | **8.10** |
  | peak | 1.0000 (pinned) | 0.508 |

  `site/test_doom.mjs` printed `peak amplitude ${audioPeak.toFixed(0)}` on a
  ±1.0 float — so full-scale clipping rendered as the reassuring "peak
  amplitude 1" and near-silence would print "0". It now reports peak to 3dp
  plus nonzero/clipped counts and **asserts `clipPct < 5`**; verified the
  guard fails (24.6%) against the pre-fix firmware.
- **THE audio bug (fixed 2026-08-14): the worklet's resample ratio was
  INVERTED.** `site/audio-worklet.js` had `this.ratio = sampleRate / 11025`
  (≈4.35 at 48 kHz) and advanced the 11025 Hz read cursor by that per OUTPUT
  sample. The correct step is SRC/DST — `11025 / sampleRate` ≈ 0.23. The
  inverted form consumed input **~19x too fast**, so every sound played far
  above audible pitch and the queue starved instantly: the user-visible
  symptom was "no correct sound, only crackling", and it had been that way
  since the worklet landed. It survived because the tests only counted
  samples the guest PRODUCED — nothing ever checked that playback consumed
  them at the right rate, so a completely broken resampler passed.
  Measured on the real page (8 s of gameplay): **starved output samples
  364476 → 0**; cumulative starvation 91276 → ~3k (boot only).
- **Rate control replaced silence+flush.** The old underrun path emitted
  silence AND did `this.q = new Float32Array(0)`, discarding audio that had
  already arrived — which is why the deficit became *pure* crackle rather
  than occasional gaps. The worklet now runs a proportional controller on
  buffer depth (TARGET_Q) that nudges playback rate to match production and
  never flushes; starvation holds the last sample and decays it (soft fade,
  no click). Gotchas learned tuning it:
  * **RATIO_MIN must sit below the slowest rate the guest can force.**
    Required rate == fps/35, so 24 fps needs 0.69 but a dip to 14 fps needs
    0.40 — a 0.5 floor guaranteed dropouts on every dip. Floor is 0.28.
  * **Drain audio on the 40 ms pump tick even while rAF is alive.** The rAF
    loop drains once per frame with irregular cadence, so samples reached
    the worklet in clumps and the buffer dipped empty between them
    (7.5% → 5.5% starvation from this alone).
  * **Bump the `?v=` on `addModule('audio-worklet.js?v=N')` on every edit** —
    the module is cached hard and a stale worklet is indistinguishable from
    an audio bug.
  Consequence, by design: when the guest runs below 35 fps the stream plays
  slow/pitched-down (rate ~0.72 at 25 fps) but CONTINUOUS — which matches
  the visibly slow-motion game. The stats line now shows it (`audio 0.72x`).
- **`noCountHook` adopted for doom (2026-08-14)**: no per-block JS callback
  at all. Measured on the page: FPS 22 → **25/35**, audio production +16%
  (rate 0.62 → 0.72). The MIPS readout *drops* (24.5 → 19.0) because block
  counting over-reported ~1.39x; the new number is the honest one.
- **Old note, kept because the framing is still right — the residual
  slowness is the fps shortfall, not a separate audio bug.** `DOOM_SubmitAudio` emits one frame's worth
  (11025/35 = 315 samples) per RENDERED frame. The driver derives the guest
  clock FROM the frame counter (`clockMs = bootClock + frames*FRAME_MS`,
  doom.js:186/499), so guest time advances exactly one frame-period per
  frame and those 315 samples are **already exactly realtime in game
  time**. The game just runs at ~63% of wall speed, so it produces ~63% of
  the audio wall time consumes (measured: 27720 samples in 5 s of play at
  ~22 fps), and `site/audio-worklet.js` converts each shortfall into an
  audible gap — on underrun it emits silence AND flushes the queue
  (`this.q = new Float32Array(0); this.pos = 0`).
  **Correction to an earlier note in this file: "mix by elapsed guest time"
  is NOT a fix** — guest time is frame-derived, so it yields the identical
  315/frame. Mixing by WALL time is worse: it advances sfx sample positions
  faster than the game logic, pitch-shifting/garbling the sounds. Real
  options: (a) raise fps (the only true fix, blocked by the ~23 MIPS core
  ceiling); (b) make the worklet stretch/resample on underrun instead of
  inserting silence — continuous but slightly slowed audio, matching a game
  in slow motion; (c) accept and document (current state).
- Browser-verified on an isolated headless Chrome (own `--user-data-dir`
  and a unique CDP port — **note: the default 9334 may already be held by a
  real Chrome session on this machine; always pick a fresh port**):
  `MIPS: 23.4 · FPS: 22`, detail toggle active, 64000 non-black pixels.

### Audio (I2S mixer) + WASD (2026-08-13, doom.bin now ~284 KB)
- **Sound path**: `doom/f407/i_sound_f407.c` (was fully stubbed) now has an
  8-channel 8-bit→16-bit mixer. Sfx lumps are resolved lazily via the
  canonical `I_GetSfxLumpNum` (`"ds"+name` → `W_GetNumForName`; the engine
  calls it when `lumpnum < 0` — returning `sfx->lumpnum` as the stub did
  leaves it -1 forever and `W_LumpLength(-1)` aborts). Samples cached in
  `sfxinfo->driver_data` (points past the 8-byte header). Sound lump format
  verified empirically from doom1.wad: `[u16 3][u16 11025][u16 length][u16 0]`
  then `length` 8-bit unsigned samples (NO ascii name header).
- **Mixer** (`DOOM_SubmitAudio()`, declared in doomplatform.h, called from
  `DG_DrawFrame` — one 315-sample frame = 11025/35): sums `sample*vol` per
  active channel and normalizes `* 32768 / (1905 * active)` so full scale
  needs every ACTIVE channel at max vol — **1905 = 127×15 (max |sample-128|
  × max engine vol 0..15); the old `16129 = 127×127` constant assumed vol
  127 and left even an all-channels-maxed mix at ~12% of full scale
  (permanently weak/muffled audio, user reported)**. Naive `sample*vol*2` +
  ±30000 clamp hard-clipped every loud sample (earlier "crackling" report).
  I2S1 init in `I_InitSound`: RCC_APB2ENR bit 12, CR1=0, I2SPR=(2)|(1<<8),
  I2SCFGR=(1<<11)|(1<<10)|(1<<9)|(1<<0); per-sample `while(!(SR & TXE))` +
  DR write (audio_play_test pattern).
- **Playback (doom.js, AudioWorklet since 2026-08-13)**: `ext_devices:
  { speaker: true }` (the emulator's speaker drain is opt-in,
  `takeSpeakerSamples()` else returns empty); `AudioContext` created lazily
  on first keydown (autoplay policy, async `initAudio` awaits
  `addModule('audio-worklet.js?v=19')`). `drainAudio` posts each drain as a
  **transferred Float32Array** to the worklet node (`port.postMessage(s,
  [s.buffer])` — `takeSpeakerSamples` returns a fresh array, safe to
  transfer). The worklet (`site/audio-worklet.js`, `DoomAudio`) resamples
  11025 Hz → context rate (linear interp) on the audio thread — no
  BufferSource scheduling jitter, no crackles; bounded queue MAX_QUEUE=4096
  (~0.37 s) DROPS OLDEST samples when production runs ahead (the one-time
  boot catch-up), so latency never exceeds ~0.4 s; underrun (slow machine)
  emits silence and resets `pos=0` so the next samples play immediately.
  Counter `window.__audioTotal` for smoke assertions. Historical (pre-
  worklet): BufferSource per 1024 samples with `playbackRate` adaption +
  `MAX_AHEAD` drop guard — replaced because chunk scheduling glitched at
  high rates and adaptive rate desynced production.
- **Realtime lock (2026-08-13)**: the guest clock is DERIVED FROM ITS OWN
  FRAMECOUNT — `emu.write32(CLOCKMS, floor(bootClock + frames * FRAME_MS))`
  before EVERY `emu.step()` (`bootClock` = wall time at boot, `FRAMECOUNT`
  read each step). Guest time can never run ahead of the guest's own
  execution, so the melt wipe (and every `I_GetTime`-based wait) resolves
  in one step and audio production stays locked to wall 11025 samples/s.
  Pacing: `paceFrames=0/paceWall=bootClock` anchored AT BOOT (was: anchored
  at first rendered frame with `target=Infinity` — that let boot run ahead
  and created a one-time catch-up burst); `target = paceFrames + floor(now
  - paceWall)/FRAME_MS` per rAF; `STEP_INST` reduced 300000 → 60000
  (~0.66 frames/step) so pacing overshoot ≤1 frame. DOOM needs only ~5
  MIPS for 35 fps, so the smaller steps cost nothing measurable (smoke
   measured 24.0 MIPS). The realtime lock + worklet combined make browser
   audio play in lockstep with the game (rv32emu-demo parity).
- **Background-tab audio pump (2026-08-13, the "running in background, sound
  is broken" fix)**: rAF stops entirely in a hidden tab, so the realtime lock
  starves production and the worklet queue underruns in ~0.37 s. doom.js adds
  `pumpAudio()`: `setInterval(pumpAudio, 40)` + worklet hunger messages
  (`postMessage('need')` when `q.length < MAX_QUEUE`, throttled 0.02 s)
  drive 12-step bursts of `emu.step(20000)` while `performance.now() -
  lastRAF > 200` (rAF alive = the loop drives; the pump never double-drives).
  The clock write is **monotonic** (`clockMs = Math.max(clockMs, bootClock +
  frames*FRAME_MS)`) — never write a lower value, or the melt wipe's
  `while (tics<=0)` wait (and the engine's tic pacing) sees negative deltas
  and spins: the melt's wait loop is a thin loop whose sustained spin is the
  §7-class TCI wedge. **Chrome TCI wedge finding (headless Chrome only,
  never Node 22.22): a melt-wipe crossing stepped GAP-FREE (back-to-back
  `emu.step` calls, no rAF/interval gaps) wedges the Unicorn WASM instance
  permanently; the SAME stepping at the rAF cadence (12-step bursts ~40 ms
  apart) is clean over 200M+ inst. So the pump uses 20k-inst steps (the §7
  safe budget) in 12-step bursts paced by a 40 ms interval — the rAF
  cadence. Verified: hidden-tab test freezes rAF 4 s → audio keeps flowing
  (+15750 samples ≈ 54 % of the visible rate) → rAF restored → full rate;
  PASS. Debug handles: `window.__audioNode`, `window.__noDrain`,
  `window.__noUart`, `window.__audioTotal`, `window.__doom`.
  **2026-08-15: emulation moved into a Worker** (`site/doom-worker.js`);
  `site/doom.js` is now only the UI shell, and the audio pump / rAF
  double-driving described above is GONE — the worker steps on its own
  timer, so `window.__pump` / `window.__audioPumpTimer` / `window.__emu`
  no longer exist on that page (use `window.__doom`). See §17. ABI gotcha: FRAMECOUNT/CLOCKMS are at **0x20002514 /
  0x20002518** (ABI base 0x20002000 + 0x514/0x518) — reading 0x20000514
  returns a dead SRAM location (frames always 0 → clock pinned → guest
  time-waits never resolve). Known limits: Chrome throttles hidden-tab
  intervals to ~1 s for audible tabs, so fully-hidden production drops to
  ~1 burst/s — partial background audio is browser-enforced.
- **WASD works now**: the engine's default bindings (m_controls.c) are
  arrow-only (key_up=0xAD, key_down=0xAF, key_left=0xAC, key_right=0xAE,
  strafe 0xA0/0xA1, use 0xA2, fire 0xA3); `DOM_TO_DOOM` in doom.js maps
  'w'/'s'/'a'/'d' → those arrow codes (menus get WASD navigation for free).
  No firmware rebuild needed for this.
- **extra_ram zeroing** (emulator.js): the §11/§16 "zero SRAM" fix only
  covered 0x20000000 — DOOM's .data/.bss live in the 16MB extra_ram at
  0xC0000000, which was seeded with firmware-transfer leftovers (browser
  showed garbage `currentMenu`/`itemOn` — menu flow broke; Node harness
  passed by luck of the leftover pattern). Each extra_ram region is now
  zeroed right after mmap, same as SRAM.
- **mobj x/y offsets**: mobj_t (Strife-fork) = thinker(12) + x,y,z (x@+12,
  y@+16, z@+20) + snext/sprev (24/28) + angle@+32. Reading mo+4/mo+8 reads
  the thinker's function pointer (0xC017xxxx — looks like a huge negative
  fixed-point x).
- **Smoke** (/tmp/opencode/doom_smoke2.mjs, v=16) now also asserts: index.html
  header DOOM button + footer link, WASD forward motion ≥ 5 units, audio
  samples played (fire with Ctrl first — menu blips alone are ~±150 at vol 64).
  Measured run: 83.1 u forward, 93.2° turn, 45675 samples, 22.6 MIPS / 15 FPS.
- `node site/test_doom.mjs` asserts audio: drains `takeSpeakerSamples()`
  incrementally (the ring only keeps the last 64 chunks — a single end-of-run
  drain misses the gunshots) and requires peak amplitude > 0.005. The Node
  harness sends raw 0x77 for W (unbound in-engine) — only the browser
  translates; the harness tests the engine path with arrow taps instead.

---

## 17. DOOM Web Worker (2026-08-15)

Emulation for `doom.html` runs in `site/doom-worker.js`; `site/doom.js` is
the UI shell. The worker owns the emulator, the stepping loop, the guest ABI
and the framebuffer→RGBA conversion. The page keeps only what a worker
cannot touch — canvas, keyboard, `AudioContext`/worklet, `localStorage` —
and does one `putImageData` per frame on a transferred buffer.

**It buys responsiveness at no throughput cost** — but only because the
burst cadence is driven by the PAGE's rAF: `doom.js` posts a `tick` every
animation frame and the worker runs one burst per tick. Its rAF callback
only posts a message, so the main thread stays free while the guest keeps
the old build's 16ms-per-16.7ms duty.

Interleaved A/B, 45s headless runs (machine had other load — the ratio is
the result, not the absolute numbers):

| build | guest inst | rAF-gap p95 |
|---|---|---|
| main-thread (pre-worker) | 770–779M | 33.4ms |
| worker, self-timed `MS_BUDGET` 28 | 668–681M | 16.8ms |
| worker, self-timed `MS_BUDGET` 44 | 720–724M | 16.8ms |
| worker, page-tick driven (shipped) | 762–780M | 16.8ms |

Things that bite here:

- **No `SharedArrayBuffer`.** It needs COOP/COEP headers GitHub Pages cannot
  set, and it would buy nothing: the two WASM modules have separate linear
  memories regardless, and Unicorn's build is single-threaded (§7).
- **Never SKIP a queued tick.** A "minimum gap between bursts" guard that
  dropped ticks arriving while a burst was still running cost a whole frame
  each time and measured 413M vs 720M guest inst. The gap between bursts is
  the message dispatch plus the rest of the frame — the same ~0.7ms the old
  rAF loop ran with, and a real event-loop turn is all the TCI needs.
- **The self-timed fallback is for when rAF is dead** (hidden tab). The
  worker watches for ticks going stale (200ms) and then drives itself on a
  `setTimeout`, with a bigger burst budget because `setTimeout` is clamped
  to ~4ms once nested and that clamp is a pure duty-cycle tax (self-timed
  at 28ms burst = 12% below baseline, at 44ms = ~6%). Verified with
  `window.__doom.stopTicks()`: +131.5M inst over 8s with no ticks, and
  resuming them does not wedge it.
- **A `MessageChannel` yield is NOT a valid way to reclaim duty cycle**
  (~99%, no clamp): over a 60s run it degraded to 760M inst at MIPS 1.1 —
  the same class of stall §7/§16 describe. Do not "optimize" it back.
- **Unicorn cannot be `import`ed into the worker.** It is a classic
  emscripten script that assigns a global, so the worker fetches and
  indirect-`eval`s it, and must pass an explicit `locateFile`: there is no
  `document.currentScript` for emscripten to derive the `.wasm` path from,
  and it would otherwise look for it next to the worker instead of in
  `vendor/`.
- **Savegames are a round-trip.** Blobs live in guest EXTRAM but
  `localStorage` is on the page, so a load request stops the stepping loop
  (`loadPending`) until the answer arrives — the guest busy-waits on
  `SAVEREADY`, so there is nothing to run meanwhile.
- **Two cache-busting `?v=` values now.** `doom.js?v=` in `doom.html`, and
  `doom-worker.js?v=` inside `doom.js`. Worker scripts cache exactly as hard
  as module scripts. Bump the right one.
- `window.__emu` / `window.__pump` / `window.__audioPumpTimer` are gone from
  this page — the emulator is not on that thread. `window.__doom` exposes
  `booted`, `paused`, `stats`, `key(code, pressed)`, `send(msg)` and the
  `stopTicks()`/`startTicks()` test hooks.

---

## 18. Low-power model + demo firmwares (2026-08-22)

### WFI/STOP low-power path (emulator-level)
- **Model**: `stm32-periph-wasm/src/peripherals/pwr.rs` gained `wakeup()`
  (sets `CSR` bit 2 = WUF); `Peripherals::pwr_wakeup()` (mod.rs) downcasts the
  `0x4000_7000` slot and calls it; `lib.rs` exports `pwr_wakeup()` (wasm).
- **RTC wakeup**: `rtc.rs` RTC alarm sets NVIC pending IRQ **41** (F407
  `RTC_Alarm`; the model previously hardcoded 43 — wrong). The demo clears
  RTC `ISR` bit 0 to "start" the counter (model gate in `advance_time`).
- **emulator.js** (`lowpower` opt, default false):
  - Forces `HOOK_CODE` (codeHook) — `blockCounting`/`minimalPolls` must NOT
    take precedence or the WFI trap never installs. Hook-dispatch order is
    `noCountHook` → `minimalPolls||lowpower` → `blockCounting` → `perInstHook||
    freertos` → `blockHookFull`.
  - codeHook detects `WFI`/`WFE` (16-bit `0xBF30`/`0xBF20`, 32-bit
    `0xF3BF 0x8F4F`/`0x2F5F`) with `primask==0` and no pending interrupt →
    sets `sleeping = (SCR>>2)&1 ? 2 : 1` (STOP vs SLEEP) and `emu_stop()`.
  - `step()` top: if `sleeping`, `tick_n(WAKE_STEP=120000)`,
    `periph_read(RTC_BASE=0x40002800,4)` (advances the virtual RTC → fires
    alarm → NVIC pending), then if `has_pending_interrupt()` → on STOP call
    `pwr_wakeup()` (sets WUF), clear `sleeping`, and fall through to run the
    guest (the WFI becomes a no-op); else `instCount += WAKE_STEP; return`
    (stay asleep). `has_pending_interrupt()` is the **non-consuming** check
    from `lib.rs`:89 (use it for wakeup; `get_next_pending_interrupt()`
    consumes and must NOT be used here).
- **Browser**: `app.js` `LOWPOWER_FIRMWARES = new Set(['deep_sleep_demo'])`
  passes `lowpower: true` to `createEmulator`.

### Demo firmwares (added 2026-08-22)
- `deep_sleep_demo/` — arms RTC alarm ~3s ahead, enters STOP via WFI, the
  emulator halts and advances the virtual RTC until the alarm wakes it; prints
  `WOKE FROM STOP` + `Wakeup flag (WUF) set`. Node test: `site/test_lowpower.mjs`.
- `can_demo/` — friendly two-phase CAN showcase (single-node loopback +
  two-node arbitration, lower ID wins, broadcast to both). Node test:
  `site/test_candemo.mjs` (complements `can_test`/`test_can.mjs` which check
  the raw model mechanics).
- Both added to `tools/make_firmware.mjs` → `site/firmware.js` (35 firmwares),
  so they appear in the browser dropdown.

### Edge-case tests (`site/test_edge_cases.mjs`, item 7)
High-value emulator-edge coverage (not guest-library re-tests): bad-image
rejection via `loaders.js` (non-HEX, HEX checksum, truncated ELF, ELF with no
PT_LOAD), invalid-MMIO (unmapped `0x10000000` read/write rejected), reset/
reload (fresh instance boots after a prior one), and multi-instance stress
(five sequential `createEmulator` instances all boot — validates
`reset_state`). Wired into `npm test`.

### Gotchas
- The hook that detects WFI is `codeHook`; `blockCounting` (default true)
  would install `blockHook` instead and the WFI trap would never run — so
  `lowpower` is checked BEFORE `blockCounting` in the hook-dispatch chain.
- RTC time only advances when `ISR` bit 0 is clear (a model gate); the
  deep-sleep demo clears it (`RTC_ISR &= ~1u`) to "start" the counter.
- Writing a multi-instance test: always `close()` each emulator (Unicorn is
  1.75 GB/instance) and run `step()` inside the drain loop — a loop that only
  calls `drainUart()` produces empty UART (the guest never executes).

### CAN host-injection API (2026-08-23)
- **Rust**: `can.rs::can_inject(sys, id, dlc, data, ext, rtr)` delivers a frame
  from an external transmitter onto the shared bus — broadcast to every CAN
  node (CAN1/CAN2) whose accept filters pass it, via `Can::receive_frame`.
  Exported to wasm as `can_inject(id: u32, dlc: u32, data: &[u8])` (standard
  11-bit frames; `ext`/`rtr` hardcoded false in the binding).
- **emulator.js**: `emu.canInject(id, dlc, data)` → `can_inject(id & 0x7FF,
  dlc & 0xF, new Uint8Array(data))`. Mirrors `injectFrame` (ETH) as the CAN
  equivalent for host-driven RX demos/tests.
- **Firmware**: `can_host_rx/` — CAN1 with pass-all filter, polls RX FIFO,
  prints `RX id=0xNNN data=...` for each injected frame. Node test:
  `site/test_can_inject.mjs` (asserts `RX id=0x00000123 data=HELLO!!!`). Added
  to `tools/make_firmware.mjs` → `site/firmware.js` (36 firmwares).

### CLI `--lowpower` flag (2026-08-23)
- `cli.mjs` (the `stm32f4-emu` bin) delegates to `createEmulator`, which already
  implements the low-power halt/wake path (`lowpower` opt). Added `-l` /
  `--lowpower` parsing that passes `lowpower: true` through, so firmware that
  enters STOP/SLEEP via WFI runs end-to-end from the CLI:
  `stm32f4-emu deep_sleep_demo.bin --lowpower`. Each sleeping `step()` advances
  the virtual RTC by `WAKE_STEP=120000` instructions (consuming budget) until
  the RTC alarm fires and `pwr_wakeup()` clears the halt. Without the flag the
  firmware still wakes (the CLI's codeHook calls `tick_n` which advances the
  RTC), but it spins the full instruction budget instead of halting.

- **Browser UI**: `site/index.html` has a "CAN injection" sidebar panel (ID
  hex + up to 8 hex data bytes + "Inject CAN frame" / "Inject HELLO!!!"
  buttons); `site/app.js` wires them to `emu.canInject` (validates 11-bit ID
  and hex bytes, shows status in `#canStatus`). `can_host_rx` and `can_demo`
  were added to the Firmware dropdown so the feature is demoable in the page.
  Verified by headless-Chrome CDP smoke (`/tmp/opencode/canui_smoke.mjs`):
  boots `?fw=can_host_rx`, clicks Inject, asserts `RX id=0x00000123
  data=HELLO!!!`.

### Watchdog (IWDG/WWDG) — 2026-08-23
- **Model**: `iwdg.rs` + `wwdg.rs` were already registered in the peripheral
  map (0x40003000 / 0x40002C00). Previously they only *requested* a reset and
  the JS driver merely stopped — the guest never actually rebooted. Fixed:
  - The countdown now runs **continuously off the virtual (instruction-count)
    clock** via `Peripheral::tick` (driven by `tick_n`), not only on register
    access — so the counter reaches 0 even while the firmware is idle.
  - On expiry `request_watchdog_reset(cause)` latches a per-source reset-cause
    bit (`IWDG_RESET_FLAG` / `WWDG_RESET_FLAG`) and the JS driver **reboots the
    guest to the reset vector** (SP/PC from 0x08000000/0x08000004) instead of
    stopping, so the firmware's startup re-runs. The model also disables the
    watchdog on fire so the reboot doesn't immediately re-trigger.
  - `rcc.rs` now reflects the cause in `RCC->CSR`: `IWDGRSTF` (bit 29) /
    `WWDGRSTF` (bit 30), cleared on the firmware's `RMVF` (bit 24) write.
  - New exports `iwdg_reset_flag()`, `wwdg_reset_flag()`,
    `clear_watchdog_reset_flags()` (cause-aware `request_watchdog_reset(u8)`).
- **Firmware `watchdog_demo/`**: starts IWDG (prescaler /8, max reload ≈1 s),
  pets it through an "alive" loop, then stops petting so it expires → MCU
  resets → reboots → reads `RCC->CSR` and prints `IWDG reset detected`.
  Note: STM32F4 `RCC_CSR` is at **offset 0x74** (0x40023874), not 0x24.
- **Firmware `wwdg_demo/`**: starts the window watchdog with prescaler /8 and
  window `W=0x7F` (no early-window restriction, so it behaves as a plain
  down-counter), pets it every 2 ms through an "alive" loop, then stops
  petting → counter underflows → MCU resets → reboots → prints
  `WWDG reset detected` (reads `RCC->CSR` `WWDGRSTF` bit 30). The WWDG model
  supports the full window semantics (a `W < 0x7F` rejects refreshes while
  counter > W) plus the early-wakeup interrupt (CFR bit 9 / SR bit 0); the
  demo exercises the reset path.
- **Firmware `wwdg_window_demo/`**: sets a *real* window `W=0x50` (early-window
  restriction active). The alive loop polls the counter and only refreshes
  when it has dropped into the legal window (`counter <= W`), proving correct
  refreshes survive; then it deliberately refreshes while the counter is still
  above `W`, triggering a **window violation** reset. On reboot prints
  `WWDG reset detected`. Exposes both previously-untested model paths.
- **Tests**: `site/test_watchdog.mjs` (IWDG, **WDOG PASS**),
  `site/test_wwdg.mjs` (WWDG underflow, **WDOG-WW PASS**), and
  `site/test_wwdg_window.mjs` (WWDG window violation, **WDOG-WW-WIN PASS**),
  all wired into `npm test`. The Wwdg Rust module also has two unit tests:
  `window_violation_reset` and `ewi_fires_at_window` (EWI fires at the window
  edge + NVIC IRQ 0 pending). `cargo test` 31/31.

### TIM input capture (2026-08-23)
- The TIM model already implemented **output compare / PWM** (up/down/center
  counting, CC match + PWM-duty tracking, UIF/CCxIF interrupts) — so the
  `buzzer_test` JS probe was redundant, not a missing model. The genuinely
  missing path was **input capture**: a TIx edge latching the live counter
  into `CCR[ch]`.
- **Model**: `tim.rs` now decodes `CCxS` (CCMR1/CCMR2) to tell input from
  output capture; `capture_trigger(ch)` latches `CNT` into `CCR[ch]`, sets
  `CCxIF`, and sets `CCxOF` on an overrun before the first is serviced. Output
  channels no longer latch on a capture trigger. A host-injection entry point
  `tim_inject_capture(name, ch)` (mirrors `can_inject`) simulates a TIx edge
  since the emulator has no external signal source; exported to wasm and
  `emu.timInjectCapture(name, ch)` in `emulator.js`.
- **Firmware `tim_capture_demo/`**: configures TIM3 CH1 as input capture
  (CCMR1 CC1S=0b01, CCER CC1E), then prints `cap=N` for each injected edge.
  `site/test_tim_capture.mjs` injects edges and asserts the captured counter
  values (**TIMCAP PASS**). Three Rust unit tests in `tim.rs` cover
  latch/overrun/output-ignore.
- Note: ADC/DAC/SDIO/FSMC are **already fully modeled** (conversion, noise/
  triangle waveforms, FSMC bank read/write, SDIO command state) — they were
  not stubs. **QSPI** was the last gap: it is now modeled in `qspi.rs`
  (QUADSPI register set at `0xA000_1000`; functional indirect read/write to an
  optional external flash backend via `qspi_register_flash(name, data)`, plus
  `BUSY`/`TC`/`FT` flag handling; 6 Rust unit tests in `qspi.rs`). The F407
  SVD has no QSPI block, so it is registered explicitly in both `new_wasm`
  and `from_svd` rather than via the SVD map.
- **`qspi_test/` firmware** (bare-metal, same Makefile pattern as `blinky`):
  drives the QUADSPI registers directly (no F4 QSPI HAL exists) — indirect
  write then read of four 32-bit words round-tripped through a driver-
  registered flash image, printing `QSPI OK` / `QSPI FAIL`. Built with the
  Arduino-provided xpack gcc (`make -C qspi_test`, `TOOLCHAIN` env override).
  `node site/test_qspi.mjs` registers a 256-byte flash via
  `bindings.qspi_register_flash('QUADSPI', ...)` *before* `createEmulator`
  and asserts the `QSPI OK` marker (wired into `npm test`).
- **QSPI is fully wired end-to-end (2026-08-23):**
  - `stm32-periph-wasm/pkg/cli.mjs` registers the QSPI flash before
    `init_svd` in **both** positional mode (auto-enabled when the firmware
    path contains `qspi` → loads `qspi_flash.bin` if present else a default
    256-byte image) and config mode (`config.devices.qspi`), and supports a
    `qspi` device type in the config `devices` loop. Verified:
    `node cli.mjs ../../qspi_test/qspi_test.bin 2000000` and
    `node cli.mjs --config=../../qspi_test/config.yaml 2000000` both print
    `QSPI OK`.
  - **Browser demo:** `site/vendor` was rebuilt (`wasm-pack build --release
    --target web --out-dir ../site/vendor`, then `unicorn_arm.{js,cjs}` +
    `stm32f407.svd` restored and `vendor/.gitignore` removed). `qspi_test`
    is in `site/firmware.js` (regenerated via `tools/make_firmware.mjs`,
    now 41 firmwares), selectable in `site/index.html` (`?fw=qspi_test`),
    mapped in `app.js` `DEVICE_FIRMWARES` (`qspi: [{ peripheral: 'QUADSPI',
    size: 256 }]`), and `site/emulator.js` registers the flash via
    `qspi_register_flash` *before* `init_svd` (Qspi::new clones the backend
    at construction).
  - **CDP smoke promoted into `npm test`:** `site/test_qspi_cdp.mjs` boots
    `?fw=qspi_test` in headless Chrome (driven over the DevTools Protocol)
    and asserts `QSPI OK` appears on the UART. Run standalone with
    `npm run test:qspi:browser` (needs `google-chrome` or `CHROME_BIN` +
    `python3`); it is the final step of `npm test`. The page-level debugger
    socket must be used (list `/json`, connect to the `page` target), not
    the browser-level one.
  - **Reusable CDP harness + broader browser coverage:** `site/cdp_smoke.mjs`
    exports `runCdpSmoke({ fw, markers, failMarkers, timeoutMs })` (spins up
    `python3 -m http.server site/`, launches headless Chrome, connects to the
    page-level debugger socket, navigates to `?fw=<preset>`, polls `#uart`
    for any marker). `site/test_browser.mjs` runs it over **10 presets** and is
    the **last step of `npm test`** (standalone: `npm run test:browser`). The
    markers are chosen to prove the *peripheral itself ran*, not just that the
    firmware booted:
    - `blinky` → `LED=ON`, `eth_http` → `TCP connected`,
      `oled_test` → `OLED draw done`, `tft_test` → `TFT fill done`,
      `ltdc_test` → `LTDC pixels OK`,
    - `can_test` → `CAN loopback OK` (fail: `CAN Test: FAIL`),
      `watchdog_demo` → `IWDG reset detected`,
      `rtc_test` → `RTC verify OK` (fail: `RTC verify FAIL`),
      `audio_play_test` → `I2S1 TX sine 256 samples`,
      `deep_sleep_demo` → `WOKE FROM STOP`.
    `deep_sleep_demo` was also added to the `site/index.html` firmware
    dropdown (it was in `firmware.js` but missing an `<option>`, which made
    the auto-boot resolve `fwSelect.value=''` and crash). True in-browser
    regression coverage for the demo, not just the node path.
- **Bug fixed in this pass**: the window-violation check previously fired on
  the *enable* write too — it compared the stale pre-enable counter (0x7F)
  against `W` and tripped a false violation. Now gated on `initialized` so only
  subsequent refreshes are checked (the initial load is always allowed).

---

## 19. STM32F4 high-level facade (Wokwi-style API, 2026-08-27)

A typed, rp2040js/avr8js-style wrapper over the Unicorn-based emulator, so
firmware can be driven like a real chip (`gpio`, `usart`, `spi`, `i2c`,
`dma`) without touching the `createEmulator`/hook plumbing.

### Files
- `site/stm32f4.js` — `STM32F4`, `GPIOPin`, `USART`, `DMAStream` classes +
  the `parseSpi`/`parseI2c` helpers. Pure JS; zero runtime overhead (every
  call delegates to the underlying `emu`).
- `index.mjs` — re-exports `STM32F4, GPIOPin, USART, DMAStream` and binds
  `STM32F4.create` to the resolved Node assets
  (`bindings`/`unicorn`/`svdXml`/`wasmInit` from the local `vendor/`).
- `package.json` — `"./stm32f4"` export → `site/stm32f4.js`.
- Tests: `site/test_stm32f4_api.mjs` (facade + GPIO/USART) and
  `site/test_stm32f4_periph.mjs` (Wokwi SPI/I2C), both wired into `npm test`.

### Usage
```js
import { STM32F4, decodeFirmware } from 'stm32f4-emu'; // or the local path
const mcu = await STM32F4.create({
  // optional firmware (else loadBin/loadHex/loadELF before execute)
  firmware: /* Uint8Array flash */,
  spi: [{ peripheral: 'SPI2', cs: 'PB12', dc: 'PB11', onTransfer: (ch, tx, rx) => {...} }],
  i2c: [{ peripheral: 'I2C1', address: 0x3C,
          onStart: (addr, isRead) => {}, onWrite: (b) => {}, onStop: () => {} }],
});
mcu.mcu.gpio.pin('PA5').on('change', (level) => console.log('LED', level));
mcu.usart.onData = (bytes) => process.stdout.write(bytes);   // Buffer
mcu.usart1.sendData('hi');                                  // alias of usart
mcu.execute(1_000_000);                                     // run N cycles
mcu.reset(); mcu.close();
```
- Firmware loading: `loadBin(bytes, base?)`, `loadHex(text)`, `loadELF(bytes)`.
  Each writes flash (and any ELF/Intel-Hex RAM segments) and resets SP/PC from
  the loaded vector table. A non-zero `PLACEHOLDER_VECTOR` (SP=0x20000000,
  PC=0x08000185) is used when no firmware is given so `createEmulator`'s
  reset-vector check passes; the real firmware replaces it.
- `execute(cycles)` runs the engine and drains UART into `usart.onData` as a
  `Buffer` (the emulator returns a **string**, so iterate `charCodeAt`).
- `read32/write32`, `getRegisters()`, `stop()`, `reset()`, `close()` delegate.

### Wokwi-style virtual peripherals
Built on the EXISTING bus taps — **no Rust change**:
- `mcu.spi` — declared via the `spi:[{peripheral, cs?, dc?, onTransfer?, onByte?}]`
  create option. `onTransfer(ch, tx, rx)` fires at CS deassert; `onByte(ch, byte,
  pushMiso, dc)` per byte. `mcu.spi.pushMiso(peripheral, bytes)` injects MISO.
- `mcu.i2c` — `i2c:[{peripheral, address, onStart?, onWrite?, onRead?, onStop?}]`.
  `mcu.i2c.pushRx(peripheral, bytes)` pre-supplies master-read responses.
- Why declared at `create()`: the Rust Spi/I2c peripheral snapshots its
  device list once at `init_svd()` (see `emulator.js` line ~172), so the taps
  are registered *before* init via `ext_devices.spiDevices`/`i2cDevices`.

### Event-stream gotchas (the two bugs that bit us)
1. **State must persist across `parseSpi`/`parseI2c` calls.** The model drains
   the event queue once per `step`, so a single CS-active SPI transfer (or one
   I2C START..STOP) is usually split across several handler invocations. The
   transfer state therefore lives on `spec._st`, NOT in local variables —
   locals reset to zero every call and the transfer never completes.
2. **Edge encoding (Rust is authoritative; the §13 doc is WRONG):** CS/START
   edges have bit31 set and **bit30 = 1 means asserted/START**, 0 means
   deasserted/STOP. Start a transfer on bit30-set, finish on bit30-clear.
3. **I2C address byte is not in the stream.** The model pushes only START/STOP
   edges + master-*written* data bytes; the address+R/W byte is consumed
   internally and never reaches the tap. So `onStart(addr)` is called with the
   device's *configured* `address`, not a value parsed from the wire, and read
   transactions (which emit no master-written data) are served entirely from
   the `pushRx` queue. Master reads are NOT delivered as `onWrite` events.

---

## 20. WebSocket bridge (headless Node ↔ browser UI, 2026-08-31)

A binary WebSocket protocol so the browser console (`site/app.js`) can drive
the emulator running headlessly in Node (`site/ws-bridge.mjs`). The browser
becomes a thin UI; all WASM execution happens in Node. Zero impact on the
existing local WASM path — the bridge is opt-in via `?bridge=ws://…`.

### Files

| File | Purpose |
|---|---|
| `site/ws-bridge.mjs` | Node-side WebSocket server + emulator proxy. Loads firmware, exposes STEP/READ32/WRITE32/GET_REGS/RESET/STOP, pushes UART/ETH/GPIO state to browser. CLI: `--port`, `--firmware`, `--verbose`, `--lowpower`, `--inst`. |
| `site/remote-emu.js` | Browser-side adapter. Drop-in replacement for the local `emu` object — same `step()`/`drainUart()`/`read32()`/`write32()` API, all proxied over binary WebSocket. Auto-reconnect with exponential backoff, ping/pong keepalive, per-request timeout, firmware re-send on reconnect. Exports `ConnectionState` enum and `createRemoteEmulator()`. |
| `site/test_ws_bridge.mjs` | In-process smoke test (11 assertions: LOAD_IMAGE, STEP, READ32, GET_REGS, STOP round-trips). |

### Protocol (binary WebSocket, little-endian)

**Browser → Node (requests):**

| Type | Code | Payload | Response |
|---|---|---|---|
| STEP | `0x01` | `[id:u32] [max_inst:u32]` | STEP_RESP `[id] [inst_count:u32] [stopped:u8]` |
| STOP | `0x02` | (none) | (fire-and-forget) |
| RESET | `0x03` | (none) | (fire-and-forget) |
| LOAD_IMAGE | `0x04` | `[id:u32] [flash_len:u32] [flash bytes…]` | LOAD_OK `[id]` or ERROR `[id] [msg]` |
| READ32 | `0x10` | `[id:u32] [addr:u32]` | READ32_RESP `[id] [value:u32]` |
| WRITE32 | `0x11` | `[id:u32] [addr:u32] [value:u32]` | WRITE32_OK `[id]` |
| GET_REGS | `0x12` | `[id:u32]` | REGS_RESP `[id] [36 × u32: R0-R12,SP,LR,PC]` |
| ETH_RX | `0x20` | `[len:u32] [frame bytes…]` | (none) |
| CAN_RX | `0x21` | `[id:u16] [dlc:u8] [8B data]` | (none) |
| UART_TX | `0x22` | `[len:u16] [bytes…]` | (none) |
| SET_INPUT | `0x40` | `[pin_len:u8] [pin str] [level:u8]` | (none) |
| PING | `0xFE` | (none) | PONG `[0xFF]` |

**Node → Browser (pushes):**

| Type | Code | Payload |
|---|---|---|
| PUSH_UART | `0x80` | `[len:u16] [bytes…]` |
| PUSH_ETH | `0x81` | `[len:u32] [frame bytes…]` |
| PUSH_GPIO | `0x82` | `[bank:u8] [idr:u32] [odr:u32] [moder:u32]` |
| STOPPED | `0x8A` | (none — unsolicited when emu halts) |
| PONG | `0xFF` | (response to client PING) |

Request IDs are monotonically increasing u32s. The bridge matches responses
to requests by ID. PUSH_UART/PUSH_ETH/PUSH_GPIO are unsolicited (no ID).

### Usage

```bash
# 1. Start the bridge in Node (serves the emulator over WS on port 8234)
node site/ws-bridge.mjs blinky/blinky.bin
# or with npm script:
npm run bridge -- eth_http/eth_http.bin --port 8234

# 2. Open the browser console with the bridge URL param
# (local dev server: npm run serve → http://127.0.0.1:8123)
open "http://127.0.0.1:8123/?bridge=ws://127.0.0.1:8234"
```

Without `?fw=`, the page boots whatever firmware the bridge was started with.
With `?fw=blinky&bridge=ws://…`, the browser sends the firmware image over
LOAD_IMAGE and the bridge creates a fresh emulator for it.

### Integration in app.js

- `?bridge=ws://…` URL param switches to remote mode.
- `boot()` creates a `RemoteEmu` instead of local `createEmulator()`.
- `emu.loadImage(flash)` sends the firmware to Node.
- `emu.step()` returns a Promise (WS round-trip). `drainUart()` is
  synchronous (buffered from PUSH_UART messages).
- `getRegisters()`/`read32()`/`write32()` return Promises.
- `refreshGpio()`/`refreshPeriph()`/`refreshWatch()` are async.
- Local mode (no `?bridge=`) is completely unchanged.

### Robustness (reconnect, keepalive, timeout)

The `RemoteEmu` adapter (`remote-emu.js`) handles connection failures:

- **Auto-reconnect**: exponential backoff (500ms → 1s → 2s → 4s → 5s cap)
  on connection drop. The firmware image is re-sent automatically via
  `LOAD_IMAGE` after reconnecting, so the new emulator instance is ready
  without manual intervention.
- **Ping/pong keepalive**: the server sends `PING` (0xFE) every 30s; the
  client must respond with `PONG` (0xFF). If 3 pongs are missed (90s),
  the connection is considered dead and reconnect fires.
- **Per-request timeout**: every request (step, read32, getRegisters, etc.)
  has a configurable timeout (default 10s). A stuck server won't hang the
  browser.
- **Connection state**: `emu.connectionState` returns `'connecting'`,
  `'connected'`, `'reconnecting'`, or `'disconnected'`. Callbacks:
  `onDisconnect`, `onReconnect`, `onStateChanged`.
- **Disconnect indicator**: `app.js` shows "bridge disconnected —
  reconnecting…" in the status bar when the connection drops, and clears
  it on reconnect.

### Multi-firmware (hot-swap)

In bridge mode, switching firmwares does NOT tear down the WebSocket. The
"Boot preset" button handler checks `emu.connectionState === 'connected'`
and, if so, sends `LOAD_IMAGE` over the existing connection instead of
calling `boot()` (which closes and recreates the connection). The bridge
closes the old emulator, creates a new one, and the browser resumes
stepping. File uploads (`.bin`/`.hex`/`.elf`) still call `boot()`, which
works but is slower (reconnects).

### npm package compatibility

- `ws` is a runtime dependency (`dependencies` in package.json).
- `ws-bridge.mjs` and `remote-emu.js` are in the `files` array.
- Exports: `./remote` → `site/remote-emu.js`, `./ws-bridge` → `site/ws-bridge.mjs`.
- `npm pack` includes both files; `npm run bridge` starts the server.
- `npm run test:bridge` runs the in-process smoke test.

---

## 21. WASM-native Thumb-2 CPU (`cpu='wasm'` backend, 2026-09-03)

A pure-Rust Cortex-M4 interpreter (`stm32-periph-wasm/src/cpu/`) that runs
firmware WITHOUT Unicorn. **Since 2026-09-04 it is the DEFAULT backend**
(`cpu_backend` defaults to `'wasm'`; pass `'unicorn'` to opt back into the
Unicorn core — `probe_freertos.mjs` (ISR pump) and `test_doom.mjs` (TCI
guard) stay pinned there). Status: boots blinky/eth_http/eth_test/can_test/hal_test/
audio (DMA+I2S)/exti/rtc to markers, and runs eth_http DHCP→TCP→HTTP
end-to-end (2 rounds, `npm run test:wasm`). ~20x faster than Unicorn on
compute (~50 MIPS vs ~2-3). DOOM: title renders, menu → New Game → E1M1
play + quick-save + audio, `site/test_doom_wasm.mjs` PASS (see below).

### Files
- `stm32-periph-wasm/src/cpu/thumb.rs` (~1700 lines) — the decoder. Every
  encoding verified against `arm-none-eabi-as`/`objdump` output for repo
  firmware (probes in /tmp/opencode: `t32.s`, `it*.s`, `dsp.s`, `br.s`,
  `strd.s`, `bcc*.s`, `ext.s`, `ldrd.s`, `it2/it3.s`). Key rules:
  - Data-proc op nibble `X=(op1>>5)&0xF` maps `{0:AND,1:BIC,2:ORR,3:MVN,
    4:EOR,8:ADD,10:ADC,11:SBC,13:SUB,14:RSB}` uniformly across F
    (modified-imm) and EA/EB (shifted-reg); `S=op1[4]`, `Rn=op1&0xF`.
  - `ThumbExpandImm` per ARM ARM (rotation uses `'1':imm12[6:0]` by
    `imm12[11:7]`; replication cases for `[11:10]==00`).
  - IT: slot j>=2 uses `cond` iff mask bit (5-j) equals cond bit 0
    (GAS-verified: identical patterns assemble to different masks per cond).
    `n = 4 - trailing_zeros(mask)`.
  - BL op2 is `0xFxxx` (not `0xDxxx`); B.W/Bcc.W share `op2[15:14]==10`
    with `op2[12]` selecting; B.W is 25-bit `S:I1:I2:imm10:imm11` with
    J-inversion, Bcc.W is 21-bit `S:J1:J2:imm6:imm11` with J direct.
  - 16-bit LDR/STR-reg class is `op[11:9]` (3 bits), not `op[15:12]`.
  - F8 T2 (`c>=8`) is always imm12; T3 (`c<8`) is imm8-P/U/W except c=4/5
    with `op2[11]==0` which is register-offset.
  - LDRD/STRD select is `(op1&0x0E40)==0x0840` (bit6 set) vs LDM/STM
    `0x0800`; `Rt=op2[15:12]`, `Rt2=op2[11:8]` (GAS-verified, was swapped).
  - FA shifts: value=Rn(op1), amount=Rm(op2)&0xFF (was swapped).
  - CBZ offset is `imm6*2 + op[9]*64`, base pc+4 (was `op[2]`).
  - 16-bit B/B.cond/CBZ use pc+4 base (was pc+2).
  - F3 must precede the F-bucket with exact gates (else `ubfx` decodes as
    RSB, `bgt.w`/`MSR` collide); F3AF needs `op2==0x8000`.
  - EA/EB op1 `0xEB00` bit11=1 (an `& 0x0800 == 0` guard silently dropped
    all of EB — removed).
  - SMLAxy/SMULxy/SMLAD/SMULW (FB op 1/2/3) per GAS layouts.
- `stm32-periph-wasm/src/cpu/mem.rs` — `FlatMemory` (flash + SRAM + auto-
  extending extra regions for EXTRAM/WAD) + `load()` that writes through
  flash protection (the old `write8` path silently dropped firmware!).
  Peripheral accesses are single width-correct model calls (a byte-split
  RMW emitted 4 UART chars per store — observed).
- `stm32-periph-wasm/src/cpu/mod.rs` — `Cpu` + `CpuFault` (loud halts with
  pc/op, never silent wrongness) + `deliver_irqs` gate (default false, so
  polling firmware runs full budgets like the Unicorn path).
- `stm32-periph-wasm/src/cpu/tests.rs` — native boot tests (blinky;
  eth_http DHCP incl. Offer/Ack replay from `site/testdata_offer.bin` /
  `testdata_ack.bin`, captured via `site/save_rx.mjs`). `BOOT_LOCK`
  serializes on the shared SYS global. `cargo test` 40/40.
- `site/emulator.js` — `cpu_backend: 'wasm'` branch (before Unicorn
  creation, so no Unicorn dependency at all): byte-correct `uc` shim,
  `wProcessEth`/`wProcessDma` mirrors, `injectFrame`/`sendUart`/`pin`/
  `takeSpeakerSamples`/`faultInfo`/`reset`, watchdog reboot. Warns (not
  throws) for `enable_irqs`/`irq_eth`/`freertos`/`lowpower`, which need
  exception delivery (not implemented — SVC/BKPT/RFE record faults).
- Tests: `site/test_wasm_cpu.mjs` (blinky), `site/test_flow_wasm.mjs`
  (netsim ETH flow), `site/test_audio_wasm.mjs` (DMA+I2S),
  `site/test_wasm_multi.mjs` (7 firmware smoke). `npm run test:wasm`.
- `package.json`: `test:wasm` script. `test:wasm_multi` hal_test passes on
  wasm (and now unicorn too — earlier unicorn stall was stale-env).

### Bugs fixed along the way (all verified by boot/packet traces)
BL `0xFxxx`, SP-sub mask, hi-reg selector bits[9:8], CBZ imm, B pc+4 base,
F3 shadowing, E9 entry mask, UXTB mask, F8 T2/imm12, EA/EB `0x0800` guard,
FA value/amount swap, 16-bit reg-group class, STRD Rt/Rt2 swap, LDRD gate,
CBZ `op[9]`, Bcc.W no-inversion+imm6, REV entry mask, SMLABB, LDRH-as-STRH,
register-shift-by-0 is a no-op (not imm-#0-means-32).

### Exceptions + FreeRTOS + WFI on wasm (2026-09-04, branch `feature/wasm-cpu`)
- Inline delivery (`deliver_irqs`, set via `enable_irqs`/`irq_eth`/`freertos`/
  `lowpower`): SVC/PendSV/SysTick entry (exact stacking, handler on MSP) +
  `exception_return` (F9/FD; F1 faults) with PSP/MSP banking, CONTROL.SPSEL
  coherence (return + MSR swap), post-frame PSP advance, even-stacked-PC
  tolerance (FreeRTOS stores task entries BIC'd), per-instruction bank sync.
- Native tests: `exception_svc_roundtrip`, `freertos_tasks_run` (SVC start,
  PendSV switches, TIM2 ISR sem, TASK1/2 ticks); `cargo test` 55/55.
- `site/probe_freertos_wasm.mjs`: PROBE PASS (4 TCBs; 37k batches + 3×500-inst
  mini-samples — HIGH/TASK2 slices are ~100s of inst and fall between sparse
  samples).
- WFI/WFE sleeps with delivery on (RTC alarm wakes via `wake()` + `pwr_wakeup`);
  `site/test_lowpower_wasm.mjs` (deep_sleep_demo: WOKE FROM STOP, WUF) in
  `test:wasm`. No ISR pump needed (stacking exact, no mid-`str` hazard).

### DOOM on wasm: title + menu->play (2026-09-04)
- Decoder fixes (each silent-wrong, found by GAS + native ring traces):
  T3 register-offset for all classes + `o2[11:10]==00` gate (strh-reg did
  imm8 post-indexed writeback, ate collump); SDIV/SMLAL F:F select (divides
  returned dividend; viewheight 1680); ADDW/SUBW plain imm12 (AND/fault);
  USAT + SSAT(N-1 encoding) with Q; TBB/TBH index-by-value + unmasked base
  (misaligned TBB read shifted table: wrong demo, no title); F9 LDRSB/H-reg
  (P_LoadVertexes stuck); predicated T1 MOVS/ADD-reg/SUB-reg preserve flags
  via `it_pred` snapshot (it_ok resets it_n before handlers read it; without
  this the title never advances and E1M1 music dies) — matches Unicorn/GCC/
  vanilla (raw-Unicorn probe: movlt preserves N, strlt takes).
- 16-bit MOVS-imm C-preserve (`0x30000000` mask typo cleared C);
  16-bit ADD/SUB-reg DO set flags unpredicated (GAS `adds`/`subs`; reverting
  broke strcasecmp/title), preserve only when predicated.
- `site/test_doom_wasm.mjs` (port of test_doom.mjs): PASS (menu, 72 frames,
  SAVE ok slot 0, peak 0.5, 0% clip) in `test:wasm`. Speaker shim converts
  signed (was unsigned: 57% clipped, peak 2.0).
- Native units: tbb/sdiv(+IT taken+skipped)/usat-ssat-Q/addw-subw/t3-reg/
  ldrsh-reg/cmp13/strcasecmp-pairs/it_pred/bare-flag counterparts;
  `doom_title_renders` (TITLEPIC, 60k px, no fault/bad-mem).
- Gotcha: objdump strips one leading zero (`8002004` = 0x08002004, NOT
  0x08020004) — double-check 7-digit addresses against `nm` before trusting
  breakpoints/pools. And `doom.bin` file offset == LMA-0x08000000 (flat).

### Open / not supported on the wasm backend
- Device parsers (OLED/TFT/buzzer/RTC/DCMI JS-side) now run on BOTH backends
  (shared device layer in `site/emulator.js`, driven per step; verified by
  `npm run test:browser` 10/10 on the wasm default).
- `test_fsmc.mjs` used to fail on the unicorn path (pre-existing); it passes
  on the wasm default (the register-shift-by-0 fix cured it) and the full
  `npm test` is green end-to-end.

## 22. Headed-browser sweep + DOOM fixes (2026-09-04)

41/41 console presets PASS live in headed Chrome (Playwright, fullscreen,
fresh profile each launch). Harness: `/tmp/opencode/pw_sweep1.py` (21) +
`pw_sweep2.py` (23). Relaunch the browser every 4 presets — wasm heaps
accumulate across same-page boots (§11) and the renderer dies ~boot 6.

- **eth_dhcp / eth_test need `irq_eth`** (app.js `IRQ_ETH_FIRMWARES`): their
  `static` ETH globals sit at different SRAM addresses than eth_http's
  hardcoded `E` layout in emulator.js, so the polling driver's SRAM
  flag writes miss and TX stalls (`ARP TX timeout`). Both firmwares enable
  NVIC ETH IRQ 61 + DMAIER and own an `ETH_IRQHandler`, so IRQ delivery
  (wasm `deliver_irqs`) drives them with no layout dependency.
- **netsim answers ARP for ANY target IP** (was: only `SERVER_IP`;
  eth_dhcp ARPs 10.0.2.2 → infinite ARP loop). Claims the requested IP
  with SERVER_MAC.
- **`injectRxIrq` (emulator.js, both backends)**: irq_eth RX walks the
  guest's RX list from the model's poll address (`eth_get_rx_desc_addr` =
  DMARDLAR base) for the first DMA-owned descriptor and delivers there —
  the guest ISR scans the list itself. The old code wrote every frame to
  eth_http's `rxDesc`/`rxBuf` even in irq_eth mode, so DHCP Offers landed
  in the wrong RAM and `eth_recv_packet` never saw them. Polling path
  unchanged (static layout + idx rotation = the §16 anti-clobber fix).
- **Low-detail toggle REMOVED** (doom.html/js/worker; `?v=` bumped):
  always high detail — at ~65 MIPS the core holds 35/35 high. Booting low
  and flipping to high live left the 3D view ~208px wide + black right bar
  (the guest's mid-game `R_SetViewSize` re-tune doesn't fully take); that
  path is now unreachable from the UI (worker `detail` message + firmware
  ABI slot kept for protocol compat). The "band through menu text" seen in
  screenshots is the authentic menu-over-attract-demo (not a defect).
- **Clock AFTER step (worker)**: the page used to write the absolute clock
  BEFORE each step while the guest's `DG_SleepMs` does `+=ms` during it —
  each frame advanced game time by FRAME_MS *plus* ~15ms (~1.5x speed).
  Overwriting after the step keeps exactly one frame-period per rendered
  frame. Proven: `gametic +349/10s` wall. Node harness unaffected (no page
  writer; guest `+=` self-drives).
- **Backlog-drop (worker v12→v13)**: the wall lock never *dropped* backlog,
  so every stall (loaded box, devtools, GC) was followed by a STEP_BUDGET-
  limited sprint (this theory died below — the meter was lying; kept as
  history). Fix: backlog older than ~2
  frames is dropped (slow-mo under load, never fast-forward). Threshold 2,
  not 1, so normal rAF phase jitter (≤1 frame) still smooths instead of
  stuttering. `__doomVer` 37.
- **37 was meter noise (worker v15)**: 3-minute scripted-play capture
  (telemetry + 121 screenshots) showed FPS 37–38 episodes with **tics/s
  34.6–35.5, `budget:0`, `jump:1`** — logic exact, no sprint, no inflation.
  Integer frames over jittered 0.5s windows swing ±2 around true 35.0, so
  the meter lied and there was no pacing bug left to find. FPS is now
  smoothed over 2s (`fpsSmooth`; rest of stats stay 0.5s). `__doomVer` 41.
- **RANGECHECK abort → clamp (firmware rebuild)**: user hit
  `R_DrawColumn: -10 to 47 at 33` → `I_Error` → `for(;;)` hang (vanilla
  behavior on tall close-up walls; `doomdef.h` defines RANGECHECK).
  All 4 column renderers in `doom/engine/r_draw.c` now clip instead of
  aborting (count recomputed, frac alignment preserved — the stale `count`
  would overdraw). On the emulator an abort hangs the game and an
  unclamped draw wild-writes guest memory (MMIO risk). Rebuilt
  (`make -C doom`, `tools/make_firmware.mjs` → `firmware.js?v=2`, imported
  as `firmware.js?v=2` in doom.js/app.js) and `test_doom_wasm.mjs` PASSes
  (save ok, 0% clip). Dark areas render WITH detail (tech lamps/panels
  visible) — unlit black is correct lighting, not missing draws.
- **Lockstep wasm-vs-Unicorn** (`.pw-scratch/lockstep*.mjs`): same doom.bin
  + WAD, no inputs (deterministic demo): 20M→150M **bit-identical**
  (fb hash, gametic, framecount). Later diffs (same gt, ±2 fc, same
  palette) match sampling phase (Unicorn overshoots each stepped batch by
  a translation block), not divergence. Rust CPU exonerated as the black/
  speed cause.
- **Differential fuzz (fuzz_test/, 500/500 identical)**: fixed SMLAL-arm,
  SMLSD-arm, SSAT/USAT shift-field (imm3:imm2, not contiguous), USAT
   signedness, SMLAD/SMLSD-Q, PKH top/bottom swap, QADD gate+operand order,
   LDRD post-index, UADD8/USUB8/SEL (+GE), UMLAL. MRS-APSR now returns
   NZCVQ+GE (was dropping GE, hiding correct behavior). Census method:
   every opcode form in all shipped `.elf`s checked against decoder arms.
   Full-program differential now **FUZZ-IDENTICAL, 525/525 lines**
   (`.pw-scratch/fuzzcmp.mjs`): all flag corners (FSUB/FADD/SBC0/ADC1, masked
   to NZCVQ like the other APSR ops — Unicorn's real MRS leaves low-bit EPSR
   residue `...01D3` vs our masked `...0000`), SMUAD/SMUSD/SMLAWT/PKHS5/
   SSATSH/USATSH, stamps + iter array identical. Scares along the way, all
   harness artifacts, not guest divergence: (a) the fault-phase `bkpt #0`
   raises **UC_ERR_EXCEPTION (code 21)** on Unicorn instead of a clean stop —
   expected, the fault loop treats any stop as FAULTOK; (b) a step that throws
   strands that step's UART in the model buffer — the phase-1 catch MUST
   `drainUart()` before giving up, else ~30 lines (SMUAD tail + FUZZ-FAULTS
   marker) go missing and it looks like an early crash at a truncated
   `FADD` line; (c) post-exception the Unicorn instance is wedged at the bkpt
   (every later step re-throws — the §7 rule), so fault-phase drains stay
   empty. Harness diagnostics go to console, never into the compared buffer.
- **`doom_sym()` in cpu/tests.rs**: resolves test addresses from
  doom.elf's symtab at test time — hardcoded addresses rot on every
  firmware rebuild (strcasecmp moved twice).
- **Trajectory overlap (the actual verdict)**: per-tic player positions,
  both backends: same-tic distance max 132u/mean 33u, but best-lag
  (nearest-point-on-path) distance max 21u/mean 5u. The two cores traverse
  THE SAME PATH time-shifted ~1-3 tics — no physics/arithmetic divergence.
  The split is input-application timing on the Unicorn harness path
  (stale players.cmd + shorter travel both fit application lag, and the
  lagging side still matches demo bytes when shifted). Combined with
  457/457 fuzz vectors + deterministic wasm replay, the Rust core is
  cleared; remaining autopilot wobble is harness-stepping interaction,
  invisible in play (all 41 presets + doom play/save green on both).
- **Boot-time `initAudio` + resume** (doom.js): audio was keydown-gated, so
  watch-only/scripted sessions stayed silent, and the `if (audioCtx) return`
  early-return never resumed a pre-gesture (suspended) context. Now init at
  boot; keydown/mousedown resumes if suspended. User-confirmed audible on
  v34 (`__audioState()` = `'running'`). The earlier "error from the audio
  device" lines were contention/starved-context cascade on a stale tab, not
  the worklet (re-audited clean: no NaN/poison path).
- **`window.__doom.read(addrs)` + worker `read`/`readResult`**: guest-word
  reads for state-gated smokes (menuactive 0xC00166F8, gamestate 0xC00153AC,
  currentMenu 0xC0016820, itemOn 0xC0016824, gametic 0xC000F350,
  SaveDef 0xC000BBF0 — all verified against current `nm`). Fixed sleeps
  mis-navigate (an Enter eaten mid-boot shifts the whole menu walk).
- Playwright gotchas: Chrome steals **F6** (address-bar focus) — drive it
  via `__doom.key(0xC0)`; UART box needs **Shift+Enter** for `\n`
  (Enter sends `\r` — rx_crypto_test CRCs `Hello\n`, so `\r` FAILs it).
- **Browser save names must avoid w/a/s/d** (`pw_savey` FAIL→PASS):
  doom.js maps those DOM keys to movement codes before the raw-ASCII
  passthrough, so typing `a` in the save slot sends strafe (0xAC), which
  the name entry ignores (`saveCharIndex` stays 0 — diagnosed via the
  `read` hook). Node harness sends raw 0x61 and is unaffected. In-browser,
  name the save e.g. `qpk`. (Same reason `y` works for NIGHTMARE confirm.)
- **Pointer-lock SecurityError** (user-reported): newer Chrome returns a
  promise from `requestPointerLock()` that rejects if re-clicked right
  after Esc — now swallowed with `.catch(()=>{})` (doom.js?v=30).
- **User-side observability** (doom.js?v=31): `window.__doomLog()` journal
  (boot/audio created+resumed+failed/state-flips/worker booted+errors/
  save stored) + `__audioState()`/`__audioStat`/`__audioTotal`. Speed is
  already on the stats line (`FPS: 35/35` = guest frames/sec; 35 exact).
  Verified headless: journal shows boot→audio suspended→worker booted,
  zero page errors.
- **`window.__doomVer`** (v34): build stamp — if the console doesn't print
  the current number, the tab runs a cached copy (Ctrl+Shift+R).
- **audio_test in-browser seed** (app.js, `app.js?v=1`): the browser never
  loaded the 64-sample WAV the node harness feeds via `audio_load_wav`,
  so I2S DMA read fallback audio and the firmware's checksum FAILed
  (`sum=0013F7E0` vs `93C40`) — the DMA path itself was fine (`n=64`).
  app.js now builds the identical PCM16 WAV (`makeAudioTestWav`) and loads
  it on `audio_test` boot; browser prints `DMA RX OK` + `TX n=16 OK`.
