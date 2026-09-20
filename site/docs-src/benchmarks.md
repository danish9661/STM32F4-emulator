# Benchmarks

All runs: Node 22, headless, `eth_http` firmware (DHCP + TCP + HTTP round
trip), Linux. One "round" = one full DHCP → TCP connect → HTTP body →
FIN cycle of the firmware. MIPS = emulated instructions / wall time.

## Current headline numbers (2026-09-18, Rust core — netsim peer)

Measured this session on the sole Rust Thumb-2 backend (Node 22,
headless, netsim peer — not the gVisor gateway the old rows used, so
rounds/s is not comparable across the two peers; MIPS is):

| Run | Budget | Rounds | TCP fail | Time | MIPS | rounds/s |
|---|---|---|---|---|---|---|
| eth_http netsim soak | 60M inst | **25** | 0 | 2.5 s | **24.3** | **10.1** |
| blinky compute ceiling | 100M inst | — | — | 2.6 s | **38.0** | — |

Harness: `.pw-scratch/soak_netsim.mjs` (eth_http + netsim `onTx`, 200k-inst
batches, rounds counted via `TCP connected`) and
`.pw-scratch/soak_blinky.mjs` (blinky, 200k-inst batches, UART drained).
Logs: `.pw-scratch/soak60.log`, `.pw-scratch/soak_blinky.log`. The blinky
number is the compute ceiling (no ETH traffic); eth_http pays real
per-access model-call traffic (~642 peripheral calls/round), so its MIPS
reads lower while doing more work per instruction — rounds/s is the honest
metric for I/O firmware (see AGENTS.md §16).

## Unicorn-era headline numbers (2026-08-09, post wedge-fix — archaeology)

| Run | Budget | Rounds | TCP fail | Time | MIPS | rounds/s |
|---|---|---|---|---|---|---|
| 100M soak, no restart | 100M inst | **604** | 0 | 47.9 s | ~2.1 | **12.6** |
| 200M soak (2026-08-08) | 200M inst | **1012** | 0 | 870.8 s | ~1.0 | 1.16* |
| 20M quick check (this doc) | 20M inst | 121 | 0 | 10.1 s | ~1.97 | 12.0 |

\* The 200M soak predates the throughput work and used per-round gateway
restarts (~0.7 s per restart), which is why rounds/s is much lower than the
100M run above.

Typical Unicorn-era run (20M instructions):

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
  `openhw-gw -port 5070`) if the next run shows `TCP fl=10` + recv-wait
  stall — that's stale-session pollution, not a code regression.

## How to measure

```bash
# CLI throughput (gateway mode)
cd stm32-periph-wasm/pkg
time node cli.mjs ../../firmware/eth_http/eth_http.bin 20000000 \
  --gateway --config=../../firmware/eth_http/config.yaml
grep -c "TCP connected" <outfile>   # rounds
grep -c "TCP fail" <outfile>        # must be 0

# Soak with stats
SOAK_STATS=1 node cli.mjs ../../firmware/eth_http/eth_http.bin 200000000 \
  --gateway --config=../../firmware/eth_http/config.yaml

# Browser-mode throughput (runs in rAF steps; ~a round per few frames)
node site/test_flow.mjs
```

## Backend note (post-§23)

The numbers above were measured on the old Unicorn 2.1.4 WASM core
(~1–2 MIPS, ~12 rounds/s). The Rust Thumb-2 core (sole backend since
AGENTS.md §23) delivers ~65 MIPS headless and runs the same `eth_http`
flow an order of magnitude faster per wall second; the tuning history
below is preserved as archaeology — the mechanisms (JS hooks,
`maxBatch` wedge cap) no longer exist. Compare runs by rounds/s or wall
time for a fixed workload, never by the MIPS readout (block-counting
era meters over-reported ~1.3×; see AGENTS.md §16).

## Tunables

| Env | Default | Effect |
|---|---|---|
| `MAX_BATCH` | 200000 | instructions per `step()` batch — live in `cli.mjs` (kept large so gateway RX stays prompt) |
| `GW_RESTART` | 0 | 1 = restart gateway per round (legacy, ~10x slower) — live in `cli.mjs` |
| `SOAK_STATS` | — | 1 = emit soak statistics at the end |
| `DBG_TX`/`DBG_RX` | — | 1 = trace TX/RX frames |
| `RX_HEX` | — | 1 = dump first 64 B of each injected RX frame |
| `DBG_GW`/`DBG_RX`/`DBG_TX` | — | live gateway/RX/TX traces in `cli.mjs` (`DBG_FLAG`/`DBG_IRQF`/`DBG_PC`/`DBG_DMA` are Unicorn-era names, no longer read) |
