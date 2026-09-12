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

### 1. M0+ strict mode — DROPPED (not needed, 2026-09-12)
Closed without implementation by maintainer decision; do not reopen or
mention in future work lists.

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

### 11. Nested exception SP used the stale bank, not live r13 — FIXED in
uno-r4 cores (mirror here if the F4 core shares the code)
Trigger: Arduino UNO R4 echo firmware (TinyUSB CDC, 1-byte bulk transfers
at full speed) with AGT0 1ms IRQs nesting inside the USB ISR: after ~31
transfers the core jumped to 0x1C and faulted (UsageFault on 0xFFFF).
Expected (silicon): handler mode runs on MSP and r13 is its live SP;
takes push at live SP, returns pop from live SP. The nested AGT take
pushes below the USB handler's live SP; the nested return pops it back.
Observed (core): `take_exception` reloaded r13 from the stale MSP bank
on entry (stacking the nested frame over the outer frame), and
`exception_return` unstacked from the stale bank instead of live r13
(the nested return left the bank at the dead inner frame). The outer
return then resurrected stale stack words as PC (0x1D -> 0x1C fault).
Single-level IRQs never noticed (bank == live SP when balanced).
Pointers: uno-r4 `core/*/src/cpu/mod.rs` `take_exception` (removed
`r13 = msp` reload) and `exception_return` (unstack base: live r13 for
F1/E1/F9/ED, PSP bank only for FD/ED thread-PSP returns). Proven by
`ra4m1_usb_cdc_echo` (100B two-packet round-trip through real CDC bulk
pipes with AGT running) plus the full suites staying green (149
snapshot incl. 128 legacy CPU, 21 small-core).
Status: FIXED in uno-r4 (both cores); F4 core should take the same two
edits if its take/return still use the bank.

### 12. Predicated T1 ADD/SUB-immediate clobbered flags for the next IT slot — FIXED in uno-r4
Trigger: Arduino `Serial.print(int)` (printNumber's `ite le; addle r3,#48;
addgt r3,#55`): every digit printed +0x37 ('4' -> 'k').
Expected (silicon + GCC + Unicorn): predicated T1 ALU preserves APSR, so
the else-branch sees the pre-block flags (the codebase already did this
for MOVS/ADD-reg/SUB-reg: D_PageTicker, S_Start).
Observed (core): the ADD/SUB-imm3/imm8 arms (0x1C00/0x1E00/0x3000/0x3800)
lacked the `it_pred` guard, so `addle` cleared N and the skipped `addgt`
ran too (r3 = 4+48+55 = 107). Native repro: hand-assembled
udiv+mls+cmp+ite+adds gives r3=103, fixed to 48.
Pointers: uno-r4 `core/*/src/cpu/thumb.rs` ADD/SUB-imm arms now mirror the
ADD-reg arm (`if !cpu.it_pred` around `add_flags`/`sub_flags`); proof is
`ra4m1_ite_add_imm_preserves_flags` in both cores' ra4m1.rs.
Status: FIXED in uno-r4 (both cores); F4 core should extend its own
it_pred guards the same way if its imm arms lack them.
Status (F4, 2026-09-11, UNCOMMITTED): VERIFIED shared (return-side half)
+ FIXED. F4 entry already stacked at live r13 (no stale reload); the
return unstacked from the MSP bank, which goes stale whenever a handler
moves SP (handler-mode bank sync is skipped by design), so an outer
return after a nested preemption popped garbage. Fix (`cpu/mod.rs`):
unstack base is live r13 for F1/E1/F9/ED, PSP bank only for FD/ED;
removed the dead `r13 = msp` reload in take. Native test
`nested_push_outer_returns_clean` (pushing outer + preempting inner,
marker + thread-resume asserted) FAILS pre-fix (bad access via stale
unstack) and passes post-fix; cargo 139/139. Existing nesting tests
used spinning (stackless) handlers — that was the coverage hole.

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
