# HANDOVER — TB (static block translation) share push, STM32F4 WASM emulator

**Read this file first.** It is the complete state of the TB work so a fresh
agent can continue without re-deriving anything.
Written 2026-10-20, at the end of the session that fixed the last outstanding
defect, tuned the live console loop, and took the full battery green.

---

## 0. TL;DR — where the work stands

| | State |
|---|---|
| Objective | Raise the share of traffic-firmware instructions executed as **translated blocks (TB)** in the browser page. Page-side MIPS is the ONLY primary metric (Node is diagnostic). |
| Committed | **S1** only (`0853e70`): `layout_version` + `region_offsets` (shared-memory foundation). |
| Uncommitted | **S2** (direct mode) + **S2.5** (IRQ predicate) + `emit_cond` LE fix + classifier/driver slice + hole-tolerant translation + widened driver windows + 2 skip-emitter fixes + the **§4 walk fix** + the **live-burst change** (`BURST_STEPS` 24 / `WALL_BUDGET_MS` 40) + the **submit fast path** (§13) + the **version guard** (`site/test_versions.mjs`, after two stale vendor literals in `emu-worker.js`) + the `?gate=` knob. |
| Last green battery | cargo **283/283** (`--test-threads=1`), one clean `npm test` **EXIT 0** (matrix 176/176, browser 44/44, tb_xlate 231 checks, qspi CDP, versions guard; `.pw-scratch/npmtest_final.log`), 17 TB differential harnesses PASS, `npm run test:wasm` EXIT 0 (`.pw
| **Active defect** | **None.** The RX-wait-loop validation failure is fixed (§4) and pinned by `site/test_tb_xlate.mjs` §3 (each of the three RX-loop entries is `new WebAssembly.Module`-validated). |
| Measured state | Page engine probe `eth_http`: **85–92 MIPS** with TB vs 33–43 `?notb=1` (≥60 bar met); the submit fast path alone was worth **+10% mean best-of-5** (3/3 interleaved rounds, §13). Live console page: **53–58 MIPS** and 44–48 HTTP rounds/s vs 31–40 and 26–34 interpreter (interleaved, quiet box). |
| Next move | Nothing is required to ship. Optional share push: §8 items 1–3; the submit-probe cost is now 1.3 µs (was 37–46), so the remaining TB-on time is interpreted chunks (§8 item 1) and the per-hit handshake. |

---

## 1. Hard rules (from the user — do not violate)

1. **Page-side browser MIPS is the only primary metric.** Node MIPS is
   diagnostic only. Never present a Node number as a result without its
   page-side counterpart (`.pw-scratch/cdp_mips.mjs`).
2. **Judge speed only by back-to-back / interleaved A/B on the same box
   state.** The box is shared and noisy (another tenant's headless Chrome).
   Single runs prove nothing; deltas under ~8% are noise.
3. **Every slice lands behind the full battery + a same-box A/B.**
4. **No commit of in-progress work without a separate user order.** The
   working tree is intentionally dirty. `git stash@{0}` (WIP vdpix in
   `cpu/thumb.rs`, `cpu/mod.rs`, `cpu/tests.rs`) must **not** be popped.
5. **Changes must be universal across firmware. Never modify firmware.**
6. The user authorized a full rewrite / static translation if needed; the
   page is already >80 MIPS on `eth_http` with TB default-on, the goal now
   is *share*, not raw MIPS.
7. Scratch lives in `.pw-scratch/` (keep logs + scripts, delete PNG dumps).
   Do not touch other tenants' background processes; never `pkill` blindly.

---

## 2. Architecture recap (what TB is)

The emulator has two execution paths:

* **Interpreter** — `stm32-periph-wasm/src/cpu/thumb.rs` (Rust Thumb-2
  decoder) + `cpu/mem.rs` (flat memory). Always correct, ~80 MIPS.
* **Translated blocks (TB)** — a *subset* of straight-line/loop Thumb is
  translated at runtime into a **WASM module** that runs against the
  model's own linear memory (direct mode) or a private mirror (mirror
  mode). The driver (`site/emulator.js`) compiles the module once, caches
  it, and submits runs with a fuel budget.

Flow (driver): pc → read `TB_LOOKAHEAD` halfwords → `tb_translate*_direct`
(Rust) → bytes → `new WebAssembly.Module` → instantiate → seed globals
(r0–r12, lr, xpsr, sp) → `run(fuel)` → read `tb_exit` kind + globals →
either continue, or handle kind 1 (miss) / kind 3 (resume) / kind 2
(fuel-out).

Exit kinds (global 10 = `tb_exit`, global 21 = `tb_pc`):

| kind | meaning | driver action |
|---|---|---|
| 0 | completed the region | continue at the fallthrough pc |
| 1 | miss (MMIO / unmapped / flash store) | direct mode: pc = `tb_pc` (faulting item), then `cpu.step(1)` (exactly ONE native instruction) and resubmit |
| 2 | fuel exhausted | resubmit with the remaining budget |
| 3 | resume exit (hole / true exit) | pc = `tb_addr`; a *hole* resumes at the unemittable item's own address |

Direct mode (shipped default) imports the model's exported linear memory
(`env.memory`); guest address → memory offset = `(addr - base) + ptr_offset`.
Flash segments carry bit31 = store-read-only, so a flash store misses out.

---

## 3. What is uncommitted (the S2+ stack) — quick map

**Rust (`stm32-periph-wasm/src/`)**

* `tb_xlate.rs` (the translator, ~4k lines) — the hot file. Key anchors:
  * `emit_body_walk` **line 274** — walks a loop body, emits items/exits/skips.
  * `macro_rules! close_skips` **line 324** — closes skip blocks whose target
    equals the current halfword.
  * skip preamble **line ~303-315** — opens one block per distinct internal-skip
    target, **outermost-first** (largest target first).
  * `classify_body` **line 2867** — validates a latched body, returns the
    internal-skip edges (`Skip { hw, cc, target_hw, k }`).
  * `item_starts_of` **line 2816** — the single source of truth for item
    boundaries (items-only walks desync at the first branch).
  * `translate_at_impl` **line 2974** — linear scan, latch selection, emission.
  * `translate_at_direct` **line 2806** — direct-mode entry.
  * `MAX_HOLES = 24` — unemittable items become "holes" (resume exits).
* `tb_emit.rs` — the WASM byte builder (`Code`).
* `cpu/mem.rs` — `region_offsets`, `layout_version`, contiguous-RAM fast path.
* `cpu/mod.rs` — `Cpu::pending_ready()` (the S2.5 IRQ predicate).
* `lib.rs` — exports: `tb_translate*`, `tb_translate*_direct`,
  `tb_translate_meta*`, `pending_exception_ready`, `region_offsets`,
  `layout_version`.

**JS (`site/emulator.js`)** — the driver. Anchors: `TB_LOOKAHEAD = 80`
(line 1499), `TB_BACK = TB_LOOKAHEAD*2 - 2` (1504), `TB_FWD = 64` (1505),
`TB_MIN_RUN = 5000` (1515, constant — the page-measured value, see §7),
`tbClass` / `tbBodyOk` / `tbFlowOk`,
`tbFindTop` / `tbMidLatch` / `tbTryLoop` / `tbTryEntry`, `tbExecDirect`
(with `tbMissPc`), `tbRun`. `TB_DIAG=1` (declaration ~1480, accessor ~2545)
is a **kept dev knob**: per-pc hit/decline attribution for
`.pw-scratch/tb_diag.mjs`, zero cost when unset (the per-submit `if (tbD)`
checks are null-cheap; it is not on the per-instruction path).

**In-repo pin** — `site/test_tb_xlate.mjs` (untracked, must be added):
condition-code sweep (14 cc × 16 NZCV) + the exact firmware spin
halfwords with exact instruction accounting + `WebAssembly.Module`
validation of the three RX-loop entries. **231 checks.** Wired into
`npm test` / `npm run test:unit`. **It is a silent-wrongness surface:
`emit_cond` must stay green.**

---

## 4. DEFECT — FIXED (RX-wait loop module failed WASM validation)

**Status: fixed and verified.** `emit_body_walk` now drains *every* skip/exit
starting at the current halfword (plus a trailing drain after the last item),
exactly as sketched below; `bhw` reaches the latch and `open` ends empty.
Verified by `TB_DIAG=1 node .pw-scratch/tb_diag.mjs 12 20000` (`eBuild 0`,
hits 699, translated 5.33M = 44% at gate 5000, kind3 8, missRetry 5) and
pinned in-repo by `site/test_tb_xlate.mjs` §3, which builds each of the three
RX-loop entries and runs `new WebAssembly.Module(bytes)` on it (231 checks).
Keeping the diagnosis below as the record of *why* the walk must drain
eagerly — it is the same class of bug as the `local_tee` leak.

### Symptom
The eth_http RX-wait loop (base `0x080008f8`) is the hottest shape in the
whole corpus (~42% of eth_http instructions live in `eth_recv_packet`'s
wait/re-arm, plus ~21% in the TX-done countdown + flag-poll loops). Its
translated module fails WASM validation:

```
unterminated control structure @+430      (entry 0x8000972, 2846 bytes)
```

Two of three preamble skip blocks (targets 47 and 61) never close, so the
loop assembler's `end loop` / `end $end` land on the leftover skip blocks
and the function ends one `end` short.

### Root cause (confirmed by instrumentation, this session)

`emit_body_walk` processes **at most one exit per item iteration** and then
falls through to emit "the next item" at whatever `bhw` now is:

```rust
for (it, size) in items.iter() {
    close_skips!();
    ...skip edge at bhw...
    if ei < exits.len() && exits[ei].hw == bhw { ...emit exit...; bhw += esize; }
    emit_item(c, it, ...);     // <-- assumes the exit advance landed exactly on this item
    partial += 1;
    bhw += size;
}
```

That assumption holds when branches and items alternate, but **holes
(consecutive unemittable items) break it**: the RX frame handler contains
runs of holes (hw 20, 21, 22 …). The walk drains only the FIRST hole of a
run, then emits the following item at the *wrong* address and `bhw` falls
behind by the sizes of the un-drained holes.

Instrumented trace (temporary `eprintln!`, since removed):

```
STEP start=0 bhw=17 size=2 exits_left=[(17,1,false), (20,1,true), (21,1,true)]
STEP start=0 bhw=20 size=2 exits_left=[(20,1,true), (21,1,true), (22,2,true)]
STEP start=0 bhw=23 size=2 exits_left=[(21,1,true), (22,2,true), (40,1,true)]   <-- 21,22 never drained
...
UNCLOSED start=0 bhw=54 open=[61, 47] items=30
```

* `bhw` ends at **54** but the latch is at **64** → the walk never reaches
  skip targets 47 and 61 → blocks stay open → module unterminated.
* The 10-halfword deficit is exactly the un-drained hole sizes
  (21:1 + 22:2 + 40:1 + 42:1 + 44:1 + 46:1 + 53:1 + 62:1 …).
* The trace also proves the emitted items after the first hole run are
  emitted at **wrong addresses** (`base_pc + bhw*2`), i.e. the module was
  not merely unterminated — it was mis-addressed (harmless only because
  those items are dead code on the hot path, but it must be fixed).

### Fix (applied — `emit_exit_arm` + `drain_one!()`)

In `emit_body_walk`, drain **every** branch/hole that starts at the current
halfword before emitting the next item:

```rust
for (it, size) in items.iter() {
    loop {
        close_skips!();
        let mut advanced = false;
        // skip edge at bhw (emits cond + br_if(depth), consumes its exits[] entry)
        if let Some(sk) = skips.iter().find(|sk| sk.hw == bhw) { ...; advanced = true; }
        // hole / true exit at bhw (emits the kind-3 arm)
        if ei < exits.len() && exits[ei].hw == bhw { ...; bhw += esize; advanced = true; }
        if !advanced { break; }
    }
    // bhw now sits exactly on this item's start
    ...slots / emit_item / partial += 1 / bhw += size;
}
// drain trailing exits (holes after the last item, before the latch)
loop { close_skips!(); if exit at bhw { emit; bhw += esize; continue; } break; }
```

Notes:
* Keep the accounting identical (partial / cycle / `k` formulas) — only the
  *number of exits drained per item* changes.
* A `debug_assert!(open.is_empty(), "unclosed skip blocks …")` sits at the
  end of `emit_body_walk` (release builds compile it out). It is the guard
  that would have caught this in debug builds — keep it.
* After the fix `bhw` reaches `li` (64) and `open` is empty.

### Regression gap that let this through (now closed)

`cargo test` only asserts `b.is_loop` and `b.consumed` — **never that the
emitted module validates**. Rust has no wasm validator here, so add the
check on the JS side:

```js
// site/test_tb_xlate.mjs (add)
const b = bindings.tb_translate_at_direct(ops, segs, base, base + 61*2);
new WebAssembly.Module(b);   // must not throw
```

Do it for the three entries of the eth_http RX loop: `0x8f8` (top),
`0x8f8 + 61*2` (mid, the cbnz target) and `0x8f8 + 5*2` (mid). This pins
both skip bugs (the `local_tee` leak and the lazy-nesting bug) and the
hole-drain bug.

---

## 5. Ground truth — the eth_http RX-wait loop (base `0x080008f8`)

Real firmware bytes, halfword map (from `.pw-scratch/eth_http.dis`):

```
 0 ldr.w r3,[ip,#0x618]     1 | 2 tst.w #2 | 4 bne -> 18
 5 lsls r3,r3,#31           6 bmi -> 47
 7 ubfx r3,lr,#0,#10        9 cbnz r3 -> 61   (hot ~1023/1024)
10 str r4,[r1,#8]  (MMIO 0x40029008 = DMARPDR re-arm, every ~1024 iters)
11 ldr.w | 13 tst.w | 15 add.w lr,#1 | 17 beq -> 5
18..46 cold frame handler (POP {r4-r7,pc} at 46)  <-- many holes
47 ldr r3,[r2,#0] | 48 lsls #24 | 49 bpl -> 47 (self-spin) | 50 str r0,[r2,#4] (MMIO)
51..59 tx-done stores/ubfx/cmp | 60 beq -> 10
61 add.w lr,#1 | 63 cmp lr,r5 | 64 bne -> 0   <-- THE LATCH (t = 0, top)
65..73 tail | 73+ literal pool
```

* Loop = 65 halfwords / 130 bytes. Latch at hw 64.
* Holes (unemittable) at hw 3, 14, 16, 20/21, 22, 39/40, 42, 44, 46, 53, 62
  — ~11-13 total, under `MAX_HOLES = 24`.
* The three internal skips: hw 4 → 18, hw 6 → 47, hw 9 → 61.
* Hot path: 0..9 → skip to 61 → 63,64 latch. It never touches the holes,
  which is exactly why hole-tolerant translation pays off.
* The loop re-arms DMARPDR (MMIO store at hw 10) every ~1024 iterations, so
  a translated run ends by a kind-1 miss about every ~10k instructions; the
  miss → `cpu.step(1)` path keeps the following iterations translated.

---

## 6. Decisions already taken (do not re-litigate)

1. **Direct TB mode** imports the model's exported `memory` (`env.memory`);
   region buffers never move (`layout_version` guards; any new `Vec` growth
   path MUST bump it or cached blocks write freed offsets).
2. **Direct miss (kind 1)** resumes at the faulting item's PC via global 21
   (`tb_pc`) — **no retry** (every preceding store is already committed).
   The driver records `tbMissPc`, runs **exactly one** native instruction
   (`cpu.step(1)` — never the 4096-inst chunk) and resubmits.
3. **TB default ON** in the console page (`tb: !params.has('notb')`);
   escapes `?notb=1` (interpreter only) and `?tbmirror=1` (mirror path).
4. **S2.5**: TB stays ON for interrupt firmware. The core exposes
   `pending_exception_ready()`; `tbRun` checks it before every submission
   and, when true, runs `cpu.step(1)` to take the exception exactly like
   the interpreter. (The S2.5 alternative — TB declines when
   `deliverIrqs` — broke `fpu_irq_test`, `eth_adv`, `eth_feat_pps`.)
5. **Host-scripted devices** (USB host, DCMI camera, ETH feat hooks) must
   advance **per STEP**, not per burst (`site/emu-worker.js`).
6. **Skip blocks are opened in a preamble**, one per distinct internal-skip
   target, outermost-first — lazy per-edge opening nests 61 inside 47
   inside 18, whose ends can never be reached LIFO.
7. **Skip condition uses `local_set`**, never `local_tee` (a tee leaks one
   wasm stack value per skip block → module validation failure).
8. **Hole-tolerant translation**: unemittable items become resume exits
   (kind 3, `tb_addr` = own PC, cost 0); the scan continues; reachability
   treats a hole as pass-through. Backward non-top edges are *evaluated*
   exits (cost 1), not resume exits.
9. **Top latch preferred** over nested; the first nested backward edge is
   kept as a restorable fallback.
10. **`item_starts_of()` is the only source of item boundaries.**
11. `emit_cond` is a **silent-wrongness surface** (the LE arm was wrong:
    it emitted GE instead of `Z || N!=V`; it collapsed 20000-iteration
    firmware spins into one iteration and broke `eth_feat_test`).
12. **`TB_MIN_RUN = 5000` is final.** Node says 600 is better (+5.5%,
    8/8 reps); the page says 600 is ~9% worse (interleaved best
    87.4/88.0 → 78.0/79.6). The page is the metric. The code comment
    carries the numbers so nobody "fixes" it back.
13. **Live console burst = 24 steps / 40 ms wall budget** (`site/emu-worker.js`).
    The 2026-10-10 "24 steps is neutral, revert to 12" verdict predates TB:
    the engine has since ~doubled, so a 12-step burst finishes in ~18 ms of
    a ~30 ms frame and the worker idles 35–40% of every cycle. Measured
    (window rate, interleaved): 12 → 39.7/41.5 MIPS, 24 → 50.7/54.5/57.5,
    32 → 47.8/58.6, 48 → 50.2 at 97% duty. Ship 24 (best mean, ~36 ms
    latency, same class as doom-worker's 28 ms).
14. **Depth-2 tick pipeline stays.** `?depth=4` measured 44.8/55.3 vs
    default 54.5/49.6 — no effect beyond noise.
15. **Do not enlarge the *step* size** to win MIPS: at 500k steps the MIPS
    meter reads 70 but HTTP rounds collapse 83 → 3 per 20M instructions
    (RX is serviced between steps). Real work is judged at the worker's
    100k cadence or by rounds/s.

---

## 7. Measured results so far (record, do not re-derive)

### Engine (page probe `.pw-scratch/cdp_mips.mjs`, rAF loop stopped, best-of-5)
* `eth_http` TB default: **best 85–91 / median 82–89** vs `?notb=1`
  **best 33–42 / median 29–41** → **2.1–2.7×**. The 60 MIPS page bar is met
  on heavy firmware; blinky is neutral (0 TB hits by construction).
  Final paired run on the shipped bytes (after the `TB_MIN_RUN` fold and the
  `?v=56` vendor literals, interleaved): TB **86.81 / 87.41 best, 85.32 /
  86.51 median** vs `--notb` **42.92 / 43.14 best, 42.81 / 41.74 median**;
  spot re-run after the SVD literal bump: **87.57 best / 86.21 median**.

### Live console page (`.pw-scratch/cdp_live_rate.mjs`, the configuration a
user actually runs — worker loop at 24-step bursts, window rate from `#stInst`,
interleaved)
* `eth_http` default: **53.2 / 57.6 MIPS**, **44.3 / 48.0** HTTP rounds/s.
* `eth_http ?notb=1`: **40.3 / 31.2 MIPS**, 33.6 / 26.1 rounds/s.
* Burst sweep (10 s windows, interleaved, `?burst=` knob during measurement):
  12 → 39.7/41.5; 24 → 50.7/54.5/57.5; 32 → 47.8/58.6; 48 → 50.2 (92–97%
  duty). Plateau is 24–32.
* Attribution of the old ceiling (temporary `WPROF` in `emu-worker.js`, since
  removed): per burst `step` was 18–19 ms of a ~30 ms cycle with
  `gather` 0.05 ms and `postMessage` 0.06 ms — **the state toll is negligible;
  the ceiling was duty cycle**, not engine speed or UI work.

### Gate (`.pw-scratch/tb_gate_ab.mjs`, 5–8 interleaved reps, 30M inst, 20k steps)
* Node: 5000 → 600 = **+5.5%** (35.26 → 40.17 median), share 44.5% → 52.4%,
  6/6–8/8 consistent.
* Page: 5000 → 600 = **~9% worse** (see §6.12). Gate stays 5000. The 600 win
  comes from eth_http RX-loop arms averaging 1.8–3.1k inst/hit — under the
  floor, and each hit pays the fixed sync/writeback cost.

### Per-hit / submit attribution (`TB_PROF`, 20M inst, 20k steps, since removed)
* Total TB-on time ≈ 500–540 ms: **interpreted chunks ~335 ms (62%)**,
  **submit path ~127 ms (24%)** across 4045 submits, per-hit pre+run ~23 ms
  (~6.6 µs/hit: `get` 0.40, `assign` 1.92, `read` 1.59, bulk 0.71,
  `tick_n` 1.30, `setpc` 0.15 µs).
* Submit split: `mem_read` 17 ms, cache select 2 ms (1926 cached / 822
  cached-declines), `tbTrMetaAt` 35 ms over 2534 slow-path submits (only 9
  produced loops), builds 4 ms. The rest is `tbFindTop`/`tbMidLatch` assists
  plus `tbTryEntry`'s mid-entry re-probe (its cache key `'top:'+pc` never
  matches the `tbTryLoop` fast path).
* At the worker's cadence (100k steps) submits are 5× rarer, so the submit
  path is ~5% — the live number is not submit-bound.

### Step-size × work (`20M inst`, `.pw-scratch/tb_stepsize.mjs`)
| steps | TB MIPS | rounds | tb-off MIPS | rounds |
|---|---|---|---|---|
| 20k | 39.98 | 83 | 33.75 | 83 |
| 100k | 67.22 | 16 | 39.13 | 16 |
| 500k | 70.01 | 3 | 39.96 | 3 |

Rows are **not comparable as speed**: round count collapses because RX is
serviced only between steps. TB is +18% wall on equal real work (same
rounds, same insts) at the 20k cadence, and the MIPS meter at large steps is
a spin-mix artifact.

### Census / other
* **PC census** (interpreter, 20k steps): ~42% of eth_http instructions in
  `eth_recv_packet` (RX wait / re-arm), ~21% in the TX-done countdown +
  flag-poll loops.
* **Miss-exit census**: device-space misses ~1 per 12M instructions on
  eth_http / blinky / ltdc / spi_flash.
* **DOOM**: TB share ~0.1% — chunk starts rarely land on loop tops; the
  memcpy/VDP loops are entered from straight-line code. **DOOM is not a TB
  target**; measure it with the interpreter
  (`.pw-scratch/doom_play_modes.mjs`). A VDP pixel-loop interpreter fusion
  regressed DOOM ~15% and lives in `git stash@{0}` — do not pop it.
* **Blinky**: 0 TB hits in every mode — interpreter-only by construction.

---

## 8. Remaining work (all optional — the slice is shippable)

1. **Mid-entry cache fast path.** `tbTryEntry` keys entries `'top:'+pc`, so a
   mid-body submit re-runs `tbTrMetaAt` (35 ms / 2534 submits at 20k steps).
   Fix: store `e.top` on assist-built entries so `tbTryLoop`'s
   `e.top === undefined && bytesEqual` path can hit, or keep a second cache
   keyed by exact pc+layout. Worth ~5% of Node submit time; unmeasured on the
   page (likely under the 8% bar at the page's larger steps).
2. **Page-granular "no loop here" decline bitmap** (flash code never
   changes, so a repeat probe costs ~0 instead of `mem_read` + compare +
   meta). Same 5% class.
3. **Share on DOOM** would need nested-loop fusion (census first) — not
   required by any current target.
4. **Commit on a separate user order.** Everything below is already done.


---

## 9. Pre-ship checklist

* [x] `TB_PROF` (per-hit attribution) and the worker's `WPROF` burst profiler
      removed from `site/emulator.js` / `site/emu-worker.js`; the temporary
      `?prof` / `?burst` / `?wallms` / `?depth` knobs removed from
      `site/app.js`.
* [x] `TB_MIN_RUN` folded into a constant (5000) with the page-vs-Node
      measurement in the comment; the env override is gone.
* [x] `TB_DIAG` **kept deliberately** as a documented dev knob
      (`.pw-scratch/tb_diag.mjs` needs it; it compiles to a null check per
      submit and is not on the per-instruction path). Declared in §3.
* [x] `debug_assert!(open.is_empty())` kept in `emit_body_walk`.
* [x] `site/test_tb_xlate.mjs` validates the three RX-loop entries with
      `new WebAssembly.Module` (231 checks, in `npm test`).
* [x] `?v=` bumped on every edited served file and on all `VENDOR_V`
      literals after the wasm rebuild: `VENDOR_V` **56**, `app.js?v=75`
      (console.html), `emulator.js?v=9` (app.js / emu-worker.js /
      doom-worker.js), `emu-worker.js?v=20`, `doom.js?v=97` (doom.html),
      `doom-worker.js?v=64`, `__doomVer` **96**. The SVD fetches share
      `VENDOR_V` too (`board.svd + '?v=56'` in app.js, `stm32f407.svd?v=56`
      in doom.js) — they were still at 49 and are now bumped with the rest
      as §0 requires; content is byte-identical, so this only re-fetches.
* [x] `site/vendor` **and** `stm32-periph-wasm/pkg` rebuilt from the same
      source (vendor 1779388 B, pkg 1901229 B).
* [x] `git add site/test_tb_xlate.mjs` (staged; commit on order).
* [ ] Restore the SVDs + `rm site/vendor/.gitignore` if any wasm rebuild
      is run again (`npm run build:wasm` does both automatically).


---

## 10. Gotchas that will bite again

* **wasm-pack wipes its out-dir**: `site/vendor` loses the SVDs and gains a
  `.gitignore` containing `*` (which silently untracks the vendor assets).
  Restore both after every rebuild.
* **JS bitwise vs comparison**: `tk && (m & 0x800) === 0` parses as
  `tk && (m & (0x800 === 0))` — always false. Parenthesize the whole
  comparison. Same family as the bridge-u32 bug: read u32 fields with
  `>>> 0`.
* **Bridge u32 fields** must be read `>>> 0` (0xDEADBEEF parses as a
  negative int32).
* **A decline costs a 4096-instruction interpreted chunk** (65536 once
  `tbDeclines >= 200`) — a decline-thrash looks like a speed collapse.
* **Fuel/amortization**: a 10-instruction loop body still runs ~2000
  iterations per `run()`, so a `TB_MIN_RUN` gate in the thousands need not
  exclude it. Bigger *step batches* are proven HARMFUL (MIPS flat, HTTP
  rounds 124 → 24) — never optimize by enlarging batches. Bigger *bursts*
  (steps per rAF tick) are the opposite: they are what removed the console's
  idle duty cycle (§6.13).
* **Two different "batch" knobs, do not confuse them**: `emu.step(n)` size
  (100k, RX-servicing granularity — keep) vs worker **burst** steps per tick
  (24, pure throughput — tune). MIPS at large `step(n)` is a spin-mix
  artifact; judge real work by rounds/s.
* **Single runs prove nothing on this box**, however large the delta. Two
  adjacent A/B/A runs minimum, interleaved, same session. The box has other
  tenants' headless Chrome.
* **Measuring the live page**: `#stMips` is a 0.5 s window refreshed only on
  UI frames — sample it repeatedly and take the median, or compute the
  window rate from `#stInst` (`.pw-scratch/cdp_live_rate.mjs` does both).
  A single `#stMips` read can be 20 while the window is 55.
* **SysTick cadence under TB**: TB checks `pending_exception_ready()` per
  translated run, the interpreter once per JS step, so a guest that measures
  tick cadence inside one step sees a ~3.7% shift (freertos' TIM2 line).
  `.pw-scratch/tb_irq_diff.mjs` normalizes exactly that one line.
* **Melt-crossing / rAF cadence** is unrelated to TB — do not "fix" slowness
  by chasing backlog.
* **`.pw-scratch/wasm_struct.py`** is the structural WASM checker used to
  localize emitter imbalances (add frame ids if you extend it).

---

## 11. Key files

| Path | What |
|---|---|
| `stm32-periph-wasm/src/tb_xlate.rs` | the translator (hot file) |
| `stm32-periph-wasm/src/tb_emit.rs` | WASM byte builder |
| `stm32-periph-wasm/src/cpu/thumb.rs` | interpreter ground truth (CBZ ~1836, TST via `alu_op`/`rd_is_test`, UBFX ~2240) |
| `stm32-periph-wasm/src/cpu/mem.rs` | flat memory, `region_offsets`, `layout_version` |
| `stm32-periph-wasm/src/lib.rs` | wasm exports (`tb_translate*_direct`, `pending_exception_ready`, …) |
| `site/emulator.js` | the TB driver (+ temporary `TB_DIAG` block to remove) |
| `site/emu-worker.js` | page-side worker (host-scripted devices per step) |
| `site/test_tb_xlate.mjs` | in-repo TB pin (cond sweep + firmware spin + RX-loop module validation) |
| `site/test_versions.mjs` | served-asset `?v=` guard (20 refs / 10 groups, vendor group pinned together) |
| `.pw-scratch/emulator_{A,B}.js` + `chain_fast_ab.sh` / `chain_fast_live.sh` | the submit-fast-path A/B (A = pre-change driver; swap by copying over `site/emulator.js`) |
| `.pw-scratch/tb_diag.mjs` | per-pc hits/declines attribution, dumps `bad_mid.wasm` |
| `.pw-scratch/tb_gate_ab.mjs` | interleaved `TB_MIN_RUN` gate A/B |
| `.pw-scratch/tb_eth_wall.mjs` / `tb_eth_interleave.mjs` | traffic A/B harnesses |
| `.pw-scratch/test_tb_direct.mjs` | byte-identical transcript acceptance (off / mirror / direct) |
| `.pw-scratch/tb_outer_probe.mjs` | entry/window sweep at base `0x8f8` |
| `.pw-scratch/cdp_mips.mjs` | **the** page-side MIPS probe (engine acceptance metric) |
| `.pw-scratch/cdp_live_rate.mjs` | live console-page rate probe (window MIPS from `#stInst`, `#stMips` median, rounds/s) |
| `.pw-scratch/tb_floor.mjs` | per-hit register-handshake microbench (6.6 µs/hit) |
| `.pw-scratch/tb_stepsize.mjs` | step-size × TB census (the rounds-collapse table) |
| `.pw-scratch/battery_final.sh` / `.verdict` | the one-shot final battery |
| `.pw-scratch/eth_http.dis` | objdump of `eth_http.elf` (ground truth for §5) |
| `.pw-scratch/wasm_struct.py` | structural WASM checker |
| `.pw-scratch/doom_play_modes.mjs` | the honest DOOM pill harness |

---

## 12. One-paragraph state summary

TB (static block translation of Thumb-2 into WASM modules that run against
the model's own linear memory) is implemented end to end, shipped default-on
for the console page, and **green**: cargo 283/283, `TB=1` board matrix
176/176, 17 TB differential harnesses, the in-repo 231-check pin (including
`WebAssembly.Module` validation of the three eth_http RX-wait-loop entries
that used to fail), 44/44 browser presets, `npm test` and `test:wasm` all
EXIT 0. On the page it is **2.1–2.7× the interpreter on heavy firmware**
(85–91 vs 33–42 MIPS best-of-5, `eth_http`, the ≥60 bar) with a
byte-identical UART transcript, and the live console — after raising the
worker burst to 24 steps / 40 ms — runs **53–58 MIPS and 44–48 HTTP
rounds/s** versus 31–40 MIPS and 26–34 rounds/s interpreted (interleaved,
same box). The submission path was then attacked with the new cost-split
instrument and a zero-copy submit cache (§13): per-submit probe **46.3 →
1.3 µs**, lookup/interp ratio 0.333 → 0.147, page best-of-5 **+10% mean**
(3/3 interleaved rounds, `eth_http` 82.5–92.4 vs 75.8–86.2), live console
+2.5% (duty-bound, noise). The remaining known costs are measured and
small: interpreted chunks are ~77% of TB-on time at 500k steps (TB share,
not overhead) and the per-hit register handshake 6.6 µs. Two hygiene items
landed alongside: `site/test_versions.mjs` (a stale `?v=55` pair in
`emu-worker.js` had been running the previous wasm against the current
glue) and the `?gate=` / `tbMinRun` knob (verdict: keep 5000; its A/B was
load-contaminated). Nothing is blocked; the tree is intentionally
uncommitted and waiting on an explicit commit order.

---

## 13. Submit fast path (this session's slice — engine overhead, not share)

`TB_DIAG`'s cost split (`TB_DIAG=1 node .pw-scratch/tb_diag.mjs 20 500000`,
units are **ms**) showed the submit probe at **26–30% of wall** and ~3.5x the
translated run it looked up: each submit paid a 160-byte guest `mem_read`
(~6.6 µs), a `bytes.slice()` for the u16 view, the MPU/IT gates, a meta
probe (~16.5 µs) and a back-scan — and mid-body entries paid a **second**
read + meta at the top, because entries are cached under `'top:'+top` while
submits are keyed by `pc` (`cacheHits 883/2460`).

`site/emulator.js` now keeps `tbFast: pc → {e, base, meta, bytes, off}`:

- zero-copy validation: direct mode already imports the model's `Memory`, so
  `tbMemView()` returns a `Uint8Array` over `tbMem.buffer` (rebuilt only when
  the buffer object changes) and `tbGuestOff()` maps a guest window to a wasm
  offset via the region table (`list` entries now carry `off`); the check is a
  160-byte compare against the copy the entry was built from;
- the decline gates (`declined`, `tbShort`) still run per submit, so
  re-probe/permanent-decline accounting is unchanged;
- `tbSegsFor()` runs **before** the map read (that is what notices a
  `layout_version` bump and clears `tbCache` + `tbFast`), so an entry whose
  baked offsets died can never execute;
- mirror mode never populates the map, `TB_NOMETACACHE` disables it, and the
  wasm is untouched (no `VENDOR_V` bump — only `emulator.js?v=` 9 → 10).

Numbers: probe per submit **46.3 → 1.3 µs** (2,414 fast hits / 143 slow
submits, `cacheHits 0/2460`), lookup/interp **0.333 → 0.147**; page
best-of-5 `eth_http` **A 75.76/80.65/86.21 vs B 82.51/92.42/92.25**
(+9/+15/+7%, 3/3 rounds); live console +2.5% mean (noise; the loop is
duty-bound). Battery green: cargo 283/283, TB diffs + 231-check pin, `TB=1`
matrix 176/176, browser 44/44, qspi CDP / npm demo / `test:wasm` EXIT 0,
`test_tb_direct.mjs 20` byte-identical with translated counts unchanged
(12,771,547).

Two other things recorded this session: `site/test_versions.mjs` (two stale
`?v=55` vendor literals in `emu-worker.js` had been running the previous
wasm against the current glue — nothing failed, hence the guard), and
`TB_MIN_RUN` stays **5000** (its page A/B was contaminated by load drift:
the first slot won every round while the absolute level decayed 61 → 39).
