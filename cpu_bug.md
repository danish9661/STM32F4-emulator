# cpu_bug.md — shared CPU-issue tracker for board agents

The Cortex-M4F core (`stm32-periph-wasm/src/cpu/`, snapshotted under
`boards/*/core/`) is audited and complete (AGENTS.md §25: decoder census,
133 native tests, full battery). This file tracks what is *not* done or
not verified, so board agents (micro:bit v2, UNO R4, …) can claim and fix
items here, and the maintainer can read it back and land the fixes.

## Workflow (read this before touching the core)

1. **Check the OPEN table below** — your issue may already be listed as
   policy (do not "fix" policy items locally).
2. **Claim it**: set `Status: CLAIMED (@you, date)` so two agents don't
   collide. Unclaimed items are free.
3. **Reproduce**: minimal case first — a native `cargo test` (snippet or
   synthetic image, see `src/cpu/tests.rs` patterns) beats a firmware
   run. The core fails loudly with pc/opcode; quote both.
4. **Fix in the MAIN repo** (`stm32-periph-wasm/src/`), never in a board
   snapshot: fix + native test + `cargo test` green + the board-level
   proof that motivated it.
5. **Report back here**: set `Status: FIXED (commit)`, one line on what
   changed. The maintainer re-syncs snapshots (`boards/*/plan.md` §8).

Board-local workarounds for CPU bugs are forbidden — they rot. If you
are blocked, file the item here and continue elsewhere.

## Status legend

- `OPEN` — confirmed or suspected, unclaimed.
- `CLAIMED (@who)` — someone is working it.
- `FIXED (commit)` — landed in main repo; snapshots re-sync from there.
- `POLICY` — deliberately not implemented; do not file PRs against these
  without new evidence (a real firmware that needs it).

## OPEN / POLICY items

### 1. M0+ strict mode — OPEN (the one known CPU task)
Tracking M0+ firmware (micro:bit v1, interface MCUs) needs a `v6m_strict`
gate: fault on UDIV/SDIV, 32-bit Thumb-2, IT blocks, unaligned accesses,
VTOR use, BASEPRI/FAULTMASK, and M4-style MPU — instead of executing
them. The M4 core *runs* M0+ code today (superset) but is not faithful.
Owner: whoever starts the M0+ track. Method: same as every other fix —
GAS probes (`docs/encodings/`) + native tests.

### 2. Peripheral-space holes read-as-0 — POLICY
Unlisted SVD devices return 0 / swallow writes (many are documented-
reserved; HALs probe them). Wild *memory* BusFaults — that split is
load-bearing (the fsmc_test BANK4 probe). Revisit only with a real
firmware that needs a hole to fault.

### 3. UNDEFINSTR / BKPT loud halt — POLICY
Stops the run with pc+opcode instead of taking a guest UsageFault /
DebugMonitor. Any *valid* encoding reaching these is a decoder bug —
file it with the GAS proof and it gets implemented same-day.

### 4. SEVONPEND nuance — POLICY (implemented: entry-sets + pending-wakes)
Entry/return always set the event register and SEVONPEND pending wakes
WFE. Only exotic orderings (pending set+cleared without take, all
before WFE) could differ — no firmware does this.

### 5. DWT beyond CYCCNT+EXCCNT — POLICY
FOLD/CPI/SLEEP/LSU counters read 0. Fold/branch penalties would be
fabricated in an interpreter; sleep isn't cycle-counted; LSU would tax
every memory access. Revisit LSU only with a real consumer.

### 6. FPB / ETM / TPIU / trace sink — POLICY
Debugger fabric with no debugger attached. (ITM port 0 *is* implemented
— it goes to the UART console when TCR.ITMENA+TER[0].)

### 7. STIR software-trigger — POLICY (unmapped-benign)
Writes ignored, no fault. Revisit the day a firmware self-triggers an
IRQ through it.

### 8. FPCCR.USER default / THREAD bit — handled, watch item
USER defaults 0 (unpriv FPU faults NOCP until firmware sets it —
correct). If a port's RTOS does unprivileged FPU without setting USER
and faults, that is the firmware's bug, not the core's — but flag it
here before assuming.

### 9. Float-heavy new firmware — watch item, not a bug
The FPU is exact (see §25), but every new float-heavy firmware deserves
a result-check harness like `fpu_test` (bit-exact expectations, not
vibes). If numbers differ, file with the failing vector.

### 10. Raw group-space PRIGROUP/BASEPRI compare — assumption on record
Exact under the CMSIS shifted-value convention (what FreeRTOS/Arduino/
Nordic all write). If a firmware writes *unshifted* priority values,
ordering still holds but masking granularity shifts — flag it here.

## CLOSED log

Everything else ever found is closed with a native test — the full
record lives in AGENTS.md §25 (decoder census, FPU sessions, MPU
enforcement, interrupt fidelity parts 1–2, audit passes 1–4). Do not
duplicate it here; link it.

## New-entry template

```markdown
### N. Short title — OPEN
Trigger: (firmware + what it does)
Expected (silicon): ...
Observed (core): ... + exact pc/opcode/faultinfo
Pointers: (files/lines)
Status: OPEN
```
