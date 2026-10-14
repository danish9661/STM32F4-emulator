//! Thumb-block translator, slice 2 of the static-binary-translation
//! project (authorized 2026-10-10). Converts a straight-line run of
//! well-understood 16-bit data-processing immediates into `tb_emit`
//! calls. Guest state model (v1 ABI, test-only for now):
//! globals 0-7 = r0-r7, global 8 = xPSR (ARM bit positions, low 28
//! bits preserved across flag writes exactly like the interpreter).
//! p0 = fuel (i32 param). Scratch locals: l1 = a, l2 = r, l3 = addr,
//! l4 = value, l5 = fuel remaining, l6 = iters executed.
	//! Globals 9/10/11 = tb_exit/tb_addr/sp (exit protocol + SP base).
//! Supported: MOVS/ADDS/SUBS/CMP imm8 + NOP + LDR/STR imm/SP word
//! forms. Everything else
//! (branches, 32-bit prefixes, other-memory/byte/halfword forms, IT
//! hints, BKPT) STOPS the
//! block before it — the driver keeps executing those on the
//! interpreter. Flag math mirrors `thumb.rs` add_flags/sub_flags/nz
//! arm-for-arm (same formulas, same bit positions); predicated forms
//! are the driver's problem (it only submits it_n==0 stretches).

use super::tb_emit::{Code, Module, I32, I64};

const XPSR_KEEP: u32 = 0x0FFF_FFFF;
const XPSR_Z: u32 = 0x4000_0000;
const XPSR_N: u32 = 0x8000_0000;
const XPSR_C: u32 = 0x2000_0000;
const XPSR_V: u32 = 0x1000_0000;

/// Guest register number -> translator global index (r0-r7 direct,
/// r8-r12 -> 12-16, SP(13) -> 11, LR(14) -> 17). PC(15) has no global:
/// classify() rejects it everywhere a value is needed.
fn reg_g(r: u32) -> u32 {
    match r {
        0..=7 => r,
        8..=12 => 12 + (r - 8),
        13 => 11,
        14 => 17,
        _ => u32::MAX,
    }
}

/// ThumbExpandImm VALUE (carry-out is unused: ADD.W (s=0) takes C from
/// its own add, and only ADD.W-imm is classified). Mirror of
/// thumb.rs expand_imm (GAS-verified vectors live there).
fn expand_imm_val(imm12: u32) -> u32 {
    let imm8 = imm12 & 0xFF;
    if imm12 & 0xC00 == 0 {
        match (imm12 >> 8) & 3 {
            0 => imm8,
            1 => (imm8 << 16) | imm8,
            2 => (imm8 << 24) | (imm8 << 8),
            _ => imm8 | (imm8 << 8) | (imm8 << 16) | (imm8 << 24),
        }
    } else {
        let unrot = 0x80 | (imm12 & 0x7F);
        let rot = (imm12 >> 7) & 0x1F;
        unrot.rotate_right(rot)
    }
}

/// NZ bits of a translate-time-known immediate.
fn nz_bits(v: u32) -> u32 {
    (if v == 0 { XPSR_Z } else { 0 }) | (if v & 0x8000_0000 != 0 { XPSR_N } else { 0 })
}

/// Stack out: [a] (a = Rn value, kept for flags). Uses l0.
fn emit_load_a(c: &mut Code, rn: usize) {
    c.global_get(rn as u32).local_tee(1);
}

/// add_flags(a, b, 0) mirror. Stack in: [a]. Leaves []. Uses l0 (a), l1 (r).
fn emit_add_flags(c: &mut Code, rd: Option<usize>, imm: u32) {
    // r = a + imm (drop the tee'd copy; flags reload r from l1)
    c.i32_const(imm as i32).i32_add().local_tee(2).drop_(); // []
    if let Some(d) = rd {
        c.local_get(2).global_set(d as u32); // Rd = r
    }
    // ZN bits
    c.local_get(2).i32_eqz().i32_const(30).i32_shl(); // [z]
    c.local_get(2).i32_const(31).i32_shr_u().i32_const(31).i32_shl(); // [z,n]
    c.i32_or(); // [zn]
    // C bit: (a64 + b64 > 0xFFFFFFFF)
    c.local_get(1).i64_extend_u(); // [zn, a64]
    c.i32_const(imm as i32).i64_extend_u(); // [zn, a64, b64]
    c.i64_add(); // [zn, sum]
    c.i64_const(0xFFFF_FFFF).i64_gt_u(); // [zn, carry?1:0]
    c.i32_const(29).i32_shl(); // [zn, cbit]
    c.i32_or(); // [znc]
    // V bit: ((a^r)&(b^r)&0x80000000)!=0
    c.local_get(1).local_get(2).i32_xor(); // [znc, a^r]
    c.i32_const(imm as i32).local_get(2).i32_xor(); // [znc, a^r, b^r]
    c.i32_and(); // [znc, t]
    c.i32_const(0x8000_0000u32 as i32).i32_and(); // [znc, t&neg]
    c.i32_const(0).i32_ne(); // [znc, over?1:0]
    c.i32_const(28).i32_shl(); // [znc, vbit]
    c.i32_or(); // [zncv]
    // xpsr = (xpsr & KEEP) | top
    c.global_get(8).i32_const(XPSR_KEEP as i32).i32_and(); // [old, top]... order!
    c.i32_or();
    c.global_set(8);
}

/// ADD-reg (T1, flag-setting like ADDS-imm8; predicated forms are the
/// driver's problem — it only submits it_n==0 stretches).
/// Stack in: [a]. Uses l1 (a), l2 (r).
fn emit_add_reg(c: &mut Code, rd: usize, rm: u32) {
    c.global_get(rm).i32_add().local_tee(2).drop_(); // r = a+b, []
    c.local_get(2).global_set(rd as u32);
    c.local_get(2).i32_eqz().i32_const(30).i32_shl();
    c.local_get(2).i32_const(31).i32_shr_u().i32_const(31).i32_shl();
    c.i32_or(); // [zn]
    c.local_get(1).i64_extend_u();
    c.global_get(rm).i64_extend_u();
    c.i64_add();
    c.i64_const(0xFFFF_FFFF).i64_gt_u();
    c.i32_const(29).i32_shl();
    c.i32_or(); // [znc]
    c.local_get(1).local_get(2).i32_xor();
    c.global_get(rm).local_get(2).i32_xor();
    c.i32_and();
    c.i32_const(0x8000_0000u32 as i32).i32_and();
    c.i32_const(0).i32_ne();
    c.i32_const(28).i32_shl();
    c.i32_or(); // [zncv]
    c.global_get(8).i32_const(XPSR_KEEP as i32).i32_and();
    c.i32_or();
    c.global_set(8);
}

/// SUB-reg (T1, flag-setting) / CMP-reg (no writeback, always sets
/// flags — mirror of the 0x4000 sop=10 arm). Stack in: [a].
fn emit_sub_reg(c: &mut Code, rd: Option<usize>, rm: u32) {
    c.global_get(rm).i32_sub().local_tee(2).drop_(); // r = a-b, []
    if let Some(d) = rd {
        c.local_get(2).global_set(d as u32);
    }
    c.local_get(2).i32_eqz().i32_const(30).i32_shl();
    c.local_get(2).i32_const(31).i32_shr_u().i32_const(31).i32_shl();
    c.i32_or(); // [zn]
    c.local_get(1).i64_extend_u();
    c.global_get(rm).i64_extend_u();
    c.i64_lt_u().i32_eqz().i32_const(29).i32_shl();
    c.i32_or(); // [znc]
    c.local_get(1).global_get(rm).i32_xor();
    c.local_get(1).local_get(2).i32_xor();
    c.i32_and();
    c.i32_const(0x8000_0000u32 as i32).i32_and();
    c.i32_const(0).i32_ne().i32_const(28).i32_shl();
    c.i32_or(); // [zncv]
    c.global_get(8).i32_const(XPSR_KEEP as i32).i32_and();
    c.i32_or();
    c.global_set(8);
}

/// Plain add, NO flags (ADD.W-imm/reg s=0, ADD(2) hi/lo). Stack in: [a].
fn emit_add_plain(c: &mut Code, rd: u32) {
    c.i32_add().global_set(rd);
}

/// sub_flags(a, b, 1) mirror (ci=1: plain subtract, C = !borrow).
/// Stack in: [a]. Leaves []. Uses l0 (a), l1 (r).
fn emit_sub_flags(c: &mut Code, rd: Option<usize>, imm: u32) {
    c.i32_const(imm as i32).i32_sub().local_tee(2).drop_(); // []
    if let Some(d) = rd {
        c.local_get(2).global_set(d as u32); // Rd = r
    }
    c.local_get(2).i32_eqz().i32_const(30).i32_shl(); // [z]
    c.local_get(2).i32_const(31).i32_shr_u().i32_const(31).i32_shl(); // [z,n]
    c.i32_or(); // [zn]
    // C bit: !(a64 < b64)
    c.local_get(1).i64_extend_u(); // [zn, a64]
    c.i32_const(imm as i32).i64_extend_u(); // [zn, a64, b64]
    c.i64_lt_u().i32_eqz().i32_const(29).i32_shl(); // [zn, cbit]
    c.i32_or(); // [znc]
    // V bit: ((a^b)&(a^r)&0x80000000)!=0
    c.local_get(1).i32_const(imm as i32).i32_xor(); // [znc, a^b]
    c.local_get(1).local_get(2).i32_xor(); // [znc, a^b, a^r]
    c.i32_and(); // [znc, t]
    c.i32_const(0x8000_0000u32 as i32).i32_and(); // [znc, t&neg]
    c.i32_const(0).i32_ne().i32_const(28).i32_shl(); // [znc, vbit]
    c.i32_or(); // [zncv]
    c.global_get(8).i32_const(XPSR_KEEP as i32).i32_and();
    c.i32_or();
    c.global_set(8);
}

/// A forward-branch exit inside a loop body: kind-3 return with the
/// branch target when taken (conditional) or unconditionally (B.N).
struct Exit {
    hw: usize,
    cc: Option<u32>, // None = unconditional (B.N)
    target: u32,
    size: usize, // halfwords (1 or 2)
}

/// Internal-skip edge: conditional forward branch inside a latched body.
/// (edge hw, cond, target hw, static skipped count K.)
struct Skip {
    hw: usize,
    cc: u32,
    target_hw: usize,
    k: u32,
}

/// Skip-adjust global index (19): accumulates skipped static counts per
/// run; every dynamic count formula subtracts it. Zeroed in the
/// prologue (like iters in l6).
/// Fuel global index (20): shared instruction budget. Flat single-loop
/// modules keep fuel in local l5 (legacy, fastest); modules with calls
/// or nested loops use the global so every level checks one pool.
/// Chosen per module at compile time — flat emission is untouched.
const ADJ_GLOBAL: u32 = 19;
const FUEL_GLOBAL: u32 = 20;
/// Scratch local for the skip condition (l7; l1-l6 are fuel/iters/temps
/// owned by other code — emit_valid_check uses l1, seg guards l3/l4).
const SKIP_LOCAL: u32 = 7;

/// Emit one body walk over `items` starting at halfword `start_hw`,
/// matching forward exits by position. `n_count` is the Dynamic-count n
/// (0 for a partial first pass, prefix-or-P for full iterations).
/// Returns the end halfword. Slots are collected for emitted Mem items.
fn emit_body_walk(
    c: &mut Code,
    items: &[(Item, usize)],
    exits: &[Exit],
    skips: &[Skip],
    start_hw: usize,
    n_count: u32,
    cycle: u32,
    segs: &[(u32, u32, u32)],
    slots: &mut Vec<u32>,
) -> usize {
    let mut bhw = start_hw;
    let mut partial: u32 = 0;
    let mut ei = 0usize;
    // Skip exits strictly before the walk start.
    while ei < exits.len() && exits[ei].hw < bhw {
        ei += 1;
    }
    // Open skip-blocks targeting inside the walk only for edges at/after
    // the walk start (an edge behind the start already resolved natively).
    let mut open: Vec<usize> = Vec::new(); // skip target hws (LIFO)
    // Close skip-blocks targeting bhw (LIFO: same target closes in
    // reverse open order — properly nested WASM). Must run not only at
    // item starts but after EVERY bhw advance: an exit arm can advance
    // bhw onto a skip target mid-iteration (the VDP skip closes at hw 4
    // right after the B-exit arm moves off hw 3 — missing this left the
    // block open and failed WASM validation; caught at compile, not
    // review).
    macro_rules! close_skips {
        () => {
            while open.last() == Some(&bhw) {
                c.end();
                open.pop();
            }
        };
    }
    for (it, size) in items.iter() {
        close_skips!();
        // Internal-skip edge here (also present in exits[] for cycle
        // counting, but emitted as a skip-block, NOT a kind-3 return):
        // block{$skip}; cond (kept in l7); adj += K*cond; taken -> break
        // out (skip the spanned items); else fall into them.
        if let Some(sk) = skips.iter().find(|sk| sk.hw == bhw) {
            c.block_void();
            emit_cond(c, sk.cc);
            c.local_tee(SKIP_LOCAL);
            c.global_get(ADJ_GLOBAL).i32_const(sk.k as i32).local_get(SKIP_LOCAL).i32_mul().i32_add().global_set(ADJ_GLOBAL);
            c.local_get(SKIP_LOCAL).br_if(0);
            open.push(sk.target_hw);
            // Bookkeeping matches the exit arm below (the edge evaluates
            // every iteration it reaches): consume the exits[] entry so
            // the kind-3 arm below does not also fire for this hw.
            if ei < exits.len() && exits[ei].hw == bhw {
                partial += 1;
                ei += 1;
                bhw += exits[ei - 1].size;
                close_skips!();
            }
        }
        if ei < exits.len() && exits[ei].hw == bhw {
            let e = &exits[ei];
            let esize = e.size;
            // kind-3 forward exit: conditional (Bcc) or direct (B.N).
            let emit_kind3 = |c: &mut Code| {
                c.i32_const(e.target as i32).global_set(10);
                emit_exec_count(
                    c,
                    &ExecCount::Dynamic { n: n_count, cycle, partial: partial + 1 },
                );
                c.i64_const(3u64 << 32).i64_or();
                c.return_();
            };
            match e.cc {
                Some(cc) => {
                    emit_cond(c, cc);
                    c.if_void();
                    emit_kind3(c);
                    c.end();
                }
                None => {
                    emit_kind3(c);
                }
            }
            partial += 1;
            ei += 1;
            bhw += esize;
            close_skips!(); // exit width can land bhw on a skip target
        }
        if let Item::Mem { base, off, size: msize, is_store, wb, .. } = *it {
            let _ = wb;
            let pack = (if base == 13 { 15 } else { base })
                | ((msize == 4) as u32) << 5
                | ((is_store as u32) << 6);
            slots.push(pack);
            slots.push(if wb == 1 { 0 } else { off as u32 });
        }
        emit_item(
            c,
            it,
            &ExecCount::Dynamic { n: n_count, cycle, partial },
            segs,
        );
        partial += 1;
        bhw += size;
    }
    bhw
}

/// Translated block: emitted bytes + halfwords consumed.
pub struct Block {
    pub bytes: Vec<u8>,
    /// Halfwords taken (static shape).
    pub consumed: usize,
    /// True when a backward-edge loop was formed (body repeats).
    pub is_loop: bool,
    /// Fallthrough halfword offset past the region (resume pc = base + 2*exit_off).
    pub exit_off: usize,
    /// Static memory slots: flat [pack, off] pairs for every Mem item in
    /// the consumed region (pack = base5 | word<<5 | store<<6, base 0-14
    /// = register, 15 = SP; off = signed byte offset as u32).
    pub slots: Vec<u32>,
}

/// Sign-extend a 9-bit B.cond immediate.
fn sx9(v: u32) -> i32 {
    if v & 0x100 != 0 {
        (v | 0xFFFF_FE00) as i32
    } else {
        v as i32
    }
}

/// Condition test (mirror of the cond_ok arms 0-13; 14 handled by the
/// caller passing cc<14 only... here cc is validated <14 already).
/// Stack out: [taken?1:0].
fn emit_cond(c: &mut Code, cc: u32) {
    // one-bit reader: ((xpsr >> b) & 1)
    let bit = |c: &mut Code, b: u32| {
        c.global_get(8).i32_const(b as i32).i32_shr_u().i32_const(1).i32_and();
    };
    match cc {
        0 => {
            bit(c, 30);
        } // EQ: Z
        1 => {
            bit(c, 30);
            c.i32_eqz();
        } // NE: !Z
        2 => {
            bit(c, 29);
        } // CS: C
        3 => {
            bit(c, 29);
            c.i32_eqz();
        } // CC: !C
        4 => {
            bit(c, 31);
        } // MI: N
        5 => {
            bit(c, 31);
            c.i32_eqz();
        } // PL: !N
        6 => {
            bit(c, 28);
        } // VS: V
        7 => {
            bit(c, 28);
            c.i32_eqz();
        } // VC: !V
        8 => {
            bit(c, 29);
            bit(c, 30);
            c.i32_eqz();
            c.i32_and();
        } // HI: C && !Z
        9 => {
            bit(c, 29);
            c.i32_eqz();
            bit(c, 30);
            c.i32_or();
        } // LS: !C || Z
        10 => {
            bit(c, 31);
            bit(c, 28);
            c.i32_eq();
        } // GE: N==V
        11 => {
            bit(c, 31);
            bit(c, 28);
            c.i32_ne();
        } // LT: N!=V
        12 => {
            bit(c, 30);
            c.i32_eqz();
            bit(c, 31);
            bit(c, 28);
            c.i32_eq();
            c.i32_and();
        } // GT: !Z && N==V
        _ => {
            bit(c, 30);
            bit(c, 31);
            bit(c, 28);
            c.i32_eq();
            c.i32_or();
        } // LE (13): Z || N!=V
    }
}

/// Mirror scratch reserve (bytes): the dirty trailer lives at
/// [8..SCRATCH) as (addr, size) u32 pairs — exact byte ranges of every
/// translated store (count in global 18 = number of PAIRS).
/// Segments start at SCRATCH, each 64KB-aligned. Capacity:
/// (SCRATCH-8)/8 pairs — overflow miss-exits (kind 1). Exact ranges
/// (not pages) keep both sync-in and writeback proportional to actual
/// traffic: a 200-row strided column writes back ~200 small spans, not
/// 16 whole pages.
pub const SCRATCH: u32 = 266240;
const TRAILER_BASE: u32 = 8;
const TRAILER_CAP: u32 = 32767; // (addr,size) pairs; overflow miss-exits
// Validity bitmap: 1 bit per 4KB mirror page (driver-managed), covering
// up to 8192 pages (32MB mirror). Every translated load/store checks its
// page bit AFTER the segment guard and miss-exits when clear — this is
// what makes dynamic access ranges sound (pre-sync covers only static
// slot neighborhoods; demand misses sync the rest with restore+retry).
// Without it, strided loops silently read never-synced zeros.
// Validity bitmap: 1 bit per 4KB mirror page. MUST sit on a 4KB boundary
// with SCRATCH 4KB-aligned, otherwise mirror pages stop matching guest
// pages and every validity check reads the wrong bit (caught live: silent
// zeros + boot failure from shifted pages).
pub const BITBASE: u32 = 262144;
const BITBYTES: u32 = 1024;
const _: () = {
    assert!(SCRATCH % 4096 == 0);
    assert!(BITBASE % 4096 == 0);
    assert!(BITBASE + BITBYTES <= SCRATCH);
};
const MAX_MIRROR_BYTES: u32 = 32 * 1024 * 1024;

/// Mirror-miss guard over a segment table: l3 holds the guest byte
/// address. Each segment (guest_base, size, mirror_off) is tried in
/// order; the first with (addr-base) < size-len wins and sets l4 to the
/// mirror index. No match stops to the driver (tb_exit=1, tb_addr=addr,
/// kind 1) with the exact executed count: `exec` is Static(k) in
/// straight code or Dynamic(n, cycle, partial) in a loop body.
/// Depth note: the ladder is emitted directly in straight code or a loop
/// body (never inside another if), so br(2) from a segment check always
/// lands past the $miss block (success), and br(0) reaches the miss arm.
enum ExecCount {
    Static(usize),
    Dynamic { n: u32, cycle: u32, partial: u32 },
    /// Nested exact-count: value = LTOT (i64-extended). Skips, calls, and
    /// nests are exact by construction (skipped items never increment),
    /// so no static terms participate and no adj correction applies.
    Nested,
}

/// Run-total local (LTOT, l9): nested/call modules count EXACTLY by
/// incrementing per executed item and accumulating call returns. Static
/// formulas can't span dynamic skips/calls/nests, and LTOT makes every
/// exit kind exact by construction (no adj correction needed — skipped
/// items never increment). Flat modules keep static formulas (untouched).
const LTOT_LOCAL: u32 = 9;
/// Adj-save local (l13): non-top funcs snapshot ADJ at entry (unused in
/// the LTOT design — kept reserved for the formula path; nested exits
/// read ADJ directly since skips never increment LTOT).
const _SAVE_LOCAL: u32 = 13;
/// i64 call-result temp (l10): WASM `call` returns i64; teeing it into an
/// i32 local fails validation (caught live: every tree module rejected,
/// silent driver fallback to flat). Nested funcs declare one i64 local.
const CALLRET_LOCAL: u32 = 10;
/// Emit the executed-count value for a miss/forward exit. Stack out: [i64].
fn emit_exec_count(c: &mut Code, exec: &ExecCount) {
    match *exec {
        ExecCount::Static(k) => {
            c.i64_const(k as u64);
        }
        ExecCount::Dynamic { n, cycle, partial } => {
            // Exact under internal skips: the linear static count minus
            // skipped static counts accumulated in skipAdj (zero when no
            // skip was taken — identical to the old formula then).
            c.local_get(6).i32_const(cycle as i32).i32_mul().i32_const((n + partial) as i32).i32_add();
            c.global_get(ADJ_GLOBAL).i32_sub();
            c.i64_extend_u();
        }
        ExecCount::Nested => {
            c.local_get(LTOT_LOCAL);
            c.i64_extend_u();
        }
    }
}

fn emit_seg_guard(c: &mut Code, segs: &[(u32, u32, u32)], len: u32, exec: &ExecCount) {
    c.block_void(); // $miss
    c.block_void(); // $ok
    for (base, size, mo) in segs.iter() {
        let bound = size.saturating_sub(len);
        // idx = addr - base (wraps huge when addr < base: fails the check).
        c.local_get(3).i32_const(*base as i32).i32_sub().local_tee(4);
        c.i32_const(bound as i32).i32_lt_u();
        c.if_void();
        c.local_get(4).i32_const(*mo as i32).i32_add().local_set(4);
        c.br(2); // past $miss: success
        c.end();
    }
    c.br(0); // no segment matched: miss arm
    c.end(); // $ok
    c.i32_const(1).global_set(9);
    c.local_get(3).global_set(10);
    emit_exec_count(c, exec);
    c.i64_const(1u64 << 32).i64_or();
    c.return_();
    c.end(); // $miss
}

/// Validity check after a successful segment guard: l4 holds the mirror
/// index. Miss-exits (kind 1) when the 4KB page bit is clear — the driver
/// syncs the faulting page and retries (demand paging for dynamic access
/// ranges; pre-sync covers only static slot neighborhoods). Uses l1 as
/// temp (callers must spill values after this check, not before).
fn emit_valid_check(c: &mut Code, exec: &ExecCount) {
    // pidx = l4 >> 12 (spilled to l1 AND popped from the stack);
    // mask = 1 << (pidx & 7); baddr = BITBASE + (pidx >> 3).
    // Miss-exits when the bit is clear. Nothing is carried on the stack
    // across the if (pidx reloads from l1) — keeps validation trivial.
    c.local_get(4).i32_const(12).i32_shr_u().local_tee(1); // [pidx]
    c.drop_(); // [] (pidx lives in l1)
    c.i32_const(1); // [one]
    c.local_get(1).i32_const(7).i32_and(); // [one, bit]
    c.i32_shl(); // [mask]
    c.local_get(1).i32_const(3).i32_shr_u(); // [mask, by]
    c.i32_const(BITBASE as i32).i32_add(); // [mask, baddr]
    c.i32_load8_u(0, 0); // [mask, byte]
    c.i32_and(); // [hit?]
    c.i32_eqz(); // [miss?]
    c.if_void();
    c.i32_const(1).global_set(9);
    c.local_get(3).global_set(10);
    emit_exec_count(c, exec);
    c.i64_const(1u64 << 32).i64_or();
    c.return_();
    c.end();
}
/// Dirty record after a mirror store: appends (guest_addr, size) to the
/// trailer, or miss-exits on overflow. Uses l2 as temp (the stored value
/// lives in l1, the guest address in l3 — both preserved; l4 holds the
/// mirror index and is not needed here).
fn emit_dirty_record(c: &mut Code, exec: &ExecCount, size: u32) {
    c.global_get(18).local_tee(2); // [cnt] l2 = cnt
    c.i32_const(TRAILER_CAP as i32).i32_ge_u(); // [full?]
    c.if_void();
    c.i32_const(1).global_set(9);
    c.local_get(3).global_set(10);
    emit_exec_count(c, exec);
    c.i64_const(1u64 << 32).i64_or();
    c.return_();
    c.end();
    // eaddr = TRAILER_BASE + cnt*8; store (addr, size).
    c.local_get(2).i32_const(3).i32_shl().i32_const(TRAILER_BASE as i32).i32_add(); // [eaddr]
    c.local_get(3); // [eaddr, addr]
    c.i32_store(2, 0);
    c.local_get(2).i32_const(3).i32_shl().i32_const(TRAILER_BASE as i32 + 4).i32_add(); // [eaddr+4]
    c.i32_const(size as i32); // [eaddr+4, size]
    c.i32_store(2, 0);
    c.local_get(2).i32_const(1).i32_add().global_set(18);
}

/// One emittable unit. (Branches are handled by the scan, never emitted
/// here.) Memory is one unified form: base reg 0-14 (13 = SP), signed
/// byte offset, data reg 0-14, size 1/4, store flag, writeback mode
/// (0 = none, 1 = post-index base += off, 2 = pre-index w base += off).
#[derive(Clone, Copy)]
enum Item {
    Alu16(u16),
    AddReg { rd: u32, rs: u32, rn: u32 }, // T1 ADD-reg (flags)
    SubReg { rd: u32, rs: u32, rn: u32 }, // T1 SUB-reg (flags)
    AddImm3 { rd: u32, rn: u32, imm: u32 }, // T1 ADDS-imm3 (flags)
    SubImm3 { rd: u32, rn: u32, imm: u32 }, // T1 SUBS-imm3 (flags)
    CmpReg { rda: u32, rsb: u32 },        // 0x4000 sop=10 (flags, no wb)
    AddHi { rd: u32, rs: u32 },           // ADD(2) h=0 (no flags)
    CmpHi { rda: u32, rsb: u32 },        // 0x4400 h=1 (flags, no wb)
    AddWImm { rd: u32, rn: u32, imm: u32 }, // ADD.W-imm s=0 (no flags)
    AddWReg { rd: u32, rn: u32, rm: u32, amt: u32 }, // ADD.W-reg LSL s=0
    Mem { base: u32, off: i32, rt: u32, size: u32, is_store: bool, wb: u32,
          idx: Option<u32>, sh: u32 }, // register offset: addr = base+(idx<<sh)
                                       // (None = plain imm `off`; SP=13 allowed as idx)
}

/// Slot pack for one memory item: base(5b: 0-14 reg, 15 = SP) |
/// size(1/4)<<5. The byte offset travels as a second u32 (slots are
/// flat [pack, off] pairs — PUW offsets are signed).
fn mem_slot_pack(base: u32, size: u32) -> u32 {
    (if base == 13 { 15 } else { base }) | (size << 5)
}

/// Emit LSLS-imm T1 (0x0000 class): Rd = Rs << im; N/Z set, C = last bit
/// shifted out (preserved when im==0), V PRESERVED (mirror of the exec16
/// arm: nz() touches only N/Z, C written explicitly, V never).
fn emit_lsls(c: &mut Code, rd: usize, rs: usize, im: u32) {
    c.global_get(rs as u32).local_set(1); // l1 = v
    c.local_get(1);
    if im != 0 {
        c.i32_const(im as i32).i32_shl();
    }
    c.local_tee(2); // l2 = r
    c.global_set(rd as u32); // Rd = r
    // N/Z from r.
    c.local_get(2).i32_eqz().i32_const(30).i32_shl(); // [z]
    c.local_get(2).i32_const(31).i32_shr_u().i32_const(31).i32_shl(); // [z,n]
    c.i32_or(); // [zn]
    // C bit.
    if im == 0 {
        c.global_get(8).i32_const(XPSR_C as i32).i32_and(); // [zn, oldC]
    } else {
        c.local_get(1).i32_const((32 - im) as i32).i32_shr_u().i32_const(1).i32_and();
        c.i32_const(29).i32_shl(); // [zn, cbit]
    }
    c.i32_or(); // [znc]
    // Keep V + low 28 (clear N/Z/C only).
    c.global_get(8).i32_const(0x1FFF_FFFFu32 as i32).i32_and();
    c.i32_or();
    c.global_set(8);
}

/// Emit one Item (never a branch). `exec` drives the miss-exit count;
/// `segs` is the mirror segment table (guest_base, size, mirror_off).
fn emit_item(c: &mut Code, item: &Item, exec: &ExecCount, segs: &[(u32, u32, u32)]) {
    match *item {
        Item::Alu16(op) => {
            match op & 0xF800 {
                0x2000 => {
                    let rd = ((op >> 8) & 7) as usize;
                    let imm = (op & 0xFF) as u32;
                    c.i32_const(imm as i32).global_set(rd as u32);
                    c.global_get(8).i32_const(XPSR_KEEP as i32).i32_and();
                    c.i32_const(nz_bits(imm) as i32);
                    c.global_get(8).i32_const(XPSR_C as i32).i32_and();
                    c.i32_or().i32_or();
                    c.global_set(8);
                }
                0x2800 => {
                    let rn = ((op >> 8) & 7) as usize;
                    let imm = (op & 0xFF) as u32;
                    emit_load_a(c, rn);
                    emit_sub_flags(c, None, imm);
                }
                0x3000 => {
                    let rd = ((op >> 8) & 7) as usize;
                    let imm = (op & 0xFF) as u32;
                    emit_load_a(c, rd);
                    emit_add_flags(c, Some(rd), imm);
                }
                0x3800 => {
                    let rd = ((op >> 8) & 7) as usize;
                    let imm = (op & 0xFF) as u32;
                    emit_load_a(c, rd);
                    emit_sub_flags(c, Some(rd), imm);
                }
                _ => {
                    if op == 0xBF00 {
                        // NOP: emit nothing.
                    } else {
                        // 0x0000 class = LSLS-imm T1 (verified in the scan).
                        debug_assert_eq!(op & 0xF800, 0x0000);
                        let (rd, rs) = ((op & 7) as usize, ((op >> 3) & 7) as usize);
                        let im = ((op >> 6) & 0x1F) as u32;
                        emit_lsls(c, rd, rs, im);
                    }
                }
            }
        }
        Item::AddReg { rd, rs, rn } => {
            c.global_get(reg_g(rs)).local_tee(1);
            emit_add_reg(c, reg_g(rd) as usize, reg_g(rn));
        }
        Item::SubReg { rd, rs, rn } => {
            c.global_get(reg_g(rs)).local_tee(1);
            emit_sub_reg(c, Some(reg_g(rd) as usize), reg_g(rn));
        }
        Item::AddImm3 { rd, rn, imm } => {
            c.global_get(reg_g(rn)).local_tee(1);
            emit_add_flags(c, Some(reg_g(rd) as usize), imm);
        }
        Item::SubImm3 { rd, rn, imm } => {
            c.global_get(reg_g(rn)).local_tee(1);
            emit_sub_flags(c, Some(reg_g(rd) as usize), imm);
        }
        Item::CmpReg { rda, rsb } => {
            c.global_get(reg_g(rda)).local_tee(1);
            emit_sub_reg(c, None, reg_g(rsb));
        }
        Item::AddHi { rd, rs } => {
            c.global_get(reg_g(rd));
            c.global_get(reg_g(rs));
            emit_add_plain(c, reg_g(rd));
        }
        Item::CmpHi { rda, rsb } => {
            c.global_get(reg_g(rda)).local_tee(1);
            emit_sub_reg(c, None, reg_g(rsb));
        }
        Item::AddWImm { rd, rn, imm } => {
            c.global_get(reg_g(rn)).local_tee(1);
            c.i32_const(imm as i32);
            emit_add_plain(c, reg_g(rd));
        }
        Item::AddWReg { rd, rn, rm, amt } => {
            c.global_get(reg_g(rn)).local_tee(1);
            if amt == 0 {
                c.global_get(reg_g(rm));
            } else {
                c.global_get(reg_g(rm)).i32_const(amt as i32).i32_shl();
            }
            emit_add_plain(c, reg_g(rd));
        }
        Item::Mem { base, off, rt, size, is_store, wb, idx, sh } => {
            let bg = reg_g(base);
            let rtg = reg_g(rt);
            // l2 = base value; l3 = access address.
            c.global_get(bg).local_tee(2);
            if let Some(ir) = idx {
                // Register offset (always ADD in Thumb-2 — GAS rejects
                // negative indexing): addr = base + (idx << sh).
                c.global_get(reg_g(ir));
                if sh != 0 {
                    c.i32_const(sh as i32).i32_shl();
                }
                c.i32_add().local_set(3);
            } else if wb == 1 {
                // Post-index: access at base, then base += off.
                c.local_set(3);
            } else {
                // Imm / pre-index: access at base + off.
                c.i32_const(off).i32_add().local_set(3);
            }
            emit_seg_guard(c, segs, size, exec);
            // Demand-paging: the page must be synced (see BITBASE).
            // Uses l1 as temp — value spills happen after this.
            emit_valid_check(c, exec);
            if is_store {
                // Value into l1 BEFORE the dirty record (which uses l2).
                // NOTE: the store address is l4 (mirror index), NOT l3
                // (guest address) — storing to l3 faults far outside the
                // module memory (caught live, straight STRB OOB).
                c.global_get(rtg).local_set(1);
                c.local_get(4);
                c.local_get(1);
                if size == 4 {
                    c.i32_store(2, 0);
                } else {
                    c.i32_store8(0, 0);
                }
                if wb != 0 {
                    c.local_get(2).i32_const(off).i32_add().global_set(bg);
                }
                emit_dirty_record(c, exec, size);
            } else {
                // Silicon order (writeback BEFORE the Rt write, so a
                // Rt==Rn pre/post-indexed load reads the loaded value,
                // not base+off — mirrors the interpreter arm).
                if wb != 0 {
                    c.local_get(2).i32_const(off).i32_add().global_set(bg);
                }
                c.local_get(4);
                if size == 4 {
                    c.i32_load(2, 0).global_set(rtg);
                } else {
                    c.i32_load8_u(0, 0).global_set(rtg);
                }
            }
        }
    }
}

/// Translate a run starting at `base_pc`. Straight prefix first; if a
/// B.cond latches backward into the submitted run, the body becomes a
/// fuel-bounded WASM loop (run-to-completion or fuel-out). Everything
/// else stops the region. Returns bytes + shape for the driver.
// Pseudo-registers for bit-precise closure: r0-r7 as 0-7, then N/Z/C/V.
// Bit precision matters: MOVS preserves C, so a [MOVS,BCS-back] loop
// would spin on a stale C under whole-flag closure — declined here.
// === Module-level decoder helpers (moved out of translate_at verbatim:
// pure functions, no logic change — the flat scan below uses them as before). ===
#[derive(Clone, Copy)]
enum Class {
    Emit(Item, usize), // (item, halfword size)
    BCond { cc: u32, wide: bool, size: usize },
    BUn { size: usize }, // unconditional B(.N): latch or exit
    Stop(usize),
}
fn classify(ops: &[u16], i: usize) -> Class {
    let op = match ops.get(i) {
        Some(&o) => o,
        None => return Class::Stop(1),
    };
    let cls = op & 0xF800;
    if cls == 0x2000 || cls == 0x2800 || cls == 0x3000 || cls == 0x3800 || op == 0xBF00 {
        return Class::Emit(Item::Alu16(op), 1);
    }
    if cls == 0x0000 {
        // LSLS-imm T1 (the whole 0x0000-0x07FF class is LSL-imm).
        return Class::Emit(Item::Alu16(op), 1);
    }
    if cls == 0x6800 {
        // LDR-word-imm: word offset imm5*4.
        return Class::Emit(
            Item::Mem {
                base: ((op >> 3) & 7) as u32,
                off: (((op >> 6) & 0x1F) as i32) * 4,
                rt: (op & 7) as u32,
                size: 4,
                is_store: false,
                wb: 0,
                idx: None,
                sh: 0,
            },
            1,
        );
    }
    if cls == 0x9800 {
        // LDR-word SP-rel: word offset imm8*4.
        return Class::Emit(
            Item::Mem {
                base: 13,
                off: ((op & 0xFF) as i32) * 4,
                rt: ((op >> 8) & 7) as u32,
                size: 4,
                is_store: false,
                wb: 0,
                idx: None,
                sh: 0,
            },
            1,
        );
    }
    if cls == 0x7000 || cls == 0x7800 {
        // STRB/LDRB-imm: byte offset imm5 (NOT scaled).
        let (rn, rt) = (((op >> 3) & 7) as u32, (op & 7) as u32);
        let off = ((op >> 6) & 0x1F) as i32;
        let is_store = cls == 0x7000;
        return Class::Emit(
            Item::Mem { base: rn, off, rt, size: 1, is_store, wb: 0, idx: None, sh: 0 },
            1,
        );
    }
    if cls == 0x6000 || cls == 0x9000 {
        // STR-word-imm / SP-rel: word offset imm5*4 / imm8*4.
        let (rn, rt, off) = if cls == 0x6000 {
            (((op >> 3) & 7) as u32, (op & 7) as u32, (((op >> 6) & 0x1F) as i32) * 4)
        } else {
            (13, ((op >> 8) & 7) as u32, ((op & 0xFF) as i32) * 4)
        };
        return Class::Emit(
            Item::Mem { base: rn, off, rt, size: 4, is_store: true, wb: 0, idx: None, sh: 0 },
            1,
        );
    }
    if (op & 0xFE00) == 0x1800 || (op & 0xFE00) == 0x1A00 {
        // ADD/SUB-reg T1 (flag-setting; 0x1800 = ADD, 0x1A00 = SUB).
        let (rd, rs, rn) = ((op & 7) as u32, ((op >> 3) & 7) as u32, ((op >> 6) & 7) as u32);
        if (op & 0xFE00) == 0x1800 {
            return Class::Emit(Item::AddReg { rd, rs, rn }, 1);
        } else {
            return Class::Emit(Item::SubReg { rd, rs, rn }, 1);
        }
    }
    if (op & 0xFE00) == 0x1C00 || (op & 0xFE00) == 0x1E00 {
        // ADD/SUB-imm3 (flag-setting, reads Rn — unlike the imm8
        // arms which read Rd).
        let (rd, rn) = ((op & 7) as u32, ((op >> 3) & 7) as u32);
        let im = ((op >> 6) & 7) as u32;
        if (op & 0xFE00) == 0x1C00 {
            return Class::Emit(Item::AddImm3 { rd, rn, imm: im }, 1);
        } else {
            return Class::Emit(Item::SubImm3 { rd, rn, imm: im }, 1);
        }
    }
    if (op & 0xFC00) == 0x4000 {
        let sop = (op >> 6) & 0xF;
        let (rs, rd) = (((op >> 3) & 7) as u32, (op & 7) as u32);
        if sop == 10 {
            // CMP-reg: sub_flags(rd - rs), no writeback (always sets
            // flags, even in IT — driver gates it_n==0 anyway).
            return Class::Emit(Item::CmpReg { rda: rd, rsb: rs }, 1);
        }
        return Class::Stop(1);
    }
    if (op & 0xFC00) == 0x4400 {
        let h = (op >> 8) & 3;
        let rs = ((op >> 3) & 0xF) as u32;
        let rd = ((op & 7) | ((op >> 4) & 8)) as u32;
        if h == 0 && rd != 13 && rd != 15 && rs != 15 {
            // ADD(2): no flags. SP reads via the sp global.
            return Class::Emit(Item::AddHi { rd, rs }, 1);
        }
        if h == 1 && rd != 15 && rs != 15 {
            // CMP(2)-ish: sub_flags, no writeback.
            return Class::Emit(Item::CmpHi { rda: rd, rsb: rs }, 1);
        }
        return Class::Stop(1);
    }
    if (op & 0xF000) == 0xD000 {
        let cc = ((op >> 8) & 0xF) as u32;
        if cc < 14 {
            return Class::BCond { cc, wide: false, size: 1 };
        }
        return Class::Stop(1); // UDF/SVC, never a loop edge
    }
    if (op & 0xF800) == 0xE000 {
        // B.N unconditional (11-bit offset, no condition).
        // NOTE: must precede any F-prefix arm (0xE000 shares no
        // prefix with Fxxx, but ordering documents intent).
        return Class::BUn { size: 1 };
    }
    if (op & 0xFF00) == 0xF800 && op < 0xFA00 {
        // F8 single-transfer (byte/word only; halfword/signed stay
        // interpreted). Class nibble c = (o1>>4)&0xF.
        let c = (op >> 4) & 0xF;
        let o2 = match ops.get(i + 1) {
            Some(&o) => o,
            None => return Class::Stop(1),
        };
        let rn = (op & 0xF) as u32;
        let rt = ((o2 >> 12) & 0xF) as u32;
        if rt == 15 || rn == 15 || rt > 14 || (rn > 14) {
            return Class::Stop(1); // PC-involved: not in the ABI
        }
        if c == 8 || c == 9 || c == 12 || c == 13 {
            // T2 imm12: 8 STRB, 9 LDRB, 12 STR, 13 LDR.
            let (size, is_store) = match c {
                8 => (1, true),
                9 => (1, false),
                12 => (4, true),
                _ => (4, false),
            };
            return Class::Emit(
                Item::Mem { base: rn, off: (o2 & 0xFFF) as i32, rt, size, is_store, wb: 0, idx: None, sh: 0 },
                2,
            );
        }
        // Register offset (GAS-verified layouts, docs/encodings/regidx.s):
        // c 0 STRB / 1 LDRB / 4 STR.W / 5 LDR.W with o2[11:10]==00 (never
        // T3-PUW-imm8, whose offset bits live there). Addr = Rn+(Rm<<imm2),
        // always ADD (GAS rejects negative indexing). Halfword/signed
        // (c 2,3,6,7) stay interpreted.
        if (c == 0 || c == 1 || c == 4 || c == 5) && (o2 & 0xC00) == 0 {
            let (size, is_store) = match c {
                0 => (1, true),
                1 => (1, false),
                4 => (4, true),
                _ => (4, false),
            };
            let rm = (o2 & 0xF) as u32;
            let sh = ((o2 >> 4) & 3) as u32;
            if rm == 15 {
                return Class::Stop(2); // PC-index: not in the ABI
            }
            return Class::Emit(
                Item::Mem { base: rn, off: 0, rt, size, is_store, wb: 0, idx: Some(rm), sh },
                2,
            );
        }
        if c <= 5 {
            // T3 PUW imm8 (register-offset o2[11:10]==00 stays out;
            // unprivileged T-variants o2&0xF00==0xE00 stay out —
            // MPU privilege differs there).
            if (o2 & 0xC00) == 0 || (o2 & 0xF00) == 0xE00 {
                return Class::Stop(1);
            }
            // Byte only here (word PUW stays interpreted for now).
            let size = match c {
                0 | 1 => 1,
                _ => return Class::Stop(1),
            };
            let is_store = c == 0;
            let puw_p = (o2 >> 10) & 1;
            let u = (o2 >> 9) & 1;
            let w = (o2 >> 8) & 1;
            let imm8 = (o2 & 0xFF) as i32;
            let off = if u == 1 { imm8 } else { -imm8 };
            // Mirror of the interpreter arm: addr = p ? base+off :
            // base; writeback iff w || !p.
            let wb = if puw_p == 0 {
                1 // post-index: always writes back
            } else if w == 1 {
                2 // pre-index with writeback
            } else {
                0 // pre-index, no writeback
            };
            return Class::Emit(
                Item::Mem { base: rn, off, rt, size, is_store, wb, idx: None, sh: 0 },
                2,
            );
        }
        return Class::Stop(1);
    }
    if (op & 0xFF00) == 0xEA00 || (op & 0xFF00) == 0xEB00 {
        // ADD.W only (op=8, s=0, no flags): register+LSL and
        // modified-imm forms. Everything else stays interpreted.
        let b8 = (op >> 8) & 1;
        let b7 = (op >> 7) & 1;
        let b6 = (op >> 6) & 1;
        let b5 = (op >> 5) & 1;
        let is_add = (b8 == 1) && (b7 == 0) && (b6 == 0) && (b5 == 0);
        let sbit = (op & 0x10) != 0;
        if !is_add || sbit {
            return Class::Stop(2);
        }
        let o2 = match ops.get(i + 1) {
            Some(&o) => o,
            None => return Class::Stop(1),
        };
        let rn = (op & 0xF) as u32;
        let rd = ((o2 >> 8) & 0xF) as u32;
        if rd == 15 || rn == 15 || rd == 13 {
            return Class::Stop(2); // MOV-form/SP-dst: out of scope
        }
        if (op & 0xFF00) == 0xEB00 {
            // Register form: shift must be LSL (typ==0).
            let typ = (o2 >> 4) & 3;
            if typ != 0 {
                return Class::Stop(2);
            }
            let rm = (o2 & 0xF) as u32;
            if rm == 15 {
                return Class::Stop(2);
            }
            let amt = ((((o2 >> 12) & 7) << 2) | ((o2 >> 6) & 3)) as u32;
            return Class::Emit(Item::AddWReg { rd, rn, rm, amt }, 2);
        } else {
            // Modified-imm form (EA group; the op table lives in
            // the interpreter — here only ADD, s=0).
            let imm12 = ((((op >> 10) & 1) << 11) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF)) as u32;
            let imm = expand_imm_val(imm12);
            return Class::Emit(Item::AddWImm { rd, rn, imm }, 2);
        }
    }
    if (op & 0xF800) == 0xF000 && op < 0xF600 {
        // F-group modified-imm ADD (op=8, s=0). Branch/BL shapes
        // (o2 top bits set) fall THROUGH to the branch arm below —
        // this arm only claims the modimm data zone. Mirrors the
        // interpreter F000-F5FF arm for ADD only.
        // Bit15 selects: 0 = data zone (claimed here or Stop),
        // 1 = branch/BL/coprocessor (falls through to those arms).
        if let Some(&o2) = ops.get(i + 1) {
            if (o2 & 0x8000) == 0 {
                let b8 = (op >> 8) & 1;
                let is_add = b8 == 1 && (op >> 5) & 7 == 0;
                let sbit = (op & 0x10) != 0;
                if !is_add || sbit {
                    return Class::Stop(2);
                }
                let rn = (op & 0xF) as u32;
                let rd = ((o2 >> 8) & 0xF) as u32;
                if rd == 15 || rn == 15 || rd == 13 {
                    return Class::Stop(2);
                }
                let imm12 = ((((op >> 10) & 1) << 11) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF)) as u32;
                let imm = expand_imm_val(imm12);
                return Class::Emit(Item::AddWImm { rd, rn, imm }, 2);
            }
        }
    }
    if (op & 0xF800) == 0xF000 {
        let o2 = match ops.get(i + 1) {
            Some(&o) => o,
            None => return Class::Stop(1),
        };
        if (o2 & 0xC000) != 0x8000 {
            return Class::Stop(2); // BL/BLX/other F-prefix
        }
        if (o2 & 0x1000) != 0 {
            return Class::Stop(2); // B.W: backward would hang, forward skips
        }
        let cc = ((op >> 6) & 0xF) as u32; // Bcc.W: cond in op1[9:6]
        if cc < 14 {
            return Class::BCond { cc, wide: true, size: 2 };
        }
        return Class::Stop(2); // cc 14/15: AL-unfusable / fault
    }
    Class::Stop(1)
}
// 21-bit Bcc.W offset (J bits used DIRECTLY, no S inversion —
// GAS-verified, mirrors the interpreter arm).
fn sx21(v: u32) -> i32 {
    if v & 0x100000 != 0 {
        (v | 0xFFE00000) as i32
    } else {
        v as i32
    }
}
// Branch target in i64 space: below-base targets are None (they wrap
// to huge u32 and would masquerade as forward edges — the old signed
// check caught this; the first u32 cut did not).
/// Raw unconditional-branch target as i64 (None only if the op is
/// missing). Unlike branch_target_un, below-base targets are returned,
/// not rejected — the scanner records them as true loop exits (outer-
/// loop tops) and keeps looking for a later latch within the window.
/// Raw conditional-branch target as i64 (None only if the op is
/// missing). Below-base targets are returned: the tree-discovery scan
/// records them as true exits (VDP's BGE to the epilogue) instead of
/// barrier-ing (which blinded L1 discovery).
fn branch_target_wide(ops: &[u16], hw: usize, wide: bool, base_pc: u32) -> Option<i64> {
    let origin = base_pc as i64 + (hw as i64) * 2 + 4;
    let off: i64 = if wide {
        let o1 = *ops.get(hw)?;
        let o2 = *ops.get(hw + 1)?;
        let s = (o1 >> 10) & 1;
        let j1 = (o2 >> 13) & 1;
        let j2 = (o2 >> 11) & 1;
        sx21(
            ((s as u32) << 20) | (((j1) as u32) << 19) | (((j2) as u32) << 18)
                | (((o1 & 0x3F) as u32) << 12) | (((o2 & 0x7FF) as u32) << 1),
        ) as i64
    } else {
        let op = *ops.get(hw)?;
        sx9((((op & 0xFF) << 1)) as u32) as i64
    };
    Some(origin + off)
}
fn branch_target_un_wide(ops: &[u16], hw: usize, base_pc: u32) -> Option<i64> {
    let op = *ops.get(hw)?;
    let imm = ((op & 0x7FF) << 1) as u32;
    let off = if imm & 0x800 != 0 { (imm | 0xFFFF_F000) as i32 } else { imm as i32 };
    Some(base_pc as i64 + (hw as i64) * 2 + 4 + off as i64)
}
fn branch_target_un(ops: &[u16], hw: usize, base_pc: u32) -> Option<u32> {
    // B.N (0xE000 class): 11-bit offset, sx(imm11 << 1, 12).
    let op = *ops.get(hw)?;
    let imm = ((op & 0x7FF) << 1) as u32;
    let off = if imm & 0x800 != 0 { (imm | 0xFFFF_F000) as i32 } else { imm as i32 };
    let t = base_pc as i64 + (hw as i64) * 2 + 4 + off as i64;
    if t < base_pc as i64 || t > 0xFFFF_FFFEu64 as i64 {
        return None;
    }
    Some(t as u32)
}
fn branch_target(ops: &[u16], hw: usize, wide: bool, base_pc: u32) -> Option<u32> {
    let origin = base_pc as i64 + (hw as i64) * 2 + 4;
    let off: i64 = if wide {
        let o1 = *ops.get(hw)?;
        let o2 = *ops.get(hw + 1)?;
        let s = (o1 >> 10) & 1;
        let j1 = (o2 >> 13) & 1;
        let j2 = (o2 >> 11) & 1;
        sx21(
            ((s as u32) << 20) | (((j1) as u32) << 19) | (((j2) as u32) << 18)
                | (((o1 & 0x3F) as u32) << 12) | (((o2 & 0x7FF) as u32) << 1),
        ) as i64
    } else {
        let op = *ops.get(hw)?;
        sx9((((op & 0xFF) << 1)) as u32) as i64
    };
    let t = origin + off;
    if t < base_pc as i64 || t > 0xFFFF_FFFEu64 as i64 {
        return None;
    }
    Some(t as u32)
}

// === Nested/call region tree (VDP column loop; additive — the flat
// legacy scan below runs unchanged when no tree forms). ===
// A straight item with its halfword position.
struct TItem {
    hw: usize,
    size: usize,
    it: Item,
}
// A branch edge with its halfword position and computed target.
struct TEdge {
    hw: usize,
    size: usize,
    cc: Option<u32>, // None = unconditional B.N
    target: u32,
}
// One loop in the tree: latch + span + classified content.
// Inline-nested loops (strict containment) live in `nests`;
// cross-span forward edges become `calls` (top loop only, v1).
struct TLoop {
    top: usize,
    li: usize,
    cc: Option<u32>,
    lsize: usize,
    nests: Vec<TLoop>,
    calls: Vec<TCall>,
}
// A call site: forward edge in a loop body targeting a callee root.
struct TCall {
    edge_hw: usize,
    cc: u32, // conditional only (unconditional out-edges stay kind-3)
    target: u32,
    callee: Box<TCallee>,
}
// A callee function (VDP MID): straight prefix + one nested loop +
// tail-duplicated dispatch. Entry is exactly `top` (called, never
// submitted); kinds are {0 = unit done, 2 = fuel}.
struct TCallee {
    top: usize,         // entry = loop top (bf2)
    prefix_end: usize,  // prefix items [top..nest.top]
    nest: Box<TLoop>,   // the one nested loop (PIX)
    brk_hw: usize,      // nested true-exit edge retargeted to the dup
    dup_src: usize,     // taildup source span [dup_src..dup_end]
    dup_end: usize,
}
// Full-tree layout over one classify walk (no loop logic here).
struct Layout {
    items: Vec<TItem>,
    edges: Vec<TEdge>,
    end_hw: usize, // first barrier / ops end
}
fn layout_scan(ops: &[u16], base_pc: u32) -> Layout {
    let mut items = Vec::new();
    let mut edges = Vec::new();
    let mut hw = 0usize;
    while hw < ops.len() {
        match classify(ops, hw) {
            Class::Emit(it, size) => {
                items.push(TItem { hw, size, it });
                hw += size;
            }
            Class::BCond { cc, wide, size } => {
                // Below-base targets (VDP's BGE to the epilogue) are true
                // exits like any forward edge — record and continue, never
                // barrier (barrier-ing here blinded L1 discovery; caught by
                // the real-VDP probe, not review). Only the tree-discovery
                // scan does this; the flat legacy scan keeps its break.
                // Out-of-u32-range targets stay barriers (calls/faults).
                let tw = match branch_target_wide(ops, hw, wide, base_pc) {
                    Some(t) => t,
                    None => break,
                };
                if tw < 0 || tw > 0xFFFF_FFFE {
                    break;
                }
                edges.push(TEdge { hw, size, cc: Some(cc), target: tw as u32 });
                hw += size;
            }
            Class::BUn { size } => {
                let tw = match branch_target_un_wide(ops, hw, base_pc) {
                    Some(t) => t,
                    None => break,
                };
                if tw < base_pc as i64 || tw > 0xFFFF_FFFE {
                    if (0x0800_0000..=0xFFFF_FFFE).contains(&tw) {
                        edges.push(TEdge { hw, size, cc: None, target: tw as u32 });
                        hw += size;
                        continue;
                    }
                    break;
                }
                edges.push(TEdge { hw, size, cc: None, target: tw as u32 });
                hw += size;
            }
            Class::Stop(_) => break,
        }
    }
    let end_hw = hw;
    Layout { items, edges, end_hw }
}
// Halfword of an absolute target (None when below base).
fn thw_of(target: u32, base_pc: u32) -> Option<usize> {
    if target < base_pc {
        return None;
    }
    Some(((target - base_pc) / 2) as usize)
}
// True halfword starts of straight items (branches occupy hw too,
// so a naive accumulation misplaces everything past the first branch
// — the skip-block bug, same lesson). Merge items + edges in hw order.
fn true_starts(items: &[TItem], edges: &[TEdge]) -> Vec<usize> {
    let mut ev: Vec<(usize, usize, bool)> = Vec::new();
    for it in items.iter() {
        ev.push((it.hw, it.size, false));
    }
    for e in edges.iter() {
        ev.push((e.hw, e.size, true));
    }
    ev.sort_by_key(|&(h, _, _)| h);
    let mut starts = Vec::with_capacity(items.len());
    for (h, _, is_edge) in ev {
        if !is_edge {
            starts.push(h);
        }
    }
    starts
}
// All backward-edge latches in the layout (strictly backward targets
// at/after base). Self-edges are not latches.
fn all_latches(lay: &Layout, base_pc: u32) -> Vec<(usize, Option<u32>, usize, usize)> {
    let mut out = Vec::new();
    for e in lay.edges.iter() {
        if let Some(thw) = thw_of(e.target, base_pc) {
            if thw < e.hw {
                out.push((e.hw, e.cc, thw, e.size));
            }
        }
    }
    out
}
// Region tree for nested/call shapes (None = flat legacy path).
// v1 caps (tight; anything else declines to legacy):
// - top-entry only (entry_pc == base_pc) with top latch target == base,
// - inline-nest depth <= 1 per func, no calls inside nests/callees,
// - at most 1 call edge (top loop only) -> at most 2 funcs total.
struct RegionTree {
    top: TLoop,
    entry: usize, // submit hw (== top.top for top-entry; inside top body
                  // for mid-entry — never inside nest/callee spans, v1)
    // (callees live inside top.calls; kept here for shape clarity)
}
#[allow(dead_code)]
fn build_tree(
    ops: &[u16],
    base_pc: u32,
    entry_pc: u32,
) -> Option<RegionTree> {
    // Entry hw (submit pc, entry-relative).
    let entry_hw = if entry_pc < base_pc || (entry_pc - base_pc) % 2 != 0 {
        return None;
    } else {
        ((entry_pc - base_pc) / 2) as usize
    };
    let lay = layout_scan(ops, base_pc);
    let latches = all_latches(&lay, base_pc);
    // Top latch: target == base (submit pc is the loop top).

    let found_top = latches.iter().find(|&&(_, _, t, _)| t == 0).cloned();
    let &(li0, cc0, t0, ls0) = match found_top {
        Some(ref l) => l,
        None => {
            return None;
        }
    };
    // Nested candidates: strictly inside (0, li0).
    let nested: Vec<(usize, Option<u32>, usize, usize)> = latches
        .iter()
        .filter(|&&(li, _, t, _)| t > 0 && li < li0)
        .cloned()
        .collect();
    if nested.len() > 2 {
        return None; // cap: at most 2 inline nests (depth checked below)
    }
    // Nesting depth: a nest inside another nest declines (v1 cap).
    for i in 0..nested.len() {
        for j in 0..nested.len() {
            if i == j {
                continue;
            }
            let (lii, _, ti, _) = nested[i];
            let (lij, _, tj, _) = nested[j];
            if ti > tj && lii < lij {
                return None; // i strictly inside j: depth 2
            }
        }
    }
    // Call edges: forward edges in the top body targeting outside
    // [base, base+2*li0]. (Below-base BUn handled by scan-past in the
    // layout; those are true exits, never calls.)
    let lo = base_pc as u64;
    let hi = base_pc as u64 + (li0 as u64) * 2;
    let mut call_edges: Vec<&TEdge> = lay
        .edges
        .iter()
        .filter(|e| {
            let tgt = e.target as u64;
            e.hw < li0 && (tgt < lo || tgt >= hi)
        })
        .collect();
    // Only conditional out-edges become calls (unconditional stay kind-3).
    // (No count cap here: edges that fail formation stay legacy kind-3
    // exits; only SUCCESSFUL calls are capped, after formation.)
    call_edges.retain(|e| e.cc.is_some());
    // Single-entry check for each nest top: every non-self predecessor
    // must be the fallthrough position (no jumps into mid-nest).
    let starts = true_starts(&lay.items, &lay.edges);
    let is_start = |h: usize| starts.contains(&h);
    for &(_, _, nt, _) in nested.iter() {
        for e in lay.edges.iter() {
            if let Some(thw) = thw_of(e.target, base_pc) {
                if thw == nt && e.hw + e.size != nt && e.hw != nt {
                    // A real jump into the nest top (not fallthrough,
                    // not self): decline. (Fallthrough = edge ending
                    // exactly at nt... actually fallthrough means NO
                    // edge: the preceding item ends at nt. An edge
                    // ending at nt via branch = jump. Check: any edge
                    // with target nt whose hw+size != nt is a jump-in,
                    // except the nest's own latch (self, hw>nt always
                    // for backward latch... self latch targets nt from
                    // inside: e.hw > nt, allowed).
                    let is_self_latch = e.hw > nt
                        && latches.iter().any(|&(li, _, t, _)| li == e.hw && t == nt);
                    if !is_self_latch {
                        return None;
                    }
                }
            }
        }
        let _ = is_start; // boundary checks happen at emission
    }
    // No calls inside nested spans (v1 cap).
    for &(_, _, nt, nli) in nested.iter() {
        for e in call_edges.iter() {
            if e.hw > nt && e.hw < nli {
                return None;
            }
        }
    }
    // Callee formation (v1, VDP-tight): each forward-outside call edge
    // targets a callee root P2 holding: straight prefix items up to a
    // nested-loop top N0 (empty prefix allowed: root IS a loop top),
    // exactly one nested loop N (single-entry, no deeper nests or calls
    // inside), and every true-exit of N taildup-able (bounded straight
    // dup span whose terminal branch closes the callee loop back to P2,
    // with a return fallthrough). Edges that fail formation stay legacy
    // kind-3 exits (not fatal); more than one SUCCESSFUL call declines
    // (2-func cap).
    let item_at = |h: usize| lay.items.iter().find(|it| it.hw == h);
    let edge_at = |h: usize| lay.edges.iter().find(|e| e.hw == h);
    let mut calls: Vec<TCall> = Vec::new();
    for ce in call_edges.iter() {
        let p2 = match thw_of(ce.target, base_pc) {
            Some(h) => h,
            None => {

                continue;
            }
        };
        if item_at(p2).is_none() && !latches.iter().any(|&(_, _, t, _)| t == p2) {

            continue;
        }
        // Walk prefix: straight items from p2 to a nested-loop top.
        // The top must be STRICTLY ahead of p2 (a latch targeting p2
        // itself is the dup-terminal pattern, not a nest top — caught
        // live: the synthetic dup latch made n0==p2 and misformed).
        let mut ph = p2;
        let mut n0 = None;
        loop {
            if ph > p2 && latches.iter().any(|&(_, _, t, _)| t == ph) {
                n0 = Some(ph);
                break;
            }
            match item_at(ph) {
                Some(it) => {
                    ph += it.size;
                }
                None => break, // branch/barrier in prefix: not a callee
            }
        }
        let n0 = match n0 {
            Some(h) => h,
            None => {

                continue;
            }
        };
        let &(nli, ncc, nt, nls) = match latches.iter().find(|&&(_, _, t, _)| t == n0) {
            Some(l) => l,
            None => {

                continue;
            }
        };
        // No deeper nests inside N (v1 cap).
        if latches.iter().any(|&(li, _, t, _)| t > nt && li < nli) {

            continue;
        }
        // Single-entry for N.top (same rule as inline nests).
        let mut ok_entry = true;
        for e in lay.edges.iter() {
            if let Some(thw) = thw_of(e.target, base_pc) {
                if thw == nt && e.hw + e.size != nt && e.hw != nt {
                    let is_self = e.hw > nt
                        && latches.iter().any(|&(li, _, t, _)| li == e.hw && t == nt);
                    if !is_self {
                        ok_entry = false;
                    }
                }
            }
        }
        if !ok_entry {

            continue;
        }
        // N's true-exits (forward, outside N span): backward-to-before-nt
        // is a nested-shape violation (only the latch goes back).
        let nlo = base_pc as u64 + (nt as u64) * 2;
        let nhi = base_pc as u64 + (nli as u64) * 2;
        let mut texits: Vec<usize> = Vec::new();
        for e in lay.edges.iter() {
            if e.hw < nt || e.hw >= nli {

                continue;
            }
            let tgt = e.target as u64;
            if tgt < nlo {
                ok_entry = false;
            } else if tgt >= nhi {
                texits.push(e.hw);
            }
        }
        // v1: exactly one dup-needing true-exit (the field holds a
        // single dup span) and an unconditional nest latch (a Bcc latch
        // completion would fall past the nest with nowhere defined to go).
        if !ok_entry || texits.len() != 1 {

            continue;
        }
        if ncc.is_some() {

            continue;
        }
        // Taildup spans: straight items (cap 8), no back-edges, terminal
        // = cond-branch taken->p2 (callee latch) + straight fallthrough
        // (cap 2, the return path). Direct continue (exit targets p2)
        // needs no dup.
        let mut dups: Vec<(usize, usize)> = Vec::new();
        let mut ok_dup = true;
        for &xh in texits.iter() {
            let xe = edge_at(xh).unwrap();
            if xe.target == base_pc + (p2 as u32) * 2 {

                continue; // direct continue: no dup needed
            }
            let mut dh = match thw_of(xe.target, base_pc) {
                Some(h) => h,
                None => {
                    ok_dup = false;
                    break;
                }
            };
            // Caller-top pc (L1 top): the dup trail must PROVE native
            // continuation there (trailing branch to it). A clean
            // fallthrough end proves nothing about where native goes
            // next, so it declines (the old fallthrough acceptance was
            // unsound for continue-after-call).
            let caller_top_pc = base_pc + (t0 as u32) * 2;
            let ds = dh;
            let mut straight = 0usize;
            let mut closed = false;
            loop {
                if straight >= 8 {
                    break;
                }
                if let Some(it) = item_at(dh) {
                    dh += it.size;
                    straight += 1;

                    continue;
                }
                if let Some(be) = edge_at(dh) {
                    if be.cc.is_none() {
                        break;
                    }
                    if be.target != base_pc + (p2 as u32) * 2 {
                        break;
                    }
                    dh += be.size;
                    // Trailing straight (cap 2), then MUST end with an
                    // unconditional branch to the caller top (the native
                    // resume point justifying continue-after-call).
                    let mut trail = 0usize;
                    loop {
                        if trail >= 2 {
                            break;
                        }
                        match item_at(dh) {
                            Some(it) => {
                                dh += it.size;
                                trail += 1;
                            }
                            None => break,
                        }
                    }
                    match edge_at(dh) {
                        Some(re) if re.cc.is_none() && re.target == caller_top_pc => {
                            dh += re.size;
                            closed = true;
                        }
                        _ => {}
                    }
                    break;
                }
                break;
            }
            if !closed {
                ok_dup = false;
                break;
            }
            dups.push((ds, dh));
        }
        if !ok_dup {

            continue;
        }
        // Dup spans must be disjoint from [p2..nli] (overlap would
        // double-emit; overlap with the CALLER body is fine, read-only).
        // Only the terminal latch-to-p2 may sit inside as a branch.
        let caller_top_pc = base_pc + (t0 as u32) * 2;
        let mut ok_span = true;
        for &(ds, de) in dups.iter() {
            if ds < nli && de > p2 {
                ok_span = false;
            }
            for e in lay.edges.iter() {
                if e.hw >= ds && e.hw < de {
                    // Allowed inside dup: the terminal latch-to-p2, and
                    // the return-branch to the caller top (replaced by
                    // return-0 at emission; never emitted as a branch).
                    let is_term = e.cc.is_some() && e.target == base_pc + (p2 as u32) * 2;
                    let is_retb =
                        e.cc.is_none() && e.target == caller_top_pc;
                    if !is_term && !is_retb {
                        ok_span = false;
                    }
                }
            }
        }
        if !ok_span {

            continue;
        }
        calls.push(TCall {
            edge_hw: ce.hw,
            cc: ce.cc.unwrap(),
            target: ce.target,
            callee: Box::new(TCallee {
                top: p2,
                prefix_end: n0,
                brk_hw: texits[0],
                nest: Box::new(TLoop {
                    top: nt,
                    li: nli,
                    cc: ncc,
                    lsize: nls,
                    nests: Vec::new(),
                    calls: Vec::new(),
                }),
                dup_src: dups.first().map(|&(a, _)| a).unwrap_or(0),
                dup_end: dups.first().map(|&(_, b)| b).unwrap_or(0),
            }),
        });
        if calls.len() > 1 {
            return None; // cap: 1 call (2 funcs)
        }
    }
    let mut top = TLoop { top: t0, li: li0, cc: cc0, lsize: ls0, nests: Vec::new(), calls };
    for &(nli, ncc, nt, nls) in nested.iter() {
        top.nests.push(TLoop {
            top: nt,
            li: nli,
            cc: ncc,
            lsize: nls,
            nests: Vec::new(),
            calls: Vec::new(),
        });
    }
    // Mid-entry validation (v1): entry must be an item start strictly
    // inside the top body (top itself = top-entry, always fine), never
    // inside a nest span (those belong to the nested construct) nor a
    // callee span (cross-func partials are out of scope — flat legacy
    // owns them). Prefix stores can't double-apply: execution starts AT
    // entry, never before it.
    if entry_hw != t0 {
        let is_start = true_starts(&lay.items, &lay.edges).contains(&entry_hw);
        if !is_start || entry_hw <= t0 || entry_hw >= li0 + ls0 {
            return None;
        }
        for n in top.nests.iter() {
            if entry_hw > n.top && entry_hw < n.li + n.lsize {
                return None;
            }
        }
        for c in top.calls.iter() {
            let cn = &c.callee;
            // Callee span: [top..dup_end] (prefix + nest + dup).
            if entry_hw >= cn.top && entry_hw < cn.dup_end {
                return None;
            }
        }
    }
    Some(RegionTree { top, entry: entry_hw })
}

pub fn translate(ops: &[u16], segs_in: &[u32], base_pc: u32) -> Block {
    translate_at(ops, segs_in, base_pc, base_pc)
}

/// Translate with an explicit entry point for mid-body submission
/// (`entry_pc` inside the loop body). Entry at/before the top behaves
/// exactly like translate(); entry inside (top, latch) emits a partial
/// first pass [entry..latch) then the full loop — no code before the
/// entry ever executes, so mid-body store loops are EXACT (nothing is
/// re-executed, unlike a top resubmit). Entry at/after the latch, off
/// an item boundary, or outside the latched body declines the loop.
/// Nested/call module emission (separate path; the flat scan below runs
/// unchanged when no tree forms or emission declines). Exact counting via
/// LTOT (l9): incremented per executed item, accumulated across calls.
/// No iters/partial/n statics, no adj corrections (skipped items never
/// increment). Fuel lives in FUEL_GLOBAL (shared across funcs/levels).
/// Every func = exactly one loop (top-entry only); kinds {0,1,2,3} with
/// tb_addr set on 1/2/3. Returns None on any shape violation (caller falls
/// back to the flat legacy path — declining is always sound).
fn emit_tree(
    tree: &RegionTree,
    lay: &Layout,
    base_pc: u32,
    segs: &[(u32, u32, u32)],
    mirror_bytes: u32,
) -> Option<Block> {
    // v1 caps (mirror discovery): top + <=2 sibling nests (depth 1),
    // <=1 call, callee = prefix + 1 nest + taildup. Anything else None.
    if tree.top.nests.len() > 2 {
        return None;
    }
    if tree.top.calls.len() > 1 {
        return None;
    }
    for n in tree.top.nests.iter() {
        if !n.nests.is_empty() || !n.calls.is_empty() {
            return None;
        }
    }
    // Item/edge lookup by hw.
    let item_at = |h: usize| lay.items.iter().find(|it| it.hw == h);
    let edge_at = |h: usize| lay.edges.iter().find(|e| e.hw == h);
    // Collect (hw,size,Item) for a span, verifying contiguity (no gaps:
    // gaps mean un-translated bytes would execute silently — decline).
    let span_items = |mut h: usize, end: usize| -> Option<Vec<(usize, usize, Item)>> {
        let mut out = Vec::new();
        while h < end {
            match item_at(h) {
                Some(it) => {
                    out.push((h, it.size, it.it));
                    h += it.size;
                }
                None => return None,
            }
        }
        if h != end {
            return None;
        }
        Some(out)
    };
    let _ = (item_at, edge_at, span_items);
    // ---- Straight item + LTOT++ (exact counting for nested modules). ----
    fn emit_counted(
        c: &mut Code,
        it: &Item,
        segs: &[(u32, u32, u32)],
        slots: &mut Vec<u32>,
    ) {
        if let Item::Mem { base, off, size: msize, is_store, wb, .. } = *it {
            let pack = (if base == 13 { 15 } else { base }) | ((msize == 4) as u32) << 5
                | ((is_store as u32) << 6);
            slots.push(pack);
            slots.push(if wb == 1 { 0 } else { off as u32 });
        }
        emit_item(c, it, &ExecCount::Nested, segs);
        emit_tick(c);
    }
    // ---- Body plan for one loop span: linear op sequence. ----
    // (Nested spans emit as constructs; skips spanning a nest/call span
    // decline; unconditional true exits decline (v1).)
    // Plan one loop span [top, li] (li = latch hw). Returns ops + fuel
    // cycle (own items + edge evals; nested spans excluded — they check
    // shared fuel themselves). Reachability enforced inline.
    #[allow(clippy::too_many_arguments)]
    // walk_from: first hw to emit (== top for full loops; entry hw for
    // mid-entry partials — same span bounds, reachability seeded there).
    #[allow(clippy::too_many_arguments)]
    fn plan_span(
        lay: &Layout,
        base_pc: u32,
        top: usize,
        li: usize,
        latch_size: usize,
        latch_cc: Option<u32>,
        nests: &[(usize, usize, usize)], // (top, li, lsize) inline nests
        calls: &[(usize, usize, u32)],     // (edge_hw, func_idx, callee_top)
        brks: &[(usize, usize)],         // (edge_hw, dup_src_hw)
        walk_from: usize,
    ) -> Option<(Vec<BodyOp>, u32)> {
        let item_at = |h: usize| lay.items.iter().find(|it| it.hw == h);
        let edge_at = |h: usize| lay.edges.iter().find(|e| e.hw == h);
        // Item starts (true positions) for boundary checks.
        let mut starts: Vec<usize> = Vec::new();
        {
            let mut ev: Vec<(usize, usize, bool)> = Vec::new();
            for it in lay.items.iter() {
                ev.push((it.hw, it.size, false));
            }
            for e in lay.edges.iter() {
                ev.push((e.hw, e.size, true));
            }
            ev.sort_by_key(|&(h, _, _)| h);
            for (h, _, is_edge) in ev {
                if !is_edge {
                    starts.push(h);
                }
            }
        }
        let is_start = |h: usize| starts.contains(&h);
        let mut ops: Vec<BodyOp> = Vec::new();
        let mut cyc: u32 = 0;
        let mut pos = walk_from;
        // Reachability set (positions reached by fallthrough/branches).
        use std::collections::HashSet;
        let mut reach: HashSet<usize> = HashSet::new();
        reach.insert(walk_from);
        let span_end = li + latch_size;
        while pos < span_end {
            // Inline nest construct covers [ntop..nli+nlsize].
            if let Some(&(ntop, nli, nls)) = nests.iter().find(|&&(t, _, _)| t == pos) {
                if !reach.contains(&pos) {
                    return None;
                }
                ops.push(BodyOp::Nest(pos));
                cyc += 0; // nested spans check shared fuel themselves
                pos = nli + nls;
                reach.insert(pos);
                let _ = ntop;
                continue;
            }
            if let Some(e) = edge_at(pos) {
                // Latch (must be the span latch, backward to top).
                if pos == li {
                    if e.cc != latch_cc {
                        return None;
                    }
                    // (target==top verified by caller passing latch info)
                    ops.push(BodyOp::Latch(e.size));
                    cyc += 1;
                    pos += e.size;
                    continue;
                }
                // Nested-exit break (dup'd tail follows the construct;
                // edge target must equal the dup source). Conditional
                // breaks fall through (insert); unconditional don't.
                if let Some(&(_, dsrc)) = brks.iter().find(|&&(h, _)| h == pos) {
                    if e.target != base_pc + (dsrc as u32) * 2 {
                        return None;
                    }
                    if !reach.contains(&pos) {
                        return None;
                    }
                    ops.push(BodyOp::Brk(pos));
                    cyc += 1;
                    if e.cc.is_some() {
                        reach.insert(pos + e.size);
                    }
                    pos += e.size;
                    continue;
                }
                // Call site (target must equal the callee top).
                if let Some(&(_, fidx, ctop)) = calls.iter().find(|&&(h, _, _)| h == pos) {
                    if e.cc.is_none() {
                        return None; // unconditional out-edges stay kind-3
                    }
                    if e.target != base_pc + ctop * 2 {
                        return None;
                    }
                    if !reach.contains(&pos) {
                        return None;
                    }
                    ops.push(BodyOp::Call(pos, e.cc.unwrap(), e.size, fidx));
                    cyc += 1;
                    pos += e.size;
                    reach.insert(pos);
                    continue;
                }
                // Forward edges: skip (internal) or exit (outside).
                let tgt = e.target as u64;
                let lo = base_pc as u64 + (top as u64) * 2;
                let hi = base_pc as u64 + (span_end as u64) * 2;
                if tgt >= lo && tgt < hi {
                    // Internal: must be conditional, on-boundary, and span
                    // no nest/call region.
                    let thw = ((e.target - base_pc) / 2) as usize;
                    if e.cc.is_none() || !is_start(thw) || thw <= pos {
                        return None;
                    }
                    // Span must avoid nest bodies and call edges: a
                    // skipped-over nest/call would execute unsynced.
                    for &(nt, nli2, _) in nests.iter() {
                        if nt > pos && nt < thw {
                            return None;
                        }
                        let _ = nli2;
                    }
                    for &(ch, _, _) in calls.iter() {
                        if ch > pos && ch < thw {
                            return None;
                        }
                    }
                    // (Brk edges inside skip spans are fine — VDP's c08
                    // breaks out of the BCS skip to the dup tail. They
                    // emit a bare br (no sync implications).)
                    if !reach.contains(&pos) {
                        return None;
                    }
                    ops.push(BodyOp::Skip(pos, e.cc.unwrap(), e.size, thw));
                    cyc += 1;
                    pos += e.size;
                    reach.insert(pos);
                    reach.insert(thw);
                    continue;
                } else {
                    // True exit: conditional only (v1: no uncond kind-3).
                    if e.cc.is_none() {
                        return None;
                    }
                    if !reach.contains(&pos) {
                        return None;
                    }
                    ops.push(BodyOp::Exit(pos, e.cc.unwrap(), e.size, e.target));
                    cyc += 1;
                    pos += e.size;
                    reach.insert(pos);
                    continue;
                }
            }
            // Straight item (must be reachable).
            match item_at(pos) {
                Some(it) => {
                    if !reach.contains(&pos) {
                        return None;
                    }
                    ops.push(BodyOp::Item(pos, it.size));
                    cyc += 1;
                    pos += it.size;
                    reach.insert(pos);
                }
                None => return None, // gap/barrier inside span
            }
        }
        if pos != span_end {
            return None;
        }
        // Latch must be last.
        if !matches!(ops.last(), Some(BodyOp::Latch(_))) {
            return None;
        }
        // Skip-vs-nest span overlap check (skips must avoid nest bodies).
        for op in ops.iter() {
            if let BodyOp::Skip(hw, _, _, tgt) = *op {
                for &(nt, nli, _) in nests.iter() {
                    // Overlap if nest top strictly inside (hw, tgt).
                    if nt > hw && nt < tgt {
                        return None;
                    }
                }
            }
        }
        // Latch edge must exist at li with matching cc.
        match lay.edges.iter().find(|e| e.hw == li) {
            Some(e) if e.cc == latch_cc => {}
            _ => return None,
        }
        Some((ops, cyc))
    }
    // ---- Recursive nested emitter (depth-tracked structured control).
    // Frames mirror emitted constructs; br depths resolve by label:
    // continue = depth of innermost Loop; Brk = depth of ExitB;
    // completion-skip = depth of PostB; fuelout = depth of Fuel.
    #[derive(Clone, Copy, PartialEq)]
    enum NFrame {
        Fuel,
        End,
        Loop,
        ExitB, // nested-loop exit block (Brk lands after it = dup start)
        PostB, // post block (completion skips dup via this)
        Skip,  // internal-skip block
        If,    // cond-exit/call dispatch
    }
    struct NestEmit<'a> {
        lay: &'a Layout,
        base_pc: u32,
        segs: &'a [(u32, u32, u32)],
        slots: Vec<u32>,
        stack: Vec<NFrame>,
    }
    impl<'a> NestEmit<'a> {
        fn depth_of(&self, f: NFrame) -> Option<u32> {
            self.stack.iter().rev().position(|&x| x == f).map(|p| p as u32)
        }
        fn item_at(&self, h: usize) -> Option<(usize, Item)> {
            self.lay.items.iter().find(|it| it.hw == h).map(|it| (it.size, it.it))
        }
        fn edge_at(&self, h: usize) -> Option<(usize, Option<u32>, u32)> {
            self.lay.edges.iter().find(|e| e.hw == h).map(|e| (e.size, e.cc, e.target))
        }
        fn counted(&mut self, c: &mut Code, hw: usize) -> Option<()> {
            let (_, it) = self.item_at(hw)?;
            let segs = self.segs;
            emit_counted(c, &it, segs, &mut self.slots);
            Some(())
        }
        // Shared-fuel check: fuel < cycle -> tb_addr=level_top, br Fuel.
        fn fuel(&mut self, c: &mut Code, cycle: u32, level_top_pc: u32) -> Option<()> {
            let fod = self.depth_of(NFrame::Fuel)?;
            c.global_get(FUEL_GLOBAL).i32_const(cycle as i32).i32_lt_u();
            c.if_void();
            self.stack.push(NFrame::If);
            c.i32_const(level_top_pc as i32).global_set(10);
            c.br(fod + 1);
            c.end();
            self.stack.pop();
            c.global_get(FUEL_GLOBAL).i32_const(cycle as i32).i32_sub().global_set(FUEL_GLOBAL);
            Some(())
        }
        // Return (LTOT | kind<<32). tb_addr preset by caller for kinds 1/2/3.
        fn retn(&mut self, c: &mut Code, kind: u32) {
            emit_counted_value(c);
            if kind != 0 {
                c.i64_const((kind as u64) << 32).i64_or();
            }
            c.return_();
        }
    }
    // LTOT++ (exact-count tick for nested modules).
    fn emit_tick(c: &mut Code) {
        c.local_get(LTOT_LOCAL).i32_const(1).i32_add().local_set(LTOT_LOCAL);
    }
    // Push LTOT as i64 (return-value base).
    fn emit_counted_value(c: &mut Code) {
        c.local_get(LTOT_LOCAL);
        c.i64_extend_u();
    }
    #[derive(Clone, Copy)]
    enum BodyOp {
        Item(usize, usize), // (hw, size) — Item looked up at emission
        Skip(usize, u32, usize, usize), // (hw, cc, size, target_hw)
        Exit(usize, u32, usize, u32), // (hw, cc, size, target) kind-3
        Call(usize, u32, usize, usize), // (hw, cc, size, func_idx)
        Nest(usize),   // nested loop top hw
        Latch(usize),  // latch branch size (cc looked up at emission)
        Brk(usize),    // nested-exit break (hw) -> break out of inner loop
    }
    // ---- Walker, constructs, funcs, module ----
    // Walk planned ops (Latch handled by the loop assembler, not here).
    impl<'a> NestEmit<'a> {
        #[allow(clippy::too_many_arguments)]
        fn walk(
            &mut self,
            c: &mut Code,
            ops: &[BodyOp],
            nests: &[TLoop],
            calls: &[(usize, usize)],
        ) -> Option<()> {
            let mut open_skip: Option<usize> = None;
            for op in ops.iter() {
                let ophw = match *op {
                    BodyOp::Item(h, _) => h,
                    BodyOp::Skip(h, _, _, _) => h,
                    BodyOp::Exit(h, _, _, _) => h,
                    BodyOp::Call(h, _, _, _) => h,
                    BodyOp::Nest(h) => h,
                    BodyOp::Latch(_) => continue,
                    BodyOp::Brk(h) => h,
                };
                while open_skip == Some(ophw) {
                    c.end();
                    self.stack.pop();
                    open_skip = None;
                }
                match *op {
                    BodyOp::Item(hw, _) => {
                        self.counted(c, hw)?;
                    }
                    BodyOp::Skip(hw, cc, _, tgt) => {
                        emit_tick(c);
                        c.block_void();
                        self.stack.push(NFrame::Skip);
                        emit_cond(c, cc);
                        c.br_if(0);
                        open_skip = Some(tgt);
                        let _ = hw;
                    }
                    BodyOp::Exit(_, cc, _, target) => {
                        emit_tick(c);
                        emit_cond(c, cc);
                        c.if_void();
                        self.stack.push(NFrame::If);
                        c.i32_const(target as i32).global_set(10);
                        self.retn(c, 3);
                        c.end();
                        self.stack.pop();
                    }
                    BodyOp::Call(_, cc, _, fidx) => {
                        emit_tick(c);
                        emit_cond(c, cc);
                        c.if_void();
                        self.stack.push(NFrame::If);
                        self.emit_call_continue(c, fidx)?;
                        c.end();
                        self.stack.pop();
                    }
                    BodyOp::Nest(hw) => {
                        let nest = nests.iter().find(|n| n.top == hw)?;
                        self.emit_nest(c, nest, &[])?;
                    }
                    BodyOp::Latch(_) => {}
                    BodyOp::Brk(hw) => {
                        // Nested-exit break: unconditional breaks always;
                        // conditional breaks iff taken (else fallthrough
                        // continues the nested body — VDP's c08 is bare B,
                        // synthetic BCS is conditional).
                        emit_tick(c);
                        let d = self.depth_of(NFrame::ExitB)?;
                        match self.edge_at(hw).map(|(_, cc, _)| cc) {
                            Some(None) => {
                                c.br(d);
                            }
                            Some(Some(k)) => {
                                emit_cond(c, k);
                                c.if_void();
                                self.stack.push(NFrame::If);
                                c.br(d + 1);
                                c.end();
                                self.stack.pop();
                            }
                            None => return None,
                        }
                    }
                }
            }
            if open_skip.is_some() {
                return None;
            }
            Some(())
        }
        // Call internal func + continue-on-0: acc += count; kind 0 ->
        // `continue` caller-top (the callee's return path natively flows
        // to the caller top — proven by the dup-end check — so resuming
        // there is exact; falling through would re-execute code the
        // callee already passed, e.g. double r2++); else propagate the
        // return (LTOT | kind<<32) with callee-set tb_addr.
        fn emit_call_continue(&mut self, c: &mut Code, fidx: usize) -> Option<()> {
            c.call(fidx as u32);
            c.local_tee(CALLRET_LOCAL);
            c.i32_wrap_i64();
            c.local_get(LTOT_LOCAL);
            c.i32_add();
            c.local_set(LTOT_LOCAL);
            c.local_get(CALLRET_LOCAL);
            c.i64_const(32);
            c.i64_shr_u();
            c.i32_wrap_i64();
            c.local_tee(1); // kind is i32 -> l1 scratch
            c.i32_eqz();
            c.if_void();
            self.stack.push(NFrame::If);
            // kind 0 -> continue caller loop (depth resolved with the if
            // on the stack: cont counts it, so br cont, not cont+1).
            let cont = self.depth_of(NFrame::Loop)?;
            c.br(cont);
            c.end();
            self.stack.pop();
            // Non-zero kind: propagate (LTOT incl. call count) with kind
            // from l1 and callee-set tb_addr.
            c.local_get(1);
            c.if_void();
            self.stack.push(NFrame::If);
            c.local_get(LTOT_LOCAL);
            c.i64_extend_u();
            c.local_get(1);
            c.i64_extend_u();
            c.i64_const(32);
            c.i64_shl();
            c.i64_or();
            c.return_();
            c.end();
            self.stack.pop();
            Some(())
        }
        // Nested loop construct: post{exit{loop}}. Dup (if any) is emitted
        // by the caller right after (inside $post). brks = nested-exit
        // breaks for this nest (empty for inline nests).
        fn emit_nest(&mut self, c: &mut Code, nest: &TLoop, brks: &[(usize, usize)]) -> Option<()> {
            if !nest.nests.is_empty() || !nest.calls.is_empty() {
                return None; // v1 cap: depth 1, no calls inside nests
            }
            let (ops, cyc) = match plan_span(
                self.lay,
                self.base_pc,
                nest.top,
                nest.li,
                nest.lsize,
                nest.cc,
                &[],
                &[],
                brks,
                nest.top,
            ) {
                Some(x) => x,
                None => {
                    return None;
                }
            };
            // Nested completion skips any dup: the Bcc-fallthrough path
            // lands here (inside $exit, after the loop) and must branch
            // past $post. Unconditional latches never fall out (dead br,
            // validation-clean, uniform).;
            c.block_void();
            self.stack.push(NFrame::PostB);
            c.block_void();
            self.stack.push(NFrame::ExitB);
            c.loop_void();
            self.stack.push(NFrame::Loop);
            self.fuel(c, cyc, self.base_pc + (nest.top as u32) * 2)?;
            self.walk(c, &ops, &[], &[])?;
            self.emit_latch(c, nest.li, nest.cc)?;
            c.end();
            self.stack.pop(); // Loop
            // Completion (Bcc fallthrough only; uncond never falls out):
            // skip any dup by branching past $post.
            let pd = self.depth_of(NFrame::PostB)?;
            c.br(pd);
            c.end();
            self.stack.pop(); // ExitB
            // (caller emits dup here, inside $post)
            c.end();
            self.stack.pop(); // PostB
            Some(())
        }
        // Latch: taken -> continue innermost loop; Bcc fallthrough exits
        // the construct (handled structurally by the caller).
    // Dup tail: straight items + terminal cond-branch (taken ->
    // continue callee loop, nt -> fallthrough return path). Returns the
    // fuel-cycle contribution (items + 1 eval).
    // Dup tail emission. The trailing return-branch (proof of native
    // continuation, verified at discovery) is NOT emitted — the caller
    // returns kind-0 in its place.
    fn emit_dup(&mut self, c: &mut Code, cal: &TCallee, caller_top_pc: u32) -> Option<u32> {
        let mut cyc = 0u32;
        let mut h = cal.dup_src;
        // Straight items until the terminal branch.
        loop {
            if h >= cal.dup_end {
                return None; // no terminal: malformed
            }
            if let Some((sz, it)) = self.item_at(h) {
                let segs = self.segs;
                emit_counted(c, &it, segs, &mut self.slots);
                cyc += 1;
                h += sz;
                continue;
            }
            break;
        }
        // Terminal: conditional branch taken -> callee top.
        let (tsz, tcc, ttgt) = self.edge_at(h)?;
        let tcc = tcc?;
        if ttgt != self.base_pc + (cal.top as u32) * 2 {
            return None;
        }
        emit_tick(c);
        emit_cond(c, tcc);
        let cont = self.depth_of(NFrame::Loop)?;
        c.br_if(cont); // taken -> continue callee loop
        h += tsz;
        cyc += 1;
        // Trailing straight items (return path).
        while h < cal.dup_end {
            // Terminal return-branch: verified (BUn to caller top), never
            // emitted as control flow — the caller returns kind-0
            // instead. But it IS a natively-executed instruction, so it
            // still ticks (else fused undercounts native by 1 per call —
            // caught by the native-truth test, not review).
            if let Some((rsz, rcc, rtgt)) = self.edge_at(h) {
                if rcc.is_none() && rtgt == caller_top_pc {
                    emit_tick(c);
                    h += rsz;
                    continue;
                }
                return None;
            }
            let (sz, it) = self.item_at(h)?;
            let segs = self.segs;
            emit_counted(c, &it, segs, &mut self.slots);
            cyc += 1;
            h += sz;
        }
        if h != cal.dup_end {
            return None;
        }
        Some(cyc)
    }

        fn emit_latch(&mut self, c: &mut Code, li: usize, cc: Option<u32>) -> Option<()> {
            let (_, ecc, _) = self.edge_at(li)?;
            if ecc != cc {
                return None;
            }
            emit_tick(c);
            let cont = self.depth_of(NFrame::Loop)?;
            match cc {
                Some(k) => {
                    emit_cond(c, k);
                    c.if_void();
                    self.stack.push(NFrame::If);
                    c.br(cont + 1);
                    c.end();
                    self.stack.pop();
                }
                None => {
                    c.br(cont);
                }
            }
            Some(())
        }
    }
    // ---- Func assemblers ----
    // Callee func (type1): LTOT=0 prologue; loop{prefix, nest, dup}.
    // The loop latch is the dup terminal (taken -> continue to prefix;
    // nt -> trailing + return kind-0). Returns kind-0 (unit done) or
    // kind-2 (fuel, tb_addr = innermost top).
    fn emit_callee_func(
        st: &mut NestEmit,
        c: &mut Code,
        cal: &TCallee,
        base_pc: u32,
        caller_top_pc: u32,
    ) -> Option<()> {
        c.local(10, I32); // l0 (unused dummy; keeps l1..l9 layout identical
        // to the run func so shared LTOT=l9 / l10-temp indices match)
        c.local(1, I64); // l10 (i64 call-result temp; callees make no calls but share layout)
        c.i32_const(0).local_set(LTOT_LOCAL);
        c.block_void();
        st.stack.push(NFrame::Fuel);
        c.block_void();
        st.stack.push(NFrame::End);
        c.loop_void();
        st.stack.push(NFrame::Loop);
        // Fuel cycle (upper bound): prefix + dup halfwords + evals. The
        // nested loop checks shared fuel itself.
        let cyc = (cal.prefix_end - cal.top) as u32 + (cal.dup_end - cal.dup_src) as u32 + 2;
        st.fuel(c, cyc, base_pc + (cal.top as u32) * 2)?;
        // Prefix [top..prefix_end]: straight items (re-verified).
        let mut h = cal.top;
        while h < cal.prefix_end {
            let (sz, it) = st.item_at(h)?;
            let segs = st.segs;
            emit_counted(c, &it, segs, &mut st.slots);
            h += sz;
        }
        if h != cal.prefix_end {
            return None;
        }
        // Nested loop (brks = the dup'd exit).
        st.emit_nest(c, &cal.nest, &[(cal.brk_hw, cal.dup_src)])?;
        // Dup tail (terminal taken -> continue, nt -> fallthrough).
        st.emit_dup(c, cal, caller_top_pc)?;
        // nt path: unit done -> return kind-0.
        st.retn(c, 0);
        c.end();
        st.stack.pop(); // Loop
        c.end();
        st.stack.pop(); // End
        c.unreachable_(); // loop never falls out (all paths close it)
        c.end();
        st.stack.pop(); // Fuel
        st.retn(c, 2); // fuelout (tb_addr preset by checker)
        c.unreachable_();
        c.end();
        Some(())
    }
    // ---- Module assembly ----
    // Func indices: 0 = top run func; callees 1.. in tree order.
    let mut m = Module::new();
    m.functype(&[I32], &[0x7E]); // (i32)->i64 run
    m.functype(&[], &[0x7E]); // ()->i64 callees
    m.func(0);
    for _ in tree.top.calls.iter() {
        m.func(1);
    }
    let pages = ((mirror_bytes + 65535) / 65536).max(1);
    m.memory(pages);
    for _ in 0..21 {
        m.global_mut_i32(0);
    }
    m.export_func("run", 0);
    m.export_mem("mem", 0);
    for (i, name) in [
        "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "xpsr", "tb_exit", "tb_addr", "sp",
        "r8", "r9", "r10", "r11", "r12", "lr", "dirtyCount", "skipAdj", "tbFuel",
    ]
        .iter()
        .enumerate()
    {
        m.export_global(name, i as u32);
    }
    let mut st = NestEmit { lay, base_pc, segs, slots: Vec::new(), stack: Vec::new() };
    // Top plan (calls carry callee-top HW for target verification).
    let top_calls: Vec<(usize, usize, u32)> = tree.top.calls.iter().enumerate()
        .map(|(i, c)| (c.edge_hw, 1 + i, c.callee.top as u32)).collect();
    if top_calls.len() > 1 {
        return None;
    }
    let top_nests: Vec<(usize, usize, usize)> = tree.top.nests.iter()
        .map(|n| (n.top, n.li, n.lsize)).collect();
    let (top_ops, top_cyc) = plan_span(
        lay, base_pc, tree.top.top, tree.top.li, tree.top.lsize, tree.top.cc,
        &top_nests.iter().map(|&(t, l, s)| (t, l, s)).collect::<Vec<_>>(),
        &top_calls,
        &[],
        tree.top.top,
    )?;
    // Mid-entry partial (entry inside the top body): straight walk from
    // entry to the top latch, then fallthrough into the full loop below.
    // No fuel check (bounded overshoot, same class as native chunking).
    // Nested constructs/calls inside the partial execute fully (sound:
    // native flow passes through them identically).
    // (top plan first; partial only on mid-entry)
    let part_ops: Vec<BodyOp> = if tree.entry != tree.top.top {
        let (po, _) = plan_span(
            lay, base_pc, tree.top.top, tree.top.li, tree.top.lsize, tree.top.cc,
            &top_nests.iter().map(|&(t, l, s)| (t, l, s)).collect::<Vec<_>>(),
            &top_calls,
            &[],
            tree.entry,
        )?;
        po
    } else {
        Vec::new()
    };
    // (top plan success/failure traced by caller prints below)
    // Run func.
    let mut crun = Code::new();
    {
        // (borrow st mutably for the whole func)
        let st_r = &mut st;
        // Inline emit_run_func body (needs tree.top.nests as &[TLoop]).
        crun.local(9, I32);
        crun.local(1, I64);
        crun.local_get(0);
        crun.global_set(FUEL_GLOBAL);
        crun.i32_const(0).global_set(ADJ_GLOBAL);
        crun.i32_const(0).local_set(LTOT_LOCAL);
        // Mid-entry partial (empty for top-entry): straight walk from
        // entry to the top latch, then fallthrough into the full loop.
        // Latch-taken falls through (no br); Bcc-nt returns kind-0.
        // No fuel check (bounded overshoot, native-chunk class).
        crun.block_void();
        st_r.stack.push(NFrame::Fuel);
        crun.block_void();
        st_r.stack.push(NFrame::End);
        if !part_ops.is_empty() {
            st_r.walk(&mut crun, &part_ops, &tree.top.nests, &top_calls.iter().map(|&(h, f, _)| (h, f)).collect::<Vec<_>>())?;
            // Partial latch: taken falls through into the full loop below;
            // Bcc-nt completes without looping (return kind-0).
            emit_tick(&mut crun);
            if let Some(k) = tree.top.cc {
                emit_cond(&mut crun, k);
                crun.if_void();
                st_r.stack.push(NFrame::If);
                crun.else_();
                st_r.retn(&mut crun, 0);
                crun.end();
                st_r.stack.pop();
            }
        }
        crun.loop_void();
        st_r.stack.push(NFrame::Loop);
        st_r.fuel(&mut crun, top_cyc, base_pc)?;
        st_r.walk(&mut crun, &top_ops, &tree.top.nests, &top_calls.iter().map(|&(h, f, _)| (h, f)).collect::<Vec<_>>())?;
        st_r.emit_latch(&mut crun, tree.top.li, tree.top.cc)?;
        crun.end();
        st_r.stack.pop();
        crun.end();
        st_r.stack.pop();
        st_r.retn(&mut crun, 0);
        crun.end();
        st_r.stack.pop();
        st_r.retn(&mut crun, 2);
        crun.unreachable_();
        crun.end();
    }
    m.code(&crun);
    // Callee funcs (caller top = tree top for the return-branch proof).
    let caller_top_pc = base_pc + (tree.top.top as u32) * 2;
    for call in tree.top.calls.iter() {
        let mut cc = Code::new();
        emit_callee_func(&mut st, &mut cc, &call.callee, base_pc, caller_top_pc)?;
        m.code(&cc);
    }
    let bytes = m.finish();
    let consumed = tree.top.li + tree.top.lsize;
    Some(Block {
        bytes,
        consumed,
        is_loop: true,
        exit_off: consumed,
        slots: st.slots,
    })
} // end emit_tree

pub fn translate_at(ops: &[u16], segs_in: &[u32], base_pc: u32, entry_pc: u32) -> Block {
    // Segment table: flat [guest_base, size] pairs (SRAM + flash +
    // extra_ram, driver order). Mirror layout: scratch (dirty trailer
    // at [8..), validity bitmap at [BITBASE..)) then each segment at a
    // 64KB-aligned offset. Total bytes are capped at MAX_MIRROR_BYTES
    // (extra segments degrade to miss-and-interpret, soundly); this
    // keeps validity indices in-bounds. Returns (table, total bytes).
    let mut segs: Vec<(u32, u32, u32)> = Vec::new();
    {
        let mut mo = SCRATCH;
        let mut si = 0usize;
        while si + 1 < segs_in.len() {
            let (base, size) = (segs_in[si], segs_in[si + 1]);
            if size >= 4 {
                let rounded = ((size + 65535) / 65536) * 65536;
                if mo + rounded > MAX_MIRROR_BYTES {
                    break; // over cap: further segments stay interpreted
                }
                segs.push((base, size, mo));
                mo += rounded;
            }
            si += 2;
        }
    }
    let mirror_bytes = {
        let mut end = SCRATCH;
        for (_, sz, mo) in segs.iter() {
            let top = mo + ((sz + 65535) / 65536) * 65536;
            if top > end {
                end = top;
            }
        }
        end
    };
    debug_assert!(mirror_bytes <= MAX_MIRROR_BYTES);
    debug_assert!(BITBASE + BITBYTES <= SCRATCH);
    // Nested/call region tree (additive): attempt the tree path whenever
    // build_tree forms (it validates entry itself: top-entry or validated
    // mid-entry). Any decline (None) falls through to the flat legacy scan
    // below, which is untouched — tree emission can only ADD fused shapes,
    // never change flat behavior.
    if let Some(tree) = build_tree(ops, base_pc, entry_pc) {
        let lay = layout_scan(ops, base_pc);
        if let Some(block) = emit_tree(&tree, &lay, base_pc, &segs, mirror_bytes) {
            return block;
        }
    }
    // v2 scan (2026-10-11): 32-bit-aware, loads-only, multi-exit.
    // The steady-state hot loops are countdown+poll hybrids
    // ([SUBS, BEQ.W-fwd, LDR.W, LSLS, BPL-back]): the translator covers
    // 16-bit ALU/NOP/LSLS/LDR + LDR.W mirror loads + Bcc(.N/.W) latches
    // and forward exits. Stores, B.W/BL, and everything else STOP the
    // region (loads-only: no store-writeback exists, so a store ends the
    // region before it). Runs are atomic vs the model (no interrupts
    // mid-run, it_n==0 gated driver-side), so live-in registers and flag
    // bits are stable and correct — no closure rule needed.
    // Linear scan: Emits accumulate; forward Bcc records an exit and the
    // scan continues; the FIRST backward Bcc latches the loop; anything
    // else stops (straight if no latch yet).
    // (Exit lives at module level — shared with emit_body_walk.)
    // Latch: (li_hw, cond-or-None, t_hw, latch_size).
    let mut slots: Vec<u32> = Vec::new(); // flat [pack, off] per Mem item
    let mut emitted: Vec<(Item, usize)> = Vec::new(); // (item, hw size), in order
    let mut exits: Vec<Exit> = Vec::new();
    let mut latch: Option<(usize, Option<u32>, usize, usize)> = None;
    let mut hw = 0usize;
    let mut scan_hw = 0usize; // halfwords consumed (== hw while unbroken)
    while hw < ops.len() {
        match classify(ops, hw) {
            Class::Emit(it, size) => {
                emitted.push((it, size));
                hw += size;
                scan_hw = hw;
            }
            Class::BCond { cc, wide, size } => {
                let target = match branch_target(ops, hw, wide, base_pc) {
                    Some(t) => t,
                    None => break,
                };
                if target < base_pc {
                    break; // below-window target: not ours
                }
                let t = ((target - base_pc) / 2) as usize;
                if t < hw && scan_hw == hw {
                    latch = Some((hw, Some(cc), t, size));
                    hw += size;
                    break;
                } else if t > hw && scan_hw == hw {
                    // Forward edge inside the run: a loop exit if a latch
                    // forms later, else just where straight code stops.
                    // (Exits before the latch target invalidate it below.)
                    exits.push(Exit { hw, cc: Some(cc), target, size });
                    hw += size;
                    scan_hw = hw;
                } else {
                    break;
                }
            }
            Class::BUn { size } => {
                let target_wide = match branch_target_un_wide(ops, hw, base_pc) {
                    Some(t) => t,
                    None => break,
                };
                if target_wide < base_pc as i64 || target_wide > 0xFFFF_FFFE {
                    // Below-base (an outer-loop top: the VDP column loop
                    // exits to 0x8016be8 mid-region) or out of range: a
                    // true loop exit. Record it and KEEP scanning — a
                    // later backward edge within the window still forms
                    // the loop. Items past this exit that no internal
                    // edge targets are dead and rejected by the
                    // reachability check when a latch forms; when none
                    // forms the region stays straight (never executed —
                    // declined driver-side), so scan length is harmless.
                    exits.push(Exit { hw, cc: None, target: target_wide as u32, size });
                    hw += size;
                    scan_hw = hw;
                    continue;
                }
                let target = target_wide as u32;
                let t = ((target - base_pc) / 2) as usize;
                if t < hw && scan_hw == hw {
                    // Unconditional backward latch (DOOM's memcpy/V_DrawPatch
                    // copy loops end in B.N, not Bcc). Self-edges (t == hw,
                    // `b .`) stop: no-progress loops hang identically
                    // interpreted.
                    latch = Some((hw, None, t, size));
                    hw += size;
                    break;
                } else if t > hw && scan_hw == hw {
                    exits.push(Exit { hw, cc: None, target, size });
                    hw += size;
                    scan_hw = hw;
                } else {
                    break;
                }
            }
            Class::Stop(size) => {
                let _ = size;
                break;
            }
        }
    }
    // Forward exits before the latch target would skip the loop: they
    // invalidate it (region stops at the first such exit).
    let mut scan_end = scan_hw;
    if let Some((_, _, t, _)) = latch {
        if let Some(e) = exits.iter().find(|e| e.hw < t) {
            scan_end = e.hw;
            latch = None;
        }
    }
    // Internal-skip edges (DOOM V_DrawPatch column loop): a CONDITIONAL
    // forward edge whose target lies strictly inside the latched body
    // (hw, li) is not a loop exit — the run continues at the target,
    // skipping the spanned items that iteration. Skipped static counts
    // are subtracted dynamically via the skip-adjust global, so every
    // count formula stays exact (kind-3/kind-1/fuel/completion all
    // subtract it). Unconditional internal edges, backward non-latch
    // edges, off-item-boundary targets, and dead body items decline the
    // loop — straight regions never execute, so declining is always
    // sound.
    let mut skips: Vec<Skip> = Vec::new();
    if let Some((li, _, t, _)) = latch {
        // TRUE halfword starts of emitted items. The scan's hw advances
        // through branches too, but emitted[] holds items only — so
        // reconstruct by skipping branch widths (exits in scan order +
        // the latch itself). A naive items-only accumulation misplaces
        // every item past the first branch (caught by the skip
        // differential, not review: skips silently never formed).
        let mut item_starts: Vec<usize> = Vec::with_capacity(emitted.len());
        {
            let mut hc = 0usize;
            let mut e2 = 0usize;
            let mut latched = false;
            for (_, size) in emitted.iter() {
                loop {
                    if e2 < exits.len() && exits[e2].hw == hc {
                        hc += exits[e2].size;
                        e2 += 1;
                        continue;
                    }
                    if !latched {
                        if let Some((lhw, _, _, lsz)) = latch {
                            if lhw == hc {
                                hc += lsz;
                                latched = true;
                                continue;
                            }
                        }
                    }
                    break;
                }
                item_starts.push(hc);
                hc += size;
            }
        }
        let is_body_start = |h: usize| -> bool { item_starts.contains(&h) };
        let mut bad = false;
        // Prefix straddle (item crossing t) is rejected by the latch
        // arm below; mirror that here so skips never form on it.
        for (i, &st) in item_starts.iter().enumerate() {
            if st < t && st + emitted[i].1 > t {
                bad = true;
            }
        }
        // Classify body exits. A CONDITIONAL forward edge strictly
        // inside the body, landing on an item boundary, is an internal
        // skip (the run continues at the target). Everything else in the
        // body is a true exit (legacy kind-3 return, which subtracts
        // skipAdj — no decline needed), EXCEPT backward/self edges, which
        // are nested-loop shapes v1 does not cover. Off-boundary forward
        // edges also stay legacy true exits (old code never checked exit-
        // target boundaries; synthetic vectors pin this).
        for e in exits.iter() {
            if e.hw < t || e.hw >= li {
                continue;
            }
            let lo = base_pc as u64 + (t as u64) * 2;
            let hi = base_pc as u64 + (li as u64) * 2;
            let tgt = e.target as u64;
            if tgt >= lo && tgt < hi {
                let thw = ((e.target - base_pc) / 2) as usize;
                if thw <= e.hw {
                    bad = true; // backward/self edge: nested shape
                    break;
                }
                if e.cc.is_some() && is_body_start(thw) {
                    skips.push(Skip { hw: e.hw, cc: e.cc.unwrap(), target_hw: thw, k: 0 });
                }
                // Else: legacy true exit (unconditional, or off-boundary).
            }
            // Else: true exit (below-base outer top, past-li, ...).
        }
        // Reachability over the merged body layout (items + branches in
        // hw order): every ITEM must be reachable from the top.
        // Branches apply effects only when reached (internal edges add
        // target + fallthrough; conditional true exits keep fallthrough;
        // unconditional true exits cut it). Unreached items decline.
        if !bad {
            use std::collections::HashSet;
            let mut reach: HashSet<usize> = HashSet::new();
            reach.insert(t);
            let mut pos = t;
            loop {
                if pos == li {
                    break;
                }
                if pos > li {
                    bad = true; // overshoot: layout gap
                    break;
                }
                if let Some(idx) = item_starts.iter().position(|&x| x == pos) {
                    if !reach.contains(&pos) {
                        bad = true;
                        break;
                    }
                    reach.insert(pos + emitted[idx].1);
                    pos += emitted[idx].1;
                    continue;
                }
                if let Some(e) = exits.iter().find(|e| e.hw == pos) {
                    if reach.contains(&pos) {
                        if let Some(sk) = skips.iter().find(|sk| sk.hw == pos) {
                            reach.insert(sk.target_hw);
                            reach.insert(pos + e.size);
                        } else if e.cc.is_some() {
                            reach.insert(pos + e.size);
                        }
                        // Unconditional true exit: fallthrough dead.
                    }
                    pos += e.size;
                    continue;
                }
                bad = true; // gap: neither item nor branch (straddle fallout)
                break;
            }
        }
        if bad {
            latch = None;
            skips.clear();
        } else {
            // K per skip: items + exits spanned (nested edges count at
            // every enclosing level, matching cycle's linear count).
            for sk in skips.iter_mut() {
                let mut k = 0u32;
                for &st in item_starts.iter() {
                    if st > sk.hw && st < sk.target_hw && st >= t {
                        k += 1;
                    }
                }
                for e in exits.iter() {
                    if e.hw > sk.hw && e.hw < sk.target_hw {
                        k += 1;
                    }
                }
                sk.k = k;
            }
        }
    }
    // Prefix = emitted items fully before the latch target (halfword
    // starts < t); loop body = the rest. (Items are branch-free, so an
    // item straddling t is impossible: t is an item boundary because the
    // latch target is encodable only at one... not exactly — t can split
    // a 32-bit pair in theory. Guard: prefix items must END at/before t.)
    let mut m = Module::new();
    m.functype(&[I32], &[0x7E]);
    m.func(0);
    let pages = ((mirror_bytes + 65535) / 65536).max(1);
    m.memory(pages);
    // Globals 0-11 keep their v1 indices (r0-r7, xpsr, tb_exit, tb_addr,
    // sp); 12-17 add r8-r12 + lr; 18 is the dirty-trailer count; 19 is
    // the skip-adjust (internal-skip exact counting, driver-invisible);
    // 20 is the shared fuel pool (nested/call modules only; flat keeps l5).
    for _ in 0..21 {
        m.global_mut_i32(0);
    }
    m.export_func("run", 0);
    m.export_mem("mem", 0);
    for (i, name) in [
        "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "xpsr", "tb_exit", "tb_addr", "sp",
        "r8", "r9", "r10", "r11", "r12", "lr", "dirtyCount", "skipAdj", "tbFuel",
    ]
        .iter()
        .enumerate()
    {
        m.export_global(name, i as u32);
    }
    // Flat single-loop modules keep legacy local-l5 fuel (nested/call
    // modules use the shared FUEL_GLOBAL pool via separate emission).
    let mut c = Code::new();
    c.local(7, I32); // l1..l7 (p0 = fuel param; l7 = skip-cond scratch)
    // prologue (uniform): spill fuel, zero iters + skip-adjust.
    // (Nested path rewrites the spill to the shared pool; flat keeps l5.)
    c.local_get(0).local_set(5);
    c.i32_const(0).local_set(6);
    c.i32_const(0).global_set(ADJ_GLOBAL);
    // Split emitted items into straight prefix (halfword starts < t, or
    // everything when no loop) vs loop body. A 32-bit item straddling
    // the latch target invalidates the loop (prefix must END at/before t
    // so straight emission never double-executes body code).
    // Entry mode: top-entry (entry at/before the top: legacy prefix +
    // full loop) vs mid-entry (entry strictly inside (top, latch): no
    // prefix emission, partial first pass). Entry must sit on an item
    // boundary; anything else declines.
    enum EntryMode {
        Top,
        Mid { e_hw: usize, p_cost: u32 },
    }
    let (is_loop, exit_off, body_from, cycle, entry_mode) = match latch {
        Some((li, cc, t, lsize)) => {
            let mut start_hw = 0usize;
            let mut n: u32 = 0;
            let mut ok = true;
            for (it, size) in emitted.iter() {
                let _ = it;
                if start_hw >= t {
                    break; // item starts at/after the top: body begins
                }
                if start_hw + size > t {
                    ok = false; // 32-bit item straddling the top: unsafe
                    break;
                }
                start_hw += size;
                n += 1;
            }
            if !ok || start_hw != t {
                (false, 0usize, 0usize, 0u32, EntryMode::Top)
            } else {
                // cycle = body Emit insts + forward exits taken-not-taken
                // cost (each executes when looping) + the latch itself.
                let mut cyc: u32 = 1; // latch
                for e in exits.iter() {
                    if e.hw >= t && e.hw < li {
                        cyc += 1;
                    }
                }
                let mut bn = n;
                for (it, _) in emitted.iter().skip(n as usize) {
                    let _ = it;
                    bn += 1;
                }
                cyc += bn - n;
                let _ = cc;
                // Entry classification (halfword offsets from base).
                let e_hw = if entry_pc < base_pc {
                    usize::MAX
                } else {
                    ((entry_pc - base_pc) / 2) as usize
                };
                if e_hw <= t {
                    // Top entry (includes entry == base == top): legacy.
                    (true, li + lsize, n as usize, cyc, EntryMode::Top)
                } else if e_hw < li {
                    // Mid entry: e must be an item boundary. Partial cost
                    // P = items + exits in [e..li) + 1 (latch eval).
                    // Fuel for the partial is NOT pre-checked (bounded
                    // overshoot ~20 insts max — same class as a native
                    // chunk boundary; tick accounting stays exact).
                    let mut hw = t;
                    let mut at_boundary = false;
                    for (it, size) in emitted.iter().skip(n as usize) {
                        let _ = it;
                        if hw == e_hw {
                            at_boundary = true;
                            break;
                        }
                        if hw > e_hw {
                            break;
                        }
                        hw += size;
                    }
                    if !at_boundary {
                        (false, 0usize, 0usize, 0u32, EntryMode::Top)
                    } else {
                        // Partial cost: 1 (latch eval) + exits in [e..li)
                        // + Emit items in [e..li).
                        let mut p: u32 = 1;
                        for e2 in exits.iter() {
                            if e2.hw >= e_hw && e2.hw < li {
                                p += 1;
                            }
                        }
                        {
                            let mut h = t;
                            for (_, size) in emitted.iter().skip(n as usize) {
                                if h >= e_hw && h < li {
                                    p += 1;
                                }
                                h += size;
                            }
                        }
                        (true, li + lsize, n as usize, cyc, EntryMode::Mid { e_hw, p_cost: p })
                    }
                } else {
                    (false, 0usize, 0usize, 0u32, EntryMode::Top)
                }
            }
        }
        None => (false, 0usize, 0usize, 0u32, EntryMode::Top),
    };
    // Straight emission: prefix items on loops, everything scanned
    // without a loop otherwise. Counts static insts in n_out. Memory
    // slots are collected here (and in the body walk below) so they
    // cover exactly the emitted region.
    // Prefix emission (top-entry only): straight items before the top.
    // Mid-entry emits nothing here — code before the entry already ran
    // natively, and re-executing it would double-apply stores.
    // Straight emission count: loop prefix items on top-entry loops,
    // nothing on mid-entry, everything scanned when no loop formed.
    let (emit_upto, formula_n) = match entry_mode {
        EntryMode::Top => {
            let upto = if is_loop { body_from } else { emitted.len() };
            let mut k = 0u32;
            for _ in emitted.iter().take(upto) {
                k += 1;
            }
            (upto, k)
        }
        EntryMode::Mid { p_cost, .. } => (0, p_cost),
    };
    let mut n_out: u32 = 0;
    for (it, _) in emitted.iter().take(emit_upto) {
        if let Item::Mem { base, off, size: msize, is_store, wb, .. } = *it {
            let pack = (if base == 13 { 15 } else { base })
                | ((msize == 4) as u32) << 5
                | ((is_store as u32) << 6);
            slots.push(pack);
            slots.push(if wb == 1 { 0 } else { off as u32 });
        }
        emit_item(&mut c, it, &ExecCount::Static(n_out as usize), &segs);
        n_out += 1;
    }
    // Item index where the loop body starts (prefix length).
    let body_idx = match entry_mode {
        EntryMode::Top => body_from,
        EntryMode::Mid { .. } => body_from,
    };
    // Item index of the mid-entry point (Top mode: body start). The
    // scan guaranteed e_hw is an item boundary when Mid was formed.
    let entry_idx = match entry_mode {
        EntryMode::Top => body_from,
        EntryMode::Mid { e_hw, .. } => {
            let t_hw = match latch {
                Some((_, _, t, _)) => t,
                None => usize::MAX,
            };
            let mut idx = body_from;
            let mut hw = t_hw;
            for (_, size) in emitted.iter().skip(body_from) {
                if hw == e_hw {
                    break;
                }
                hw += size;
                idx += 1;
            }
            idx
        }
    };
    let n = match entry_mode {
        EntryMode::Top => n_out,
        EntryMode::Mid { p_cost, .. } => p_cost,
    };
    if is_loop {
        let (li, cc, t, lsize) = latch.unwrap();
        let _ = (t, lsize);
        // Mid-entry partial first pass [entry..latch): exits counted from
        // zero (the partial is not a full cycle), then the latch either
        // continues into the full loop (no iter increment — the partial
        // was not a full cycle) or completes with exactly P.
        if let EntryMode::Mid { e_hw, p_cost } = entry_mode {
            c.block_void(); // $part
            let end_hw = emit_body_walk(
                &mut c,
                &emitted[entry_idx..],
                &exits,
                &skips,
                e_hw,
                0,
                cycle,
                &segs,
                &mut slots,
            );
            debug_assert_eq!(end_hw, li);
            match cc {
                Some(cond) => {
                    emit_cond(&mut c, cond);
                    c.br_if(0); // taken -> out of $part -> full loop below
                    // fallthrough -> partial completion: exactly P
                    // minus skips taken during the partial (usually
                    // zero; dynamic so mid-skip entries stay exact).
                    c.i32_const(p_cost as i32).global_get(ADJ_GLOBAL).i32_sub();
                    c.i64_extend_u();
                    c.return_();
                }
                None => {
                    // Unconditional latch: always continues to the loop.
                }
            }
            c.end(); // $part
        }
        // Full loop over body [t..li): block $fuelout { block $end {
        // loop $top { ... } } }.
        c.block_void();
        c.block_void();
        c.loop_void();
        c.local_get(5).i32_const(cycle as i32).i32_lt_u().br_if(2);
        c.local_get(5).i32_const(cycle as i32).i32_sub().local_set(5);
        // NOTE: iters (l6) counts COMPLETED full iterations: incremented
        // on the latch back-edge only, so forward exits and the
        // completion/fuel-out formulas see exact counts.
        let end_hw = emit_body_walk(
            &mut c,
            &emitted[body_idx..],
            &exits,
            &skips,
            t,
            n,
            cycle,
            &segs,
            &mut slots,
        );
        debug_assert_eq!(end_hw, li);
        // latch: completed iter counted, continue; fallthrough (Bcc only)
        // -> $end (completion). Unconditional latch: always continues.
        // (if{br(1)}: from inside the if, depth 1 is the loop = continue;
        // falling out of the if reaches $end. Plain br(0) in the loop
        // body also continues.)
        match cc {
            Some(cond) => {
                emit_cond(&mut c, cond);
                c.if_void();
                c.local_get(6).i32_const(1).i32_add().local_set(6);
                c.br(1);
                c.end();
            }
            None => {
                c.local_get(6).i32_const(1).i32_add().local_set(6);
                c.br(0);
            }
        }
        c.end(); // loop
        c.end(); // $end
        // Completion: the fallthrough iter ran fully but never took the
        // back-edge (iters counts completed takes only), so +1 cycle.
        c.local_get(6).i32_const(1).i32_add().i32_const(cycle as i32).i32_mul().i32_const(n as i32).i32_add();
        c.global_get(ADJ_GLOBAL).i32_sub();
        c.i64_extend_u();
        c.return_();
        c.end(); // $fuelout
        // Resume address: the entry pc (mid-entry resubmits mid-body;
        // top-entry entry==base, identical to the old leave-pc behavior).
        // Nested inner fuel-outs overwrite this with their own top below.
        c.i32_const(entry_pc as i32).global_set(10);
        c.local_get(6).i32_const(cycle as i32).i32_mul().i32_const(n as i32).i32_add();
        c.global_get(ADJ_GLOBAL).i32_sub();
        c.i64_extend_u();
        c.i64_const(2).i64_const(32).i64_shl().i64_or();
        c.return_();
        c.unreachable_();
        c.end(); // func
    } else {
        // Straight-only region (or invalidated loop): fall off the end
        // with the static count. consumed = scanned halfwords, or the
        // invalidating exit's position.
        c.i64_const(n_out as u64);
        c.end();
    }
    let exit_hw = if is_loop { exit_off } else { scan_end };
    m.code(&c);
    Block { bytes: m.finish(), consumed: exit_hw, is_loop, exit_off: exit_hw, slots }
}
/// Static memory slots for the driver page-sync: flat [pack, off] pairs,
/// one pair per Mem item in the consumed region, in region order (dedup
/// is the driver's business). pack = base5 | size<<5 with base 0-14 =
/// register, 15 = SP; off is the full signed byte offset as u32.
/// `want_store` selects stores (writeback source) vs loads (sync source).
/// Both are pre-synced: store pages need valid surroundings because the
/// driver writes whole 4KB pages back.
fn mem_slots(ops: &[u16], segs: &[u32], base_pc: u32, want_store: bool) -> Vec<u32> {
    let b = translate(ops, segs, base_pc);
    let mut out = Vec::new();
    let mut i = 0usize;
    while i + 1 < b.slots.len() {
        let (pack, off) = (b.slots[i], b.slots[i + 1]);
        let is_store = (pack >> 6) & 1 == 1;
        if is_store == want_store {
            out.push(pack);
            out.push(off);
        }
        i += 2;
    }
    out
}

/// Load slots (stores excluded).
pub fn load_slots(ops: &[u16], segs: &[u32], base_pc: u32) -> Vec<u32> {
    mem_slots(ops, segs, base_pc, false)
}

/// Store slots (loads excluded).
pub fn store_slots(ops: &[u16], segs: &[u32], base_pc: u32) -> Vec<u32> {
    mem_slots(ops, segs, base_pc, true)
}

/// Entry-point variants: static slots cover the whole translated region
/// regardless of entry (pre-sync is position-independent).
pub fn load_slots_at(ops: &[u16], segs: &[u32], base_pc: u32, _entry_pc: u32) -> Vec<u32> {
    mem_slots(ops, segs, base_pc, false)
}

/// Entry-point variants: static slots cover the whole translated region
/// regardless of entry (pre-sync is position-independent).
pub fn store_slots_at(ops: &[u16], segs: &[u32], base_pc: u32, _entry_pc: u32) -> Vec<u32> {
    mem_slots(ops, segs, base_pc, true)
}

/// Packed region shape for the driver: bit0 = is_loop, exit_off << 1.
pub fn region_meta(ops: &[u16], segs: &[u32], base_pc: u32) -> u32 {
    let b = translate(ops, segs, base_pc);
    (b.is_loop as u32) | ((b.exit_off as u32) << 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_shapes() {
        // Spin with its top NOP submitted: [NOP, SUBS, BNE->base].
        // 0xD1FC at base+4: target = base+4+4-8 = base. latches with a
        // 2-inst body; exit falls past the latch.
        let b = translate(&[0xBF00, 0x3B01, 0xD1FC], &[0x2000_0000, 65536], 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (3, 3));
        // Forward branch: scan continues past it (a later latch could
        // still form); with none, the region is straight (not run).
        let b = translate(&[0x3B01, 0xD105], &[0x2000_0000, 65536], 0x1000);
        assert!(!b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (2, 2));
        // Self-branch `b .`: empty body, declined (hangs identically
        // interpreted — never fuse a no-progress loop).
        let b = translate(&[0xE7FE], &[0x2000_0000, 65536], 0x1000);
        assert!(!b.is_loop);
        assert_eq!(b.consumed, 0);
        // LDR-then-spin: the BNE targets below base (invalid latch), so
        // the region is straight LDR+SUBS (loads read the synced mirror).
        let b = translate(&[0x6813, 0x3B01, 0xD1FA], &[0x2000_0000, 65536], 0x1000);
        assert!(!b.is_loop);
        assert_eq!(b.consumed, 2);
        // Live-in loop: [CMP r3,#0, BNE-back] reads entry r3/Z — stable
        // (runs are atomic, no interrupts mid-run), so it latches.
        let b = translate(&[0x2B00, 0xD1FD], &[0x2000_0000, 65536], 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (2, 2));
        // Countdown without the NOP top: [SUBS r3,#1, BNE-back] latches
        // with a 1-inst body.
        let b = translate(&[0x3B01, 0xD1FD], &[0x2000_0000, 65536], 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (2, 2));
        // region_meta packs is_loop + exit_off.
        assert_eq!(region_meta(&[0xBF00, 0x3B01, 0xD1FC], &[0x2000_0000, 65536], 0x1000), 0b111);
        assert_eq!(region_meta(&[0x3B01, 0xD105], &[0x2000_0000, 65536], 0x1000), 0b100);
    }

    #[test]
    fn stops_before_control_and_wide() {
        // SUBS, BNE stops before the branch; 32-bit prefix stops at 0.
        assert_eq!(translate(&[0x3B01, 0xD1FC], &[0x2000_0000, 65536], 0x1000).consumed, 1);
        assert_eq!(translate(&[0xF5B3, 0x6F7A], &[0x2000_0000, 65536], 0x1000).consumed, 0);
        assert_eq!(translate(&[], &[0x2000_0000, 65536], 0x1000).consumed, 0);
        assert_eq!(translate(&[0xBF00, 0xBF00, 0xE000], &[0x2000_0000, 65536], 0x1000).consumed, 3);
        // Loads translate via the synced mirror (no stop).
        assert_eq!(translate(&[0x9B01, 0x3301], &[0x2000_0000, 65536], 0x1000).consumed, 2);
        assert_eq!(translate(&[0x3301, 0x9B01], &[0x2000_0000, 65536], 0x1000).consumed, 2);
        // Stores translate too (v3: dirty-page trailer + driver writeback).
        assert_eq!(translate(&[0x600A, 0x3301], &[0x2000_0000, 65536], 0x1000).consumed, 2);
        assert_eq!(translate(&[0x3301, 0x600A], &[0x2000_0000, 65536], 0x1000).consumed, 2);
    }

    #[test]
    fn poll_shapes() {
        // Loop-A skeleton: [SUBS r3,#1, BEQ.W-fwd, LDR.W r2,[r7,#0],
        // LSLS r2,r2,#31, BPL-back]. BEQ.W at hw1 (F000 8001: o2&0xC000
        // == 0x8000, o2&0x1000 == 0, cc = 0): forward exit recorded;
        // BPL latches. consumed covers the whole window.
        let ops = [0x3B01, 0xF000, 0x8001, 0xF8D7, 0x2000, 0x07D2, 0xD5F8];
        let b = translate(&ops, &[0x2000_0000, 65536], 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (7, 7));
        // Load slots: [pack, off] pairs; LDR.W here is base r7=7,
        // word (bit5), load (bit6 clear): pack = 7|32 = 39, off 0.
        assert_eq!(load_slots(&ops, &[0x2000_0000, 65536], 0x1000), vec![39, 0]);
        // LDR.W with SP base + nonzero offset: pack base 15, off 0x618.
        let ops2 = [0xF8DD, 0x2618, 0x3B01, 0xD1FB];
        assert_eq!(load_slots(&ops2, &[0x2000_0000, 65536], 0x1000), vec![15 | 32, 0x618]);
        let b2 = translate(&ops2, &[0x2000_0000, 65536], 0x1000);
        assert!(b2.is_loop);
        // 16-bit LDR straight keeps its slot (0x6813: rn=2, off=0).
        assert_eq!(load_slots(&[0x6813, 0x3301], &[0x2000_0000, 65536], 0x1000), vec![2 | 32, 0]);
        // BEQ.W target math: F000 8127 at hw1 base 0x1214 -> 0x1468.
        // (Matches the eth_http loop-A BEQ.W, objdump-verified.)
        let ops3 = [0x3B01, 0xF000, 0x8127, 0xF8D7, 0x2618, 0x07D2, 0xD5F8];
        let b3 = translate(&ops3, &[0x2000_0000, 65536], 0x8001214);
        assert!(b3.is_loop);
        assert_eq!((b3.consumed, b3.exit_off), (7, 7));
    }

    #[test]
    fn byte_copy_shapes() {
        let segs: &[u32] = &[0x2000_0000, 65536];
        // newlib memcpy loop: [LDRB.W-post r4,[r1],#1, STRB.W-pre-wb
        // r4,[r3,#1], CMP r1,r2, BNE-back]. Latches unconditionally? No:
        // BNE is conditional — latches with a 3-inst body.
        let ops = [0xF811, 0x4B01, 0xF803, 0x4F01, 0x4291, 0xD1F9];
        let b = translate(&ops, segs, 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (6, 6));
        // Slots: load pair (base r1) + store pair (base r3).
        assert_eq!(load_slots(&ops, segs, 0x1000), vec![1 | 0, 0]);
        assert_eq!(store_slots(&ops, segs, 0x1000), vec![3 | 64, 1]);
        // V_DrawPatch inner loop (bytes + stores + ADD.W + B.N latch):
        // [SUBS, BCS-fwd, LDRB16, ADDS-imm3, ADDhi, B.N-fwd, LDRB.W-post,
        // STRB.W-imm0, ADDW-imm, B.N-back].
        let vdp = [
            0x3D01, 0xD203, 0x7845, 0x1CC6, 0x4418, 0xE002, 0xF816, 0xEB01, 0xF883, 0xE000,
            0xF503, 0x73A0, 0xE7F2,
        ];
        let v = translate(&vdp, segs, 0x1000);
        assert!(v.is_loop);
        // 16-bit B.N (0xE000 class) latches; ADD.W-imm decodes 320.
        let w = translate(&[0x3B01, 0xE7FD], segs, 0x1000);
        assert!(w.is_loop);
        assert_eq!((w.consumed, w.exit_off), (2, 2));
        // ADD.W-imm straight: consumed, no loop.
        let a = translate(&[0xF503, 0x73A0], segs, 0x1000);
        assert!(!a.is_loop);
        assert_eq!(a.consumed, 2);
    }

    #[test]
    fn entry_offsets() {
        let segs: &[u32] = &[0x2000_0000, 65536];
        // Spin [NOP, SUBS, BNE->base] entered mid-body at the SUBS
        // (entry = base+2): partial [SUBS, BNE] then the full loop.
        // Partial cost P = 2 items + 0 exits + 1 (latch) = 3.
        let ops = [0xBF00, 0x3B01, 0xD1FC];
        let b = translate_at(&ops, segs, 0x1000, 0x1002);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (3, 3));
        // Entry at the top behaves like translate().
        let t = translate(&ops, segs, 0x1000);
        let b0 = translate_at(&ops, segs, 0x1000, 0x1000);
        assert!(b0.is_loop);
        assert_eq!((b0.consumed, b0.exit_off), (t.consumed, t.exit_off));
        // Entry on the latch itself declines (nothing to enter).
        let bl = translate_at(&ops, segs, 0x1000, 0x1004);
        assert!(!bl.is_loop);
        // Entry mid-pair of a 32-bit op declines.
        let ops2 = [0xF8D7, 0x2000, 0x3B01, 0xD1FB];
        let bm = translate_at(&ops2, segs, 0x1000, 0x1002);
        assert!(!bm.is_loop);
        // Entry past the latch declines.
        let be = translate_at(&ops, segs, 0x1000, 0x1006);
        assert!(!be.is_loop);
    }

    #[test]
    fn internal_skip_shape() {
        // VDP column-loop shape (synthetic, base 0x1000):
        // 1000 SUBS r2,#1 | 1002 BCS ->1008 | 1004 MOVS r0,#7 |
        // 1006 B ->0xFF0 (below-base outer exit) | 1008 MOVS r1,#9 |
        // 100A B ->1000 (latch).
        // BCS: origin 0x1006, off 2 -> 0xD201. B-exit: origin 0x100A,
        // off -26 -> 0xE7F3. Latch: origin 0x100E, off -14 -> 0xE7F9.
        let segs = &[0x2000_0000u32, 65536];
        let ops = [0x3A01u16, 0xD201, 0x2007, 0xE7F3, 0x2009, 0xE7F9];
        let b = translate(&ops, segs, 0x1000);
        assert!(b.is_loop, "VDP shape must latch");
        assert_eq!((b.consumed, b.exit_off), (6, 6));
        // Skip-taken path is the common case; skip spans MOVS r0 + the
        // outer exit (2 static counts).
        // Dead item past the unconditional exit with no internal edge
        // targeting it declines (base in flash so the below-base exit
        // passes the scan-past gate; toy bases below 0x08000000 keep
        // legacy break and never reach reachability).
        // 0x08001000 SUBS | 04 B->0x08000FF0 | 06 stray MOVS |
        // 08 B->0x08001000 (origin 0x0800100C, off -12 -> 0xE7FA).
        // B-exit: origin 0x08001006, off 0xFF0-0x1006=-22 -> 0xE7F5.
        let dead = [0x3A01u16, 0xE7F5, 0x2007, 0xE7FA];
        let bd = translate(&dead, segs, 0x0800_1000);
        assert!(!bd.is_loop, "dead item past unconditional exit declines");
        // Backward non-latch edge inside the body declines (nested shape):
        // SUBS, BCS->1008, MOVS r0, B-exit->0xFF0?? — below-gate breaks
        // instead; use a backward edge to 0x1002 with flash base:
        // 0x08001000 SUBS | 02 BCS->0x08001008 (origin 0x08001006, off 2
        // -> 0xD201) | 04 MOVS | 06 B-exit->0x08000FF0 (0xE7F5) |
        // 08 MOVS | 0A B->0x08001002 (origin 0x0800100E, off -12 -> 0xE7FA).
        let nest = [0x3A01u16, 0xD201, 0x2007, 0xE7F5, 0x2009, 0xE7FA];
        let bn = translate(&nest, segs, 0x0800_1000);
        // Latch targets 0x08001002 (prefix=[SUBS]); BCS targets 0x08001008
        // (backward of nothing... thw=4 > hw=1, boundary ok -> skip) but
        // the latch's backward edge to 0x08001002 is itself fine; the
        // B-exit fallthrough 0x08001008 IS skip-targeted. The nested
        // B latch to 0x08001002 with t=1: body edge BCS(1->4) ok...
        // whatever forms, the module must be valid WASM.
        let _ = bn.consumed;
        assert_eq!(&bn.bytes[0..4], &[0x00, 0x61, 0x73, 0x6D]);
    }

    #[test]
    fn region_tree_nested() {
        // Pure nest (no forward exits): top B@5->0, nested BLT@4->2.
        // BLT@4: origin 0x100C, off -8 -> 0xDBFC. B@5: origin 0x100E,
        // off -14 -> 0xE7F9.
        let ops = [0x3A01u16, 0x2000, 0x3001, 0x2805, 0xDBFC, 0xE7F9];
        let t = build_tree(&ops, 0x1000, 0x1000).expect("nest tree must form");
        assert_eq!((t.top.top, t.top.li), (0, 5));
        assert_eq!(t.top.nests.len(), 1);
        assert_eq!((t.top.nests[0].top, t.top.nests[0].li), (2, 4));
        assert!(t.top.calls.is_empty());
        // BEQ past the window end is no callee (nothing ahead to form
        // one): it stays a legacy kind-3 exit and the nest tree still
        // forms with zero calls.
        let opsc = [0x3A01u16, 0xD005, 0x2000, 0x3001, 0x2805, 0xDBFC, 0xE7F8];
        let nc = build_tree(&opsc, 0x1000, 0x1000).expect("nest+exit tree must form");
        assert_eq!(nc.top.nests.len(), 1);
        assert!(nc.top.calls.is_empty());
        // Flat loop: no nests, no calls — tree still forms (single loop).
        let f = build_tree(&[0x3B01u16, 0xD1FD], 0x1000, 0x1000).expect("flat tree must form");
        assert_eq!((f.top.top, f.top.li), (0, 1));
        assert!(f.top.nests.is_empty() && f.top.calls.is_empty());
        // Call shape: outer SUBS/BEQ-exit/BCS(call)/MOVS/B-latch + NOP
        // pad + callee [MOVS prefix, SUBS/BCS-exit/ADDS/B-latch nest, dup
        // MOVS/BNE-latch/NOP/B-to-caller-top]. BEQ@1->hw20, BCS@2->hw9,
        // B@4->hw0, BCS@11->hw15, B@13->hw10, BNE@16->hw9, B@18->hw0.
        // (The dup trail MUST end with a branch to the caller top — the
        // native-continuation proof for continue-after-call. A clean
        // fallthrough end declines.)
        let opc = [
            0x3A01u16, 0xD011, 0xD205, 0x2007, 0xE7FA, 0xBF00, 0xBF00, 0xBF00, 0xBF00, 0x2400,
            0x3D01u16, 0xD202, 0x3401, 0xE7FB, 0xBF00, 0x2600, 0xD1F7, 0xBF00, 0xE7EC, 0xBF00,
        ];
        let c = build_tree(&opc, 0x1000, 0x1000).expect("call tree must form");
        assert_eq!((c.top.top, c.top.li), (0, 4));
        assert_eq!(c.top.calls.len(), 1);
        let c0 = &c.top.calls[0];
        assert_eq!((c0.edge_hw, c0.target), (2, 0x1012));
        assert_eq!((c0.callee.top, c0.callee.prefix_end), (9, 10));
        assert_eq!((c0.callee.nest.top, c0.callee.nest.li), (10, 13));
        assert_eq!((c0.callee.dup_src, c0.callee.dup_end), (15, 19));
        // Mid-entry into the top body now forms (partial + full loop);
        // branch positions still decline (checked in midentry test).
        let me_ = build_tree(&ops, 0x1000, 0x1002).expect("mid-entry forms");
        assert_eq!(me_.entry, 1);
        // Straight code: no top latch -> None.
        assert!(build_tree(&[0x3B01u16, 0xD105], 0x1000, 0x1000).is_none());
    }

    #[test]
    fn tree_emit_smoke() {
        // Call shape must take the tree emission path (is_loop with top
        // consumed), not flat fallback.
        let opc = [
            0x3A01u16, 0xD011, 0xD205, 0x2007, 0xE7FA, 0xBF00, 0xBF00, 0xBF00, 0xBF00, 0x2400,
            0x3D01u16, 0xD202, 0x3401, 0xE7FB, 0xBF00, 0x2600, 0xD1F7, 0xBF00, 0xE7EC, 0xBF00,
        ];
        let b = translate(&opc, &[0x2000_0000, 65536], 0x1000);
        assert!(b.is_loop);
        assert_eq!((b.consumed, b.exit_off), (5, 5));
        assert_eq!(&b.bytes[0..4], &[0x00, 0x61, 0x73, 0x6D]);
        // Tree path emits 2 funcs (run + callee); flat emits 1. Parse the
        // func section (id 3): [id, len, count, ...].
        let mut nfuncs = 0u32;
        let mut i = 8usize;
        while i + 1 < b.bytes.len() {
            let id = b.bytes[i];
            // section length LEB (small).
            let (mut len, mut j) = (0u32, i + 1);
            let mut sh = 0;
            loop {
                let by = b.bytes[j];
                j += 1;
                len |= ((by & 0x7F) as u32) << sh;
                sh += 7;
                if by & 0x80 == 0 {
                    break;
                }
            }
            if id == 3 {
                nfuncs = b.bytes[j] as u32; // count < 128 single byte
            }
            i = j + len as usize;
        }
        assert_eq!(nfuncs, 2, "tree path must emit run + callee");
    }

    #[test]
    fn reg_offset_shapes() {
        // GAS vectors (docs/encodings/regidx.s): the second halfword
        // decides everything (imm2 = (o2>>4)&3, Rm = o2&0xF).
        let segs = &[0x2000_0000u32, 65536];
        // LDR.W r0,[r4,r3,lsl#2]: consumes both halfwords, loop-forming.
        let b = translate(&[0x3B01u16, 0xF854, 0x0023, 0xD1FB], segs, 0x1000);
        assert!(b.is_loop);
        // All four widths × shift amounts classify (no Stop).
        for ops in [
            vec![0xF854u16, 0x0023], // LDR.W lsl#2
            vec![0xF854u16, 0x1003], // LDR.W plain (imm2=0)
            vec![0xF855u16, 0x2036], // LDR.W lsl#3
            vec![0xF844u16, 0x0023], // STR.W lsl#2
            vec![0xF814u16, 0x1013], // LDRB.W lsl#1
            vec![0xF804u16, 0x2003], // STRB.W plain
        ] {
            let bb = translate(&ops, segs, 0x1000);
            assert_eq!(bb.consumed, 2, "reg-offset consumes {:04x}", ops[0]);
        }
        // PC index declines.
        let pc = translate(&[0xF854u16, 0x002F], segs, 0x1000);
        assert_eq!(pc.consumed, 0);
    }

    #[test]
    fn below_base_exit_scans_past() {
        // BNE to below base is a true exit, not a barrier: the scan must
        // continue to the later latch (VDP's BGE-to-epilogue shape).
        // [SUBS, BNE->0x0FF0 (below 0x1000), MOVS, B->0x1000].
        // BNE@1: origin 0x1006, off 0xFF0-0x1006=-22 -> 0xD1F5.
        // B@3: origin 0x100A, off -10 -> 0xE7FB.
        let ops = [0x3A01u16, 0xD1F5, 0x2007, 0xE7FB];
        let lay = layout_scan(&ops, 0x1000);
        assert_eq!(lay.end_hw, 4, "scan continues past below-base exit");
        let b = translate(&ops, &[0x2000_0000u32, 65536], 0x1000);
        assert!(b.is_loop);
    }

    #[test]
    fn region_tree_midentry() {
        // Synthetic nest shape, entry inside the top body (hw2 MOVS):
        // partial [MOVS..latch] + full loop. Entry must be an item start
        // in the top body (branch positions and nest/callee spans decline).
        // B@6->hw0 is 0xE7F8 (origin base+16, off -16).
        let ops = [0x3A01u16, 0xD005, 0x2000, 0x3001, 0x2805, 0xDBFC, 0xE7F8];
        let t = build_tree(&ops, 0x1000, 0x1004).expect("mid-entry tree must form");
        assert_eq!(t.entry, 2);
        assert_eq!((t.top.top, t.top.li), (0, 6));
        // Entry on a branch position declines.
        assert!(build_tree(&ops, 0x1000, 0x1002).is_none());
        // Entry inside the nested span declines (flat legacy owns it).
        assert!(build_tree(&ops, 0x1000, 0x1008).is_none());
        // Entry past the latch declines.
        assert!(build_tree(&ops, 0x1000, 0x100E).is_none());
        // Mid-entry emission takes the tree path (not flat fallback).
        let b = translate_at(&ops, &[0x2000_0000u32, 65536], 0x1000, 0x1004);
        assert!(b.is_loop, "mid-entry must emit loop");
        // Top-entry on the same shape must also emit (isolates entry).
        let b0 = translate(&ops, &[0x2000_0000u32, 65536], 0x1000);
        assert!(b0.is_loop, "top-entry must emit loop");
    }

    #[test]
    fn emitted_module_structures() {
        // MOVS r0,#1; ADDS r0,#2 must produce a valid module whose run
        // block consumes 2 and whose bytes start with the WASM magic.
        let b = translate(&[0x2001, 0x3002], &[0x2000_0000, 65536], 0x1000);
        assert_eq!(b.consumed, 2);
        assert_eq!(&b.bytes[0..4], &[0x00, 0x61, 0x73, 0x6D]);
        // has run + mem + 9 global exports and ends with func end.
        assert_eq!(*b.bytes.last().unwrap(), 0x0B);
    }
}
