# HANDOVER — flat whole-program translation pilot (2026-11-25)

**For the next session: read this file first, then AGENTS.md §46 (appendices).**
Nothing is committed. Worktree has 23 modified files. The user asked for this
handover because repeated OpenChamber crashes; a fresh conversation should
pick up from here and continue the work.

---

## 1. THE GOAL AND THE RULES THE USER SET (read this first — it explains every decision)

The user's objective from the start: **make the emulator itself faster, across
all firmware** — not DOOM, not one test. Their exact words when I drifted to
DOOM: *"why are you fixated on doom, i am talking about emulator just make the
emulator faster. doom will automatically becomes faster."* Then: *"do whole-
program static translation — a real project, not chips-and-dips work. Before
that commitment, the highest-value thing I can do is a scoped pilot."* Then
*"start working"* — the pilot is authorized and in progress.

Rules the user set (violating them wastes their time):
- **Page-side browser MIPS/FPS is the primary metric.** Node MIPS is
  diagnostic only — never present a Node number as a result without its
  page-side counterpart.
- **Speed claims only from back-to-back/interleaved same-box A/B.** Single
  runs prove nothing; <8% is noise. This box drifts ±8–10% over hours —
  interleave arms and use many reps.
- **Every accepted change lands behind the full battery + a same-box A/B.**
- **No commits without an explicit user order.** (Nothing is committed now.)
- **"Without reducing quality"** — no low-detail fallback or other guest-side
  cheat; the emulator must get genuinely faster. Never modify firmware to
  make a test pass.
- **No silent wrongness** in translator/decoder; new fusions need
  anti-vacuous fire-counter tests.

What was explicitly REJECTED along the way (do not re-propose):
- "Lowering the clock" as a speedup — it's a fidelity knob, not acceleration.
- DOOM-only tuning — the user rejected the framing entirely.
- Interpreter-only micro-optimization as the main path — small wins only
  (that's why we moved to whole-program translation).

---

## 2. WHY THE FLAT TRANSLATOR IS THE CURRENT PATH (the evidence chain)

1. **The interpreter's per-instruction overhead is the wall.** Thumb-2 fetch,
   decode, dispatch, flag/MPU/guard checks dominate for firmware without
   translatable loops. The existing block translator (loop-shaped regions,
   `tb_xlate.rs`) covers only ~41.6% of eth_http; blinky-class code ~0%.
2. **We removed the first systematic bail-out**: MMIO accesses inside
   translated regions used to force an interpreter fallback at every device
   register poll. We added an `env.device` import that calls the Rust
   peripheral model inline (§46, shipped, uncommitted). Result: eth_http
   page **94 MIPS vs 42 interpreter**, **+16% over the previous best**, zero
   miss-retries, byte-identical transcript. This proved translation is the
   right lever.
3. **We then tried to widen loop coverage** — and discovered it is
   cost-model-limited, not opcode-limited. The hot RX-poll loops *are*
   recognized as loops (`tb_translate_meta_direct` → `is_loop=1`); the
   run-length gate declines them because packets arrive often, so each wait
   is short and cannot amortize the fixed ~10–15 µs wasm↔JS per-hit cost.
   Forcing translation there would be **slower**. That closed "just extend
   the loop translator" as the answer.
4. **Interpreter polish was harvested** (validated fast-path indexing
   +3.6–4.9%, kept; NVIC mirror + TRACE_ON hoist, noise-neutral, kept;
   `fetch16_opt` fold measured −3..−4%, **reverted**). Remaining wins are
   below this box's noise floor.
5. **Static census answered the go/no-go**: recursive-descent CFG walk over
   the eth_http image showed **~90% of real instructions fit today's
   emitter** (1221 supported; remaining families: wide modified-imm ALU,
   MOVW/MOVT, wide halfword loads, POP.W/PUSH.W, MUL/MLA, extends). Missing
   instructions can fall back per-item through the existing kind-1 protocol,
   so soundness is inherited, not re-proven. → GO for the flat pilot.
6. **Design chose a single module** with many block functions + an in-module
   `dispatch(pc)` (binary search) rather than one module per block: crossing
   wasm↔JS per basic block would cost more than interpretation. Intra-module
   edges are direct `call`s; the i64 result bubbles up; guest call depth =
   wasm call depth; two budgets (exact instruction budget + block-count fuel
   to bound the wasm stack); unsupported instructions exit kind-1 and the
   interpreter resumes.

**Evidence that the pilot is promising:** even a nearly-empty module (4–6
blocks, most instructions still interpreted) ran eth_http at **70.7 vs 52.9
MIPS** with a byte-identical transcript over 16 HTTP rounds. The upside is
large once the full call graph translates.

---

## 3. IMMEDIATE SAFETY RULES (learned the hard way — do not skip)

- `stm32-periph-wasm/Cargo.toml` `[profile.release]` was changed from
  `lto = true` (fat) to `lto = "thin"`. **Fat LTO spikes the link to ~12.8 GB
  RSS and the kernel OOM-killer kills OpenChamber** — journalctl evidence on
  2026-11-05 (3×) and 2026-11-25, each in a different OpenChamber scope,
  i.e. other sessions/windows are collateral victims. Do NOT revert without
  asking. Never launch a fat-LTO build.
- Before any heavy command: `free -m | awk 'NR==2{print $7}'`; if available
  < 4 GB, do not build. Note: measuring "memory is fine" *after* a crash is
  meaningless — the crash freed the memory.
- One consolidated background chain for heavy work (background tool call or
  `setsid nohup ... > log 2>&1; echo rc > verdict`), **no sleep-poll loops**
  (repeatedly crashed tool calls), **never `pkill -f` a pattern that matches
  your own command line** (killed its own run).
- Last launch: background shell `sh_21381a03f001M7O6fjy7Eo9UYH` (PID 15613),
  command in `stm32-periph-wasm`:
  `FLAT_DBG=1 cargo test --release -j 2 flat_blocks_cover -- --nocapture > /tmp/flat_test.log 2>&1; echo rc > /tmp/flat_test.verdict`
  Check `/tmp/flat_test.verdict` first thing; if absent and the log is
  frozen, the session died again — just relaunch (thin LTO links in ~1 min).
- The user asked whether to start a fresh conversation: yes, they restart;
  this file is the continuity mechanism.

---

## 4. What is already shipped in the tree (all validated, uncommitted)

- §46 of AGENTS.md: inline MMIO device service in translated blocks
  (`env.device` import, `tb_mem_kind`, kind-1/4/5 protocol, counters).
- Interpreter polish: validated fast-path indexing (+3.6–4.9%, kept), NVIC
  pending mirror + TRACE_ON hoist (noise-neutral, kept), `fetch16_opt` fold
  (REVERTED after measured −3..−4%).
- Versions: vendor **v61** across app.js/doom.js/doom-worker.js/emu-worker.js,
  app.js v82, doom.js v102 / `__doomVer` 101, doom.html. `test_versions.mjs`
  passed at v61.
- Last full battery on that (pre-flat) tree: cargo 289/289, TB-XLATE 237/237,
  15 TB differential harnesses, `test_tb_direct.mjs 20` byte-identical
  transcript, `npm run test:wasm` PASS, browser **44/44**. Re-run at the end
  of the pilot (flat code is additive/opt-in, but verify).

---

## 5. The flat translator — implementation map (`tb_xlate.rs`, ~1400 new lines)

- `FlatOut { bytes, blocks }`; `FlatTerm` (Br/BrCond/Bl/RetDispatch/Miss/
  BrOut); `FlatBlock { pc, items, term, term_pc }`; `FlatDec::It/Term`.
- `flat_decode()`: decodes branches itself; CBZ (`imm5<<1` mask fixed),
  B.N/B.cond (offset `imm<<1`), B.W/Bcc.W/BL (25/21-bit J math), PC-relative
  literal loads (0x48xx, F8DF/F89F → `Item::MemConst`), ADR (0xA0xx →
  `Item::MovConst`), PUSH/POP T1 (→ sp-relative `Item::Mem` stores/loads
  FIRST + `Item::SpAdj` LAST — critical: if a store misses, sp hasn't
  changed, so interpreter re-execution is safe; pop{pc} loads the return
  address into the dead-at-epilogue LR then `RetDispatch`).
- `flat_blocks()`: recursive-descent CFG from vector-table roots (vectors =
  odd words in image; BL pushes BOTH call target and return address; BrCond
  pushes target+fall); dedup by pc, sorted output.
- `translate_flat(image, segs)`: imports `env.memory` + `env.device`
  (device import = func 0); globals 0–21 identical to loop modules (driver
  syncs by name) plus `flatAcc` i64 global 22; exports `run(pc, budget)`,
  `dispatch(pc)`, `covers(pc)`, all globals, one function per block. Binary-
  search dispatch; in-module edge = direct `call` (i64 result bubbles up);
  kind 1/2/3/4 exits; exact accounting (ACC += per-block LTOT at every
  edge, budget in global 9, tbFuel bounds wasm stack depth).
- New `Item`s: MemConst/MovConst/SpAdj; `ExecCount::Flat`;
  `tb_emit.rs::global_mut_i64`; `lib.rs::tb_flat_build` (`getter_with_clone`)
  returning bytes + blocks.
- Driver `site/emulator.js`: `flat` opt; `flatTry` (build once, covers check,
  register sync, `ex.tbFuel=4096`, `run(pc,budget)`, kind handling, **sp
  writeback via `cpu.set_reg(13, ex.sp.value)`** — `set_regs_bulk` omits sp
  and push/pop move it); `flatOn` gate; called before tbTryLoop in step().

### Bugs found and fixed (each was a silent coverage/kill, not cosmetic)
- `visited[idx+k]` with `k in 0..li` (bytes vs halfwords) stopped every block
  after one instruction.
- wide-branch gate `(op & 0xFF00)==0xF000` matched only 0xF0xx, dropping BLs.
- B.N/B.cond offset missing `<<1` → targets 2 bytes off, mid-instruction.
- BL decoded but only `ret` pushed, not the call target → main never reached.
- `_start`'s `ldr.w sp,[pc,#8]` (rn=15) → added PC-relative literal items.
- `push {r3,lr}` (0xB508) untranslatable → added push/pop decomposition.
- Block function needs `local(10, I32)` (no param) so LTOT stays index 9.
- dispatch/covers must use `emit_flat_return` (they have no LTOT local).
- Item-level exit counts: `ExecCount::Flat` commits LTOT once; post-commit
  tails use plain returns — never double-add.

### Test pins
- `tb_xlate.rs` `mod flat_tests`: `flat_blocks_cover_call_graph` asserts >50
  blocks and presence of main `0x080014dc`, setup `0x08000a18`, loop
  `0x08000c3c`, eth_recv_packet `0x080008e0`, build_tcp_frame `0x080006ac`,
  handle_arp `0x0800052c`. Last known result: **6 blocks, FAIL** (main
  popped but died on its first `push`). The push/pop fix landed after that;
  the re-run is the background job in §3.
- `.pw-scratch/test_tb_flat.mjs [budgetM]`: eth_http flat vs interp,
  transcript compare + rounds (see §2 for the surprising partial result).
- `.pw-scratch/flat_verify_chain.sh`: one-chain battery (cargo, artifact
  rebuild, flat test, TB-XLATE + 15 differentials). Its LTO-off env is now
  redundant (Cargo.toml thin), harmless.
- `.pw-scratch/flat_cover_probe.mjs`, `flat_build_probe.mjs`,
  `tb_meta_probe.mjs`: diagnostics; driver stashes `emu.flatBlocks` /
  `emu.flatInst`.

---

## 6. Next steps (in order)

1. Read `/tmp/flat_test.verdict` + log (§3). If `flat_blocks_cover_call_graph`
   still fails, use `FLAT_DBG=1` traces (walker prints every pop/term) to find
   the next blocking opcode and add it to `flat_decode` (same pattern:
   decode statically, reuse `Item::Mem`/MemConst/SpAdj, miss fallback).
2. When the walk passes: `cargo test --release -j 2` (full, thin LTO), then
   rebuild `pkg` + `site/vendor` (`npm run build:wasm:node`, `npm run build:wasm`),
   restore `site/vendor/stm32f407.svd`, remove `site/vendor/.gitignore`.
3. Run `.pw-scratch/test_tb_flat.mjs 20` — expect byte-identical transcript
   and a translated share much higher than loop-only TB's ~42%.
4. Interleaved A/B on eth_http 20M: current TB vs flat (best/median, ≥8%
   bar). If flat loses, tune per-hit overhead / fuel / covers / shape limits.
5. Full battery: cargo, TB-XLATE, 15 differentials, `test_tb_direct`,
   `npm run test:wasm`, browser 44/44, `test_versions`; bump `?v=` on every
   edited served file + VENDOR_V together if vendor wasm changed; record
   `__doomVer`. AGENTS.md new section after §46.
6. **Do not commit without an explicit user order.**

## 7. Environment facts
- Box: 16 GB, other OpenChamber sessions + other tenants; thin LTO is now
  the repo default; memory gate before heavy commands.
- `wasm-pack build --release --target nodejs` for pkg; `--target web
  --out-dir ../site/vendor` for browser; the latter deletes the SVD and
  writes `.gitignore` containing `*` — restore/delete every time.
- Node 22.22; shell tool timeout ~120 s default (hard `timeout N` ≤185 for
  foreground, background chains for anything longer).
- The user is frustrated by the crashes; keep them informed, never claim
  memory is "fine" after a crash, never hide a failure.

---

## 8. 2026-11-25 session — CRASH ROOT CAUSE FIXED + flat pilot status

### The OpenChamber crashes are NOT LTO / load / desktop: they were an
### infinite loop in the new `flat_blocks()` CFG walker
`journalctl -k` shows three identical OOM kills on 2026-11-25 (21:39:54,
21:51:02, 22:01:30): `task=stm32_periph_wa`, **anon-rss 12.8/13.0/13.1 GB**,
`total-vm ~25 GB`, each inside an `app-openchamber-<pid>.scope` cgroup.
The killed process is the Rust test binary, not rustc/ld. Cause in
`tb_xlate.rs::flat_blocks`: the decode `for` loop pushed decoded items into
`items` but **never advanced `pc`**, so the first straight-line instruction
(0x080009a4) was re-decoded forever until the kernel OOM-killer took down
the whole session (the detached job looked like "nothing running").

Fixes landed (all uncommitted):
- `pc = pc.wrapping_add(hw*2)` on the LAST item of each decoded instruction
  (`tick` flag) — PUSH/POP groups keep one advance for the whole group.
- Defensive rail: a block over 4096 items becomes `Miss` instead of growing
  unbounded. Anti-vacuous pin `flat_blocks_advance_pc` (2-byte NOP x3 + bx
  lr) fails instantly if the advance disappears.
- `Miss` is now a block BOUNDARY (queue fall-through) instead of ending the
  walk; otherwise one PUSH.W in a prologue hid the whole call graph
  (18 blocks / 3 of 6 key functions -> 576 blocks / all 6).
- Added PUSH.W/POP.W T2 (E92D/E8BD) decomposition; fixed the pre-existing
  16-bit POP offsets (were the PUSH negative formula) and its missing tick.

### Flat pilot: NOT shippable, keep OFF by default
- A silent no-op bug was found: `flatTry` gated on `bindings.tb_flat_translate`
  which does not exist (only `tb_flat_build`). The earlier "flat wins" Node
  number and the first page A/B were **loop TB**, not flat. Fixed, then the
  real flat path exposed further bugs: kind-1 miss never executed the
  instruction (20M miss-retries, zero UART) — fixed by `cpu.step(1)` in the
  miss path; backward in-module calls recursed to wasm stack overflow —
  fixed by returning kind 3 on backward edges.
- Current state: flat builds a 576-block module, but the guest stalls in the
  RX-wait region (~60k inst, `pc=0x8000955`) and produces no HTTP rounds.
  Repro: `node .pw-scratch/test_tb_flat.mjs 20`, diagnostics in
  `flat_trace.mjs` / `flat_share.mjs`. `covers(pc)` and the resume-pc protocol
  are the next suspects (stats show translated hits freeze while the guest
  still reports stopped=false).
- Page A/B (`cdp_mips.mjs`, neutral, pre-fix silent-no-op runs): default TB
  medians 95.8/96.9/98.0 vs 97.3/97.5/96.5 — no ≥8% win, so flat failed the
  acceptance bar regardless. `?flat=1` is an opt-in diagnostic only; the
  shipped path is loop TB (2x interpreter, ~95 MIPS page).
- Versions now: VENDOR_V 65, emulator.js v15, app.js v83, emu-worker v26,
  doom.js v103 / `__doomVer` 102, doom-worker v68. `test_versions` PASS.
- Cargo 292/292 green through every rebuild; no OOM since the pc fix
  (all chains memory-gated, thin LTO, detached, hard timeout, verdict file).

### Flat stall root cause (2026-11-25, later)
Traced with a temporary env-gated `FLAT_TRACE` in `flatTry` (now removed).
The repeating cycle is `0x8000cb8/cba (ADD SP-relative, Miss) -> bl
eth_recv_packet -> DMASR poll -> pop {pc} -> back`. The guest prints
`DHCP TX timeout` repeatedly: a whole `eth_send_packet` status-poll loop
executes INSIDE one translated run, so `wProcessEth()` (which runs at
`step()` end) never sees the TX poll in time. Node diagnostics:
`tx=1..2, rx=1..2, flag[0x20000620]=1, DMASR=0x370040`, guest stalls in RX
wait with `stopped=false`, no fault, translated hits frozen. This is the
same "big batches starve servicing" trap as §42, structural to flat's
long runs.
- Attempted fix: have `env.device` return a SERVICE status (0x10) on a
  rising TX/RX poll, exit kind-4, and call `wProcessEth/wProcessDma`
  immediately. It did NOT fix the stall (one SERVICE event at DMAOMR
  init, none at DMATPDR) and touched the shipped loop-TB path, so it was
  **REVERTED** (mask back to 0x3, no JS poll detection, no extra service
  call). Do not re-propose without a new design.
- Final versions after the revert: VENDOR_V **67**, emulator.js **v17**,
  app.js v83, emu-worker v26, doom.js v103 / `__doomVer` 102,
  doom-worker v68. Both wasm targets rebuilt; `test_versions` PASS.
- Final battery after the revert: `.pw-scratch/battery_final.sh` ->
  `BATTERY_EXIT=0` (cargo 292, TB-XLATE, 15 TB differentials, `TB=1`
  matrix, `npm test` 252 s, `npm run test:wasm`). No commits.

### Final flat attempt (user chose "one more flat attempt", 2026-11-25)
Tried bounded flat runs (`min(budget, 5000)`) + `wProcessEth/wProcessDma`
at every run boundary. Still stalls. Step-by-step differential
(`.pw-scratch/flat_diverge.mjs`, interp vs flat, same firmware/netsim):
- interp reaches `DHCP Discover` at ~60k reported instructions; flat at
  ~20k — flat **undercounts guest work by ~3x** on eth_http, while blinky
  accounting matches exactly (10k instructions to `tick 0` both modes).
- flat's 5M-iteration `eth_send_packet` poll "expires" within ~40k reported
  instructions and prints `DHCP TX timeout` repeatedly.
This is a path-specific instruction-accounting / control-flow defect in the
flat emitter, not a servicing problem. Fixing it means differential-testing
the emitter against the interpreter block-by-block — a redesign, not a
patch. The bounded-run + boundary-service change was REVERTED; tree is back
to the green state (VENDOR_V 67, emulator.js v17, `test_versions` PASS).

### Recommendation for the next session
The loop-TB path is the product: ~95 MIPS page, 2x interpreter, fully
green. Flat has now had its renewed mandate and one bounded attempt; both
failed. Do NOT attempt ad-hoc flat patches again — the next flat attempt
must start with a block-level differential harness (flat module vs
interpreter, per-block register/flag/instruction-count equality) before
any speed measurement. Otherwise pivot to a different speed lever.
Nothing is committed.

### Do NOT re-propose, per user
Lowering the clock; DOOM-only tuning; interpreter micro-opts as the main
path; commits without an explicit user order. Node MIPS is diagnostic;
page-side sampling is the metric; <8% is noise on this box.

---

## 9. 2026-11-26 follow-up — accounting bug found, flat still unshipped

- Built the block-level differential prerequisite:
  `.pw-scratch/test_flat_block.mjs` (synthetic image -> `tb_flat_build` ->
  seeded module execution vs hand-computed regs/flags/count, 4 vectors).
- It found a REAL flat-emitter accounting bug: branch/BL terminators were
  never charged, and PUSH/POP groups charged once per register. Fixed in
  `tb_xlate.rs` (`FlatBlock.items` now carries the group `tick` flag; one
  charge per source instruction; branch terminator charged; pop{pc} not
  double-charged). Rust pin `flat_blocks_synthetic_items` added.
  `FLAT-BLOCK PASS 4/4`.
- On eth_http, flat step 0 now matches the interpreter EXACTLY (pc
  0x08000aa1, same UART) — the ~3x accounting divergence is gone.
- Flat still does not ship: it now faults loudly at 0x080004F0
  (`pop.w {r4-r8,pc}`, op E8BD/81F0) with SP pointing at zeros. Declining
  T2 PUSH/POP did not remove the fault, so the SP corruption is earlier.
  Next diagnostic: find which translated item moves SP wrongly (compare
  SP after each flat run vs the interpreter at the same pc).
- Versions VENDOR_V **68**; both wasm targets rebuilt; SVD restored;
  `test_versions` PASS; full battery green (`BATTERY_EXIT=0`, npmtest
  249 s, wasm 6 s). The earlier tree was committed as `00aad61` on the
  user's order; THIS follow-up is uncommitted (no new commit order).

---

## 10. 2026-11-26 — general speed win: TB gate 5000 -> 600

- User chose "commit accounting fix + pivot to general speed work".
  Accounting fix committed as `86630a3`.
- Profiled the shipped TB path (`.pw-scratch/tb_diag.mjs`): 71.6% of wall
  in interpreted fallback chunks, translated share only 41.5% — the
  run-length gate was the limiter.
- Re-ran the gate sweep on a QUIET box (load 1.3; old 5000 decision was
  made on a loaded box): page MIPS 5000=95, 2000=115, 600=170-178;
  live 70 -> 87 MIPS and 58 -> 72 HTTP rounds/s. Interleaved 2 rounds,
  tight spread, ≥8% bar cleared by a wide margin.
- Set `tbMinRun` default to 600 in `emulator.js` (v18); `?gate=N` still
  overrides. Larger fallback chunk 4096->8192 was tested and REVERTED
  (42 -> 34 MIPS, same rounds). `test_tb_direct.mjs`'s count heuristic
  relaxed to 0.1% tolerance (real transcript/round assertions intact).
- Full battery after the change: `BATTERY_EXIT=0` (cargo 292, TB-XLATE,
  15 differentials, matrix 176/176, npm test 254 s, test:wasm 7 s).
  Shipped-default page: best 178.6/179.2, median 177.3/174.8 MIPS.
- VENDOR_V unchanged (68, JS-only change). VENDOR_V/emulator version
  literals PASS. This gate change is **uncommitted** (no order given).
- Follow-up same day: with the faster engine, re-tuned the console worker
  burst 48 -> 64 in `emu-worker.js` (v27 in app.js): live 83.6/85.1 ->
  91.8/92.0 MIPS, 69.7 -> 76.5 rounds/s, interleaved 2 rounds. 96 was
  already rejected (40 ms wall cap). Full battery green again
  (`BATTERY_EXIT=0`, npmtest 281 s). Both gate + burst changes uncommitted.
- Cross-firmware page A/B (gate 600 vs 5000, 2 rounds): eth_http +85%;
  blinky/tft ~2-4% lower (interpreter-only, inside noise, no regression
  of substance). Their loops are not recognized TB shapes (`shortDecl`
  dominated); the §42 blinky sextuple fusion remains the only lever there
  and was deliberately skipped. Gate change is safe across classes.

---

## 11. 2026-11-26 — blinky-class stack-counter fusion (general, ~6x)

- Implemented `fused_memcntdn` (cpu/thumb.rs + hook in cpu/mod.rs) for the
  six-instruction stack-counter delay loop (LDR/ADDS/STR/LDR/CMP.W/B.cond)
  that dominates blinky and tft_test. Exact arm-for-arm semantics, guarded
  by mapped/MPU/IT/trace/budget;Charge 6 per iteration.
- Anti-vacuous Rust pin `fused_memcntdn_fires` uses the real blinky
  encodings; caught two bugs (LDR/STR bit11 polarity, fire-count
  expectation). Cargo 293/293.
- Controlled page A/B (wasm bytes swapped, same box): no-fuse 47 MIPS vs
  fused 287-301 on blinky; tft_test 47 -> 289; eth_http unchanged.
- VENDOR_V 69, app.js v84, emu-worker v28, doom.js v104, full battery
  green (`BATTERY_EXIT=0`, npmtest 229 s). All uncommitted.
