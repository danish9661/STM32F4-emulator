# STM32F4 Emulator — Handover Document
**Date:** 2026-09-13  
**Repo:** `/home/danish1075/Documents/stm32 F4` (note: space in path — always quote)  
**Branch:** `main` (dirty, uncommitted fixes)  
**Last commit:** `3c8ffe5` (green board 2026-09-08)

---

## 1. Project Overview

This repo emulates an **STM32F407** microcontroller (Cortex-M4F) entirely in WASM:

- **Rust CPU core** (`stm32-periph-wasm/src/cpu/`) — Thumb-2 interpreter compiled to WASM, exact inline exception entry/return (NVIC, SysTick, SVC/PendSV, WFI). Unicorn 2.1.4 was the original core and differential-test oracle; **removed in §23** after bit-identical parity proven (543/543 fuzz vectors + lockstep + trajectory overlap).
- **Peripheral model** (Rust, `wasm-bindgen`) — RCC, USART, GPIO, DMA, ETH, TIM, NVIC, USB OTG FS, I2C, SPI, I2S/SAI, LTDC, DCMI, QSPI, CAN, DMA2D, FPU (VFPv4-SP), MPU, WDT, RTC, ADC/DAC/SDIO/FSMC, etc.
- **Gateway / test driver (JS)** — Steps the core, drains UART, injects network packets, services ETH TX/RX protocol.
- **Firmware** — Bare-metal Arduino/STM32Cube-style `.ino`/`.c` built with `arm-none-eabi-gcc` (xpack 14.2.1 from Arduino core).

**Key paths:**
```
stm32-periph-wasm/          # WASM peripheral model crate
  src/
    cpu/thumb.rs            # Thumb-2 decoder (1700+ lines, GAS-verified)
    cpu/mem.rs              # FlatMemory (flash + SRAM + EXTRAM)
    cpu/mod.rs              # Cpu, CpuFault, inline IRQ delivery
    peripherals/            # eth.rs, usb.rs, tim.rs, dma.rs, fpu.rs, mpu.rs, etc.
  pkg/                      # wasm-pack output (bindings + cli.mjs + test_*.mjs)
stm32-emulator-main/        # Native SDL emulator (not headless path)
webserver/                  # Old Arduino sketch (not used on Linux)
eth_http/ eth_dhcp/ eth_test/ eth_irq_test/  # Ethernet firmwares
can_test/ blinky/ oled_test/ tft_test/ ...  # 200+ demo firmwares
site/                       # Browser demo (console.html, doom.html, etc.)
  emulator.js               # Universal factory (createEmulator)
  app.js                    # Console UI
  boards.js                 # 5 board variants (F401/F411/F407/F429)
  vendor/                   # wasm-pack --target web output (wasm + SVD)
openhw-local-gateway/       # Go gVisor-tap-vsock gateway (openhw-gw binary)
scripts/verify_ethernet.sh  # Regression: runs all 3 ETH firmwares end-to-end
```

---

## 2. Current State (What We Were Doing)

**Target:** Debug `eth_feat_test` — 60-phase Ethernet feature matrix (IP checksum offload, jabber watchdog, defer/collision, hash/perfect/SA/DA/RA filtering, VLAN, PTP, WOL, wire pacing, LwIP sockets, STOP+WOL wake, PHY/MDIO).  
**Status:** **50/60 phases PASS** (was 49/60 — IPCO OFF fixed by the TST fix), 10 FAIL. Core CPU tests: **180/180 PASS** (cargo, single-threaded).

### Failing Phases
| Phase | Symptom |
|-------|---------|
| **JABBER WD** | `ok=1, t0=0x200005DC, len=0` — TX completes, but RX wait times out. Frame IS in rx_buf (`00000002...`) yet `eth_recv_frame` returns 0. RX head stays `0x003C0000`, rxpoll=1, NIS stuck set (`dmasr=0x20360098`). |
| **DEFER** | Phase 8b DEFER OK / DEFER DROP OK missing. |
| **HPF/SAF/RA/SAIF** | All 10 filter phases missing (MISS) — downstream of JABBER WD (no RX frames delivered → no filter matches). |

### Failing Phases (OLD — IPCO row SOLVED, kept for archaeology)
| Phase | Symptom |
|-------|---------|
| **IPCO OFF** | Diag prints `l=60 mc=0 sa=02:00:00:00:00:01 p=0x1391` (all match criteria met) but `if (l == 60 && mc == 0 && ...)` evaluates **false** — verdict jumps to FAIL. Trace shows mcmp loop region (0x8001c14-0x8001cc0) only **2 hits** in 2080 trace entries — match loop never executes. |
| **JABBER WD** | `ok=1, t0=0x200005DC, len=0` — `eth_recv_frame` returns 0 despite TX descriptor showing completion. |
| **DEFER** | Phase 8b DEFER OK / DEFER DROP OK missing. |
| **HPF/SAF/RA/SAIF** | All 10 filter phases missing (MISS). |
| **Downstream** | Everything after IPCO OFF cascades (no RX frames delivered → no filter matches). |

### Root Cause Hypothesis (SUPERSEDED — the mcmp theory was wrong)
The verdict chain is NOT the mcmp `if` — it is a `tst`/`beq` at 0x8001ef8-0x8001f02 (see "SOLVED: IPCO OFF" above). The mcmp loop runs fine (native LDRB tests green, disasm shows Rt!=Rn). Sub-items 1–3 below are obsolete:
1. ~~Stale `len`~~ — no: `len`/`l` were correct; the Z flag was wrong.
2. ~~mcmp inlining / LDRB Rt==Rn~~ — no: disasm shows no Rt==Rn pair. Do NOT add noinline to mcmp.
3. ~~Branch miscompile~~ — no: the branch correctly followed a wrong flag.

---

## 3. What We've Done (Fixes Applied)

### Core CPU (all cargo 180/180 PASS single-threaded; note §7 parallel flake)
| Fix | Location | Verification |
|-----|----------|--------------|
| **16-bit TST was SUB-flags (IPCO root cause — REAL FIX)** | `cpu/thumb.rs:1163` sop 8: `sub_flags(a,b,1)` → `nz(a&b)` | `tst16_preserves_rn_and_sets_z` test; `eth_feat_test` IPCO OFF OK (was FAIL with frame correct in RAM) |
| VLAN VLANTI==0 disables gate | `peripherals/eth.rs` `accept()` | IPCO still OK; WD loopback accepts again after VLAN phase writes back 0 |
| DMASR W1C recomputes summaries inline | `peripherals/eth.rs` DMA 0x14 arm | cargo green; JABBER WD still FAIL (see §7 — theory disproven, kept as hygiene) |
| Fault-channel leak (deferred data fault) | `cpu/mem.rs` `apply_fault` | `fault_channel_leak` test |
| LDRH T1 zero-offset decode | `cpu/thumb.rs:1155` | `ldrh_t1_zero_offset` test |
| LDRB post-indexed imm8 | `cpu/thumb.rs:1913` | `ldrb_post_indexed_imm8` test |
| LDRB pre-indexed Rt==Rn writeback ordering (PUW) | `cpu/thumb.rs:2239-2248` | `ldrb_pre_indexed_rt_eq_rn` test |
| VFPv4-SP (full FPU: moves, arithmetic, fused VFMA, vcvt, lazy stacking) | `cpu/thumb.rs` + `peripherals/fpu.rs` | 17 native FPU tests |
| MPU full enforcement (region/AP/XN/subregion, background, HFNMI, fetch XN, stacking faults) | `peripherals/mpu.rs` + `cpu/mem.rs` | `mpu_test` 5 regions PASS |
| USB OTG FS device (CDC-ACM echo) | `peripherals/usb.rs` | `test_usb.mjs` PASS |
| DMA2D (F429) convert/blend + IRQ56 | `peripherals/dma2d.rs` | `dma2d_test` 5 phases PASS |
| QSPI indirect read/write + flash backend | `peripherals/qspi.rs` | `qspi_test` PASS |
| ETH: PHY/MDIO peer, TX checksum offload, RX checksum status, accept filtering (perfect/hash/broadcast/multicast/promisc/DAIF/ROD), VLAN, PTP (binary timebase, target trigger, TX/RX snapshots, drift correction), WOL (magic packet + wakeup frame CRC), wire pacing (TX/RX busy_until), LwIP sockets (DHCP/DNS/TCP/UDP server+client), loopback, STOP+WOL wake | `peripherals/eth.rs` + `site/emulator.js` | `eth_feat_test` 49/60 PASS |

### Tooling / Infra
- **wasm-pack** `--target nodejs` (pkg) + `--target web` (site/vendor) — VENDOR_V=27
- **site/vendor/** SVDs restored after each rebuild (wasm-pack deletes dir)
- **npm test** = `cargo test` + `site/test_board_matrix.mjs` (138 firmwares) + `test_browser.mjs` (10 presets headless Chrome CDP) + `test_ws_bridge.mjs` + `test_edge_cases.mjs`
- **Green board script** `.pw-scratch/greenboard.sh` runs all 3 batteries to verdict file

---

## 4. How to Run / Test

### Prerequisites (all installed on this machine)
```bash
# Rust + wasm32-unknown-unknown + wasm-pack 0.14.0 (~/.local/bin)
# Arduino core STMicroelectronics:stm32 3.0.0 (arduino-cli)
# xpack-arm-none-eabi-gcc 14.2.1-1.1 (~/.arduino15/packages/STMicroelectronics/tools/...)
# Go 1.22+ (for openhw-gw)
# Node 22 + npm
# python3 (http.server)
# google-chrome / chromium (headless CDP)
```

### Build Commands
```bash
# 1. WASM peripheral model (MUST use --target nodejs for Node, --target web for browser)
cd stm32-periph-wasm && wasm-pack build --release --target nodejs
cd stm32-periph-wasm && wasm-pack build --release --target web --out-dir ../site/vendor
# After web build: cp monox/stm32f407.svd site/vendor/ && rm site/vendor/.gitignore

# 2. Firmwares (bare-metal Makefiles use TOOLCHAIN env)
export TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-"
make -C eth_feat_test          # builds eth_feat_test.bin / .elf
make -C eth_http               # also eth_dhcp, eth_test, eth_irq_test
# ... any firmware dir

# 3. Regenerate firmware bundle for browser
node tools/make_firmware.mjs   # updates site/firmware.js (base64 blobs)

# 4. Gateway binary (if needed)
cd openhw-local-gateway && go build -mod=vendor -o openhw-gw .
```

### Run Main Test (eth_feat_test)
```bash
cd stm32-periph-wasm/pkg
# Node probe (2500 iters × 20000 steps, IRQETHLP + netsim)
node .pw-scratch/feat_probe.mjs
# Or via matrix (wired into npm test)
node site/test_board_matrix.mjs   # runs all 138 firmwares including eth_feat_test
```

### Run Full Regression
```bash
# Core + device matrix + browser + bridge + edge cases
npm test
# Or individually:
cargo test --release                    # 179/179
node site/test_board_matrix.mjs         # 138/138 firmwares
node site/test_browser.mjs              # 10 presets headless Chrome
node site/test_ws_bridge.mjs            # WS bridge smoke
node site/test_edge_cases.mjs           # bad-image, invalid-MMIO, reset, multi-instance

# Ethernet trio (needs gateway + local HTTP server on 8092)
node /tmp/opencode/http_server.js &     # HTTP server for gVisor
scripts/verify_ethernet.sh 200000000    # 200M inst per firmware
```

### Debug / Trace
```bash
# Per-phase probe with asm trace (edit .pw-scratch/feat_probe.mjs to add traceNotTaken)
node .pw-scratch/feat_probe.mjs

# Get register state at any PC
# emulator.js getRegisters() returns {R0..R15, XPSR}

# Objdump firmware
$TOOLCHAIN/objdump -d eth_feat_test/eth_feat_test.elf | grep -A30 '<main>:'
```

---

## 5. Key Files to Inspect for Current Bug

| File | Why |
|------|-----|
| `eth_feat_test/main.c:963-985` | IPCO OFF match loop + diag + verdict (`if (l == 60 && mc == 0 && ...)`) |
| `eth_feat_test/main.c:137` | `mcmp` function — **add `__attribute__((noinline))`** to prevent GCC re-inlining |
| `stm32-periph-wasm/src/cpu/thumb.rs:2239-2248` | PUW writeback-before-load fix for Rt==Rn (pre-indexed LDRB) |
| `site/emulator.js:1053` | `getRegisters()` for live register capture |
| `.pw-scratch/feat_probe.mjs` | Per-phase probe runner (2500×20000, traceNotTaken option) |
| `site/test_board_matrix.mjs:146-147` | eth_feat_test matrix entry |

---

## 6. Immediate Next Steps (Do These in Order)

1. **JABBER WD** (current headline): `ok=1, t0=0x200005DC, len=0`. Instrumentation so far (all in `.pw-scratch/dbg*.mjs`, since deleted — patterns below):
   - WD TX (1500 B, jab_buf @0x20000C68, desc 0x800005DC) IS serviced by the driver (`[wasm-tx] tdes0=0x800005dc buf=0x20000c68 len=1500`), loopback queues it — but the RX wait at 0x8002A1A spins to timeout while `rx_desc[0]` stays `0x003C0000` (CPU-owned) and rxpoll=1.
   - The frame never reaches `rx_buf` as the WD frame: post-verdict `rxbuf = 00000002...` is a STALE 60 B frame, and `jab_buf` header reads back IP-total `0x0826`/UDP-claimed-len 1 with stored cksum 0 (model `eth_rx_csum_status = 0x0D` = bad on the STALE bytes; FEF was set so it should still forward — the drop is elsewhere).
   - NIS is stuck (`dmasr=0x20360098`, alternating `0x20370099/0x20360098` = NIS flicker from the ISR's own W1C clears). The DMASR-0x14 W1C recompute fix is IN (rebuilt) but did not move it.
   - ISR trace (dbg76): the WD-wait ISR reads DMASR once (`ldr r3,[r2]` @0x8000441), takes the TS branch, W1Cs TS, then falls to the `cbz r1` RS check with a STALE r1 — but gen DOES advance 73→74 inside the ISR, so the completion IS consumed there while `rx_flag` is never set. The waiter's `eth_recv_frame` heartbeat (`DMARPDR=1` every 64 iters) re-arms into a head that never becomes DMA-owned.
   - Next: instrument SAME-step TX-capture → rxQueue → wDeliverRx → RS → ISR → flag with per-step `t0/rx0/flag/gen/rxp/dmasr` sampling from the WD `str r0,[r2,#4]` (DMATPDR write @0x8002A18). Key question: does the loopback frame deliver while the waiter spins, or does it sit queued behind a CPU-owned head (RBUS hold) because the MFC-burst prologue left the head CPU-owned?
2. **DEFER**: phase 8b; diagnose after JABBER WD (may share the RX-delivery root cause).
3. **DO NOT touch**: `mcmp` (exonerated), 16-bit TST (fixed), VLAN gate (fixed), `VENDOR_V` (27, current).
4. Rebuild + probe loop:
   ```bash
   cd stm32-periph-wasm && wasm-pack build --release --target nodejs && cp pkg/stm32_periph_wasm_bg.wasm ../site/vendor/
   cargo test --release -- --test-threads=1   # 180/180 (parallel has a PRE-EXISTING BOOT_LOCK-gap flake: 15 fails, all lockless-snippet races; NOT caused by the TST fix — verify with --test-threads=1)
   node .pw-scratch/feat_probe.mjs f407 2500 20000
   ```

## 7. Important Gotchas (appended 2026-09-13)
- **`getRegisters().PC` lies during exception entry**: it reports the HANDLER pc (0x800043d) while the trace shows thread code still executing — the core updates r15 at entry before the first handler instruction retires. Use `LR == 0xFFFFFFF9/FD` (not PC) to tell handler vs thread mode, and the PC trace (not register sampling) for verdict-site work.
- **`uart_puts` re-entrancy**: stopping at a FAIL marker then single-stepping walks the *print loop* (0x80001E0), not the verdict — drain the print first (step until PC leaves 0x80001Ex) or you will misattribute the verdict path.
- **Single-stepping THROUGH the verdict needs the trace**: `emu.step(1)` + `getRegisters()` sampling misses the 0x8001ef8→beq window (interrupts interleave); `emu.traceStart()/takeTrace()` + post-hoc search is the reliable way.
- **cargo parallel flake (PRE-EXISTING, not from the TST fix)**: `cargo test --release` (default threads) intermittently fails ~15 tests (all pass solo / with `--test-threads=1` → 180/180). Cause: 32 snippet tests (`run_snippet` locks BOOT_LOCK internally, but the test FNS themselves don't hold it) race `boot()`'s `init_svd_for_test` + shared SYS. Always verify with `-- --test-threads=1`.
- **16-bit ALU sop table (thumb.rs:1120)**: sop 8 is TST (`nz(a&b)`, no writeback) — it was `sub_flags(a,b,1)` (CMP). If you touch this table, re-check every sop against ARM ARM: 0 AND, 1 EOR, 2 LSL, 3 LSR, 4 ASR, 5 ADC, 6 SBC, 7 ROR, 8 TST, 9 RSB, 10 CMP, 11 CMN, 12 ORR, 13 MUL, 14 BIC, 15 MVN. (appended 2026-09-13)

- **Space in repo path** — always quote: `"/home/danish1075/Documents/stm32 F4"`
- **wasm-pack deletes `site/vendor/`** on rebuild — must restore SVD + remove `.gitignore` after every web build
- **VENDOR_V** in `app.js`/`doom.js`/`doom-worker.js` must be bumped together after every wasm-pack rebuild (browser caches wasm URL separately from JS import)
- **Node buffers stdout on kill** — use `writeSync(1, ...)` or redirect to file for debugging hangs
- **Headless Chrome CDP** — use page-level debugger socket (`/json` → `webSocketDebuggerUrl` for `type: "page"`), not browser-level
- **No commits** until user explicitly asks — tree is intentionally dirty with fixes
- **Firmware .bin can be stale** vs .ino — always `make -C <dir>` + `tools/make_firmware.mjs` after source changes
- **eth_feat_test matrix entry** uses `irqEth: true, lowpower: true, script: netsim` at 2500×20000 — do not change without understanding the phase timing

---

## 8. Environment Context

- **OS:** Linux (this machine)
- **Node:** 22.x (V8 — the old Unicorn wedge §7 does NOT reproduce here)
- **Chrome:** Headless for `test_browser.mjs` (port 9223, fresh `--user-data-dir` per run)
- **Gateway:** `openhw-gw` on port 5070 (Go gVisor-tap-vsock) — launch with `setsid nohup ./openhw-gw &`
- **HTTP server:** `node /tmp/opencode/http_server.js &` on 127.0.0.1:8092 (required for eth_http/gateway)

---

## 9. If You Need to Reproduce the Bug from Scratch

```bash
cd "/home/danish1075/Documents/stm32 F4"
export TOOLCHAIN="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-"
cd stm32-periph-wasm && wasm-pack build --release --target nodejs
make -C eth_feat_test
cd pkg && node .pw-scratch/feat_probe.mjs
# Observe: "IPCO OFF FAIL" with diag "OFF r l=0000003C mc=00000000 sa=0000000200000002 p=0000001300000013"
```

---

## 10. Contact / Questions

- **AGENTS.md** in repo root has full working agreement + architecture + history
- **cpu_bug.md** in repo root tracks CPU decoder bugs (main-repo fixes only)
- **docs/progress-and-future.md** has FreeRTOS to-do and other deferred items
- **PERIPHERAL_TESTS.md** in `site/` documents every peripheral test

**Do not commit anything.** The user will decide when to commit.