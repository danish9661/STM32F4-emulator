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

use super::tb_emit::{Code, Module, I32};

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

/// Emit one body walk over `items` starting at halfword `start_hw`,
/// matching forward exits by position. `n_count` is the Dynamic-count n
/// (0 for a partial first pass, prefix-or-P for full iterations).
/// Returns the end halfword. Slots are collected for emitted Mem items.
fn emit_body_walk(
    c: &mut Code,
    items: &[(Item, usize)],
    exits: &[Exit],
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
    for (it, size) in items.iter() {
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
}

/// Emit the executed-count value for a miss/forward exit. Stack out: [i64].
fn emit_exec_count(c: &mut Code, exec: &ExecCount) {
    match *exec {
        ExecCount::Static(k) => {
            c.i64_const(k as u64);
        }
        ExecCount::Dynamic { n, cycle, partial } => {
            c.local_get(6).i32_const(cycle as i32).i32_mul().i32_const((n + partial) as i32).i32_add();
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
    Mem { base: u32, off: i32, rt: u32, size: u32, is_store: bool, wb: u32 },
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
        Item::Mem { base, off, rt, size, is_store, wb } => {
            let bg = reg_g(base);
            let rtg = reg_g(rt);
            // l2 = base value; l3 = access address.
            c.global_get(bg).local_tee(2);
            if wb == 1 {
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
    // v2 scan (2026-10-11): 32-bit-aware, loads-only, multi-exit.
    // The steady-state hot loops are countdown+poll hybrids
    // ([SUBS, BEQ.W-fwd, LDR.W, LSLS, BPL-back]): the translator covers
    // 16-bit ALU/NOP/LSLS/LDR + LDR.W mirror loads + Bcc(.N/.W) latches
    // and forward exits. Stores, B.W/BL, and everything else STOP the
    // region (loads-only: no store-writeback exists, so a store ends the
    // region before it). Runs are atomic vs the model (no interrupts
    // mid-run, it_n==0 gated driver-side), so live-in registers and flag
    // bits are stable and correct — no closure rule needed.
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
                Item::Mem { base: rn, off, rt, size: 1, is_store, wb: 0 },
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
                Item::Mem { base: rn, off, rt, size: 4, is_store: true, wb: 0 },
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
                    Item::Mem { base: rn, off: (o2 & 0xFFF) as i32, rt, size, is_store, wb: 0 },
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
                    Item::Mem { base: rn, off, rt, size, is_store, wb },
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
                let target = match branch_target_un(ops, hw, base_pc) {
                    Some(t) => t,
                    None => break,
                };
                if target < base_pc {
                    break;
                }
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
    // sp); 12-17 add r8-r12 + lr; 18 is the dirty-trailer count.
    for _ in 0..19 {
        m.global_mut_i32(0);
    }
    m.export_func("run", 0);
    m.export_mem("mem", 0);
    for (i, name) in [
        "r0", "r1", "r2", "r3", "r4", "r5", "r6", "r7", "xpsr", "tb_exit", "tb_addr", "sp",
        "r8", "r9", "r10", "r11", "r12", "lr", "dirtyCount",
    ]
        .iter()
        .enumerate()
    {
        m.export_global(name, i as u32);
    }
    let mut c = Code::new();
    c.local(6, I32); // l1..l6 (p0 = fuel param)
    // prologue (uniform): spill fuel, zero iters.
    c.local_get(0).local_set(5);
    c.i32_const(0).local_set(6);
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
                    // fallthrough -> partial completion with exactly P:
                    c.i64_const(p_cost as u64);
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
        c.i64_extend_u();
        c.return_();
        c.end(); // $fuelout
        c.local_get(6).i32_const(cycle as i32).i32_mul().i32_const(n as i32).i32_add();
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
