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
F4 (verified 2026-09-16, was already fixed; uno-r4 report confirmed)
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
Status: VERIFIED FIXED in F4 (2026-09-16): this tree's `take_exception`
has no `r13 = msp` reload (comment cites #11 explicitly) and
`exception_return` unstacks from live `r13` for F1/E1/F9/ED with the PSP
bank only for FD/ED thread-PSP returns. The dedicated native test
`cpu/tests.rs::nested_push_outer_returns_clean` (pushing outer + preempting
inner, the exact hole — existing nesting tests used stackless spinning
handlers) passes: `cargo test --release nested_push_outer_returns_clean`
1 passed. No edit needed; closing the mirror item.

### 12. Predicated 16-bit flag-setting clobbered flags for the next IT slot — FIXED in F4 (uno-r4 report confirmed + generalized)
Trigger: Arduino `Serial.print(int)` (printNumber's `ite le; addle r3,#48;
addgt r3,#55`): every digit printed +0x37 ('4' -> 'k').
Expected (silicon + GCC + Unicorn, ARM ARM: "16-bit instructions in the IT
block, other than CMP, CMN and TST, do not set the condition flags"):
predicated T1 ALU preserves APSR, so the else-branch sees the pre-block
flags (the F4 codebase already did this for MOVS-imm/ADD-reg/SUB-reg:
D_PageTicker, S_Start).
Observed (F4 core): only the ADD/SUB-reg + MOVS-imm arms had the `it_pred`
guard — the uno-r4 imm arms (0x1C00/0x1E00/0x3000/0x3800) lacked it here
too, AND the same hole covered shifts-imm (0x0000/0x0800/0x1000), the whole
0x4000 ALU table (AND/EOR/LSL/LSR/ASR/ADC/SBC/ROR/RSB/ORR/MUL/BIC/MVN),
and the 0x4400 CMP-hi arm (CMP-hi itself is a flag-setting exception, but
its ADD-hi/MOV-hi neighbours must preserve). Undoing the narrow report
would have left ~20 arms still clobbering (any `itt` + shift/ALU firmware
re-fails). Native repro: hand-assembled udiv+mls+cmp+ite+adds gives
r3=103, fixed to 48 (same shape as uno-r4's ra4m1 proof).
Pointers: F4 `stm32-periph-wasm/src/cpu/thumb.rs` `exec16` — every 16-bit
writeback arm now computes + stores unconditionally and only calls the
flag setter when `!it_pred` (ADC/SBC/RSB/ADD/SUB-imm split into
wrapping-arith + gated `add/sub_flags`; CMP-imm/CMP-hi/CMN/TST pass
through ungated as the three exceptions). Proof is
`cpu/tests.rs::it_block_16bit_preserves_flags` (GAS vectors from
xpack arm-none-eabi-as, `.pw-scratch/it12_probe.s`: addle/addmi/CMPEQ-
sets/ADDNE-preserves/shifts/AND-ORR/TST-sets).
Status: FIXED in F4 (this commit); cargo 181/181 single-threaded,
feat probe 60/60 f407, matrix eth_feat 2/2.

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

### 13. Branch PC base: RAW pc+4 is correct for B/BL (NOT word-aligned) — CORRECTED
Trigger: Arduino AnalogWave `wave.begin()` never took effect (no GPT,
no DTC, LED off, no fault) — first suspected as a BL PC-align bug.
Analysis (CORRECTED after llvm cross-check): for `bl setup` at 0x9ed6
(F7FA F913, off=-24026) raw pc+4 lands exactly on 0x4100 (llvm agrees);
word-aligned (pc+4)&~3 lands on 0x40FE (wrong). Same for `bl main` at
0x7b4e (raw 0x44EC = llvm's target; aligned 0x44EA = wrong). A trial
align-everything patch broke Reset_Handler->main immediately (HardFault
at 0x7b50), confirming raw pc+4 is the silicon rule HERE (the
`(addr+4)&~3` rule applies to LDR-literal/TBB data accesses, which the
core already does, and to R15 *reads* — not to branch offset bases).
The AnalogWave stall is therefore NOT a branch bug: setup runs, begin
runs, begin returns false (or DTC never fires) — under separate
investigation in uno-r4 (BKPT-as-nop change already landed for the
abort() trap at setup+0x20). Lesson: end-PC sampling misses one-shot
init code (setup ~10 instr inside one 500-step); use state signals
(DTCVBR/DADR/PORT), not PC sampling, for init tracing.
Pointers: uno-r4 `core/*/src/cpu/thumb.rs` (reverted, raw pc+4 kept).
Status: CLOSED (no bug — decoder was right).
Trigger: Arduino AnalogWave `wave.begin()` (any sketch whose `bl target`
sits at pc&2 != 0): `bl setup` at 0x9ed6 never lands — setup/begin/DTC
never run, sketch spins in `loop`, no fault (looks like a peripheral
stall, not a CPU bug).
Expected (silicon): Thumb PC reads `(addr+4)&~3` (word-aligned), so BL
from 0x9ed6 uses base 0x9ed8 and lands on setup at 0x4100 (llvm-objdump
agrees).
Observed (core): `thumb.rs` BL/B.W used raw `pc+4` (0x9eda), landing at
0x4102 — setup runs minus its `push {r3,lr}`, returns via `pop {r3,pc}`
to garbage, and by luck lands back in the main loop. Other firmwares
passed by luck (calls from pc&2==0 are unaffected; +2 on others usually
lands benignly). Native repro: hand-assembled `LDR r0,=PORT1 /
LDR r1,=bits / STR` pair needs the `0x4804/0x4905` staggered pair
because of the same alignment rule (proven in the EK zero-boot test).
Pointers: uno-r4 `core/*/src/cpu/thumb.rs` BL arm + B.W arm
(`pc.wrapping_add(4)` → `(pc.wrapping_add(4)) & !3`); same for CBZ/CBNZ
and B.cond/B (16-bit, same rule). LDR-literal/TBB already align.
Status: CLOSED — see CORRECTED analysis above.
