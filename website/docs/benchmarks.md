---
sidebar_position: 6
title: Benchmarks
description: Performance measurements — throughput, soak results, tuning history, and environment reproducibility.
---

# Benchmarks

All runs: Node 22, headless, `eth_http` firmware (DHCP + TCP + HTTP round
trip), Linux. One "round" = one full DHCP → TCP connect → HTTP body →
FIN cycle of the firmware. MIPS = emulated instructions / wall time.

## Current headline numbers (2026-08-09, post wedge-fix)

| Run | Budget | Rounds | TCP fail | Time | MIPS | rounds/s |
|---|---|---|---|---|---|---|
| 100M soak, no restart | 100M inst | **604** | 0 | 47.9 s | ~2.1 | **12.6** |
| 200M soak (2026-08-08) | 200M inst | **1012** | 0 | 870.8 s | ~1.0 | 1.16* |
| 20M quick check (this doc) | 20M inst | 121 | 0 | 10.1 s | ~1.97 | 12.0 |

\* The 200M soak predates the throughput work and used per-round gateway
restarts (~0.7 s per restart), which is why rounds/s is much lower than the
100M run above.

Typical run (20M instructions):

```
real  0m10.138s   user 0m10.569s
121 rounds, 0 TCP fail, ~2 MIPS, ~12 rounds/s
```

## Throughput tuning history (2026-08-08)

20M-instruction `--connect` runs (eth_http, fresh gateway):

| Config | Steps | Rounds | Wall | MIPS | rounds/s |
|---|---|---|---|---|---|
| Baseline (old cli.mjs) | ~2600 | 101 | ~20.0 s | ~1.00 | 5.05 |
| TICK_EVERY=1000 (no poll split) | 1939 | 67 | 11.05 s | 1.81 | 6.06 |
| TICK_EVERY=5000 (no poll split) | 2473 | 95 | 13.9–14.14 s | 1.44 | 6.8 |
| **POLL_EVERY=1000 + TICK_EVERY=5000** | 3061 | 118 | 15.8–16.0 s | **1.25** | **7.46** |

Key wins: batching WASM calls (`tick_n(5000)` instead of per-instruction
`tick`), splitting the cheap all-JS poll (every instruction) from the
expensive WASM poll (every 1000), and removing the per-round gateway
restart (default since 2026-08-09).

## Soak results

### 100M-instruction soak (2026-08-09, final config)
- **604 TCP connected, 0 TCP fail, 0 `fl=18` stale frames, 0 timeouts**
- 47.9 s wall, ~12.6 rounds/s
- No per-round gateway restart (gVisor opens a fresh session per
  connection; the firmware ignores stale `fl=18` frames)

### 200M-instruction soak (2026-08-08)
- **1012 TCP connections, 0 TCP fail, 0 SYN-ACK timeouts** in 870.8 s
- RSS grew linearly 153→214 MB (~0.06 MB/round; not runaway, but not a
  plateau either)

### Connect-mode soaks (external gateway process, `--connect`)
- Two consecutive 100M runs against the **same** gateway process: 1055 TCP
  rounds total (526 + 529 connected), 0 fail, 0 SYN-ACK timeouts, 525
  `RESET` messages. RSS ~193 MB at end of each run — the abandoned-stack
  leak resets with the room, doesn't accumulate.
- Two consecutive 15M runs into 74 stale sessions: 74/77 TCP connected,
  0 fail, 0 timeouts.

### Restart mode (legacy, `GW_RESTART=1`)
- ~1.27 rounds/s (each kill+spawn+reconnect costs ~0.7 s). Only for
  pathological stale-session cases.

## Environment / reproducibility

- Deterministic: the model clock is instruction-count based, so a fixed
  instruction budget is reproducible across machines (wall time varies
  with CPU).
- The gateway must be reachable: check `ss -tlnp | grep <port>` before
  trusting a 0-round result — a dead gateway burns the whole budget in the
  DHCP wait (documented in AGENTS.md §10).
- After a long soak, restart the gateway (`kill <pid>`, relaunch
  `openhw-gw -port 5099`) if the next run shows `TCP fl=10` + recv-wait
  stall — that's stale-session pollution, not a code regression.

## How to measure

```bash
# CLI throughput (gateway mode)
cd stm32-periph-wasm/pkg
time node cli.mjs ../eth_http/eth_http.bin 20000000 \
  --gateway --config=../../eth_http/config.yaml
grep -c "TCP connected" <outfile>   # rounds
grep -c "TCP fail" <outfile>        # must be 0

# Soak with stats
SOAK_STATS=1 node cli.mjs ../eth_http/eth_http.bin 200000000 \
  --gateway --config=../../eth_http/config.yaml

# Browser-mode throughput (runs in rAF steps; ~a round per few frames)
node site/test_flow.mjs
```

## Tunables

| Env | Default | Effect |
|---|---|---|
| `MAX_BATCH` | 20000 | instructions per `emu_start`; must stay < ~40k (Unicorn WASM wedge) |
| `TICK_EVERY` | 5000 | instruction interval for `tick_n()` + watchdog + interrupt checks |
| `POLL_EVERY` | 1000 | instruction interval for DMA/ETH poll checks |
| `GW_RESTART` | 0 | 1 = restart gateway per round (legacy, ~10x slower) |
| `SOAK_STATS` | — | 1 = emit soak statistics at the end |
| `DBG_TX`/`DBG_RX` | — | 1 = trace TX/RX frames |
| `RX_HEX` | — | 1 = dump first 64 B of each injected RX frame |
| `DBG_FLAG`/`DBG_IRQF`/`DBG_PC`/`DBG_GW`/`DBG_DMA` | — | diagnostic traces (see cli.mjs) |
