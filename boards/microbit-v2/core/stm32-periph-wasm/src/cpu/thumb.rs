//! WASM-native Thumb-2 (Cortex-M4) interpreter core.
//!
//! Decodes and executes integer Thumb/Thumb-2 instructions against a
//! [`Memory`](super::mem::Memory) and the shared peripheral model. Every
//! encoding here was verified against `arm-none-eabi-as`/`objdump` output
//! for the repo's own firmware (see the `t32.s`/`it3.s` probes): the op
//! nibble `X=(op1>>5)&0xF` maps `{0:AND,1:BIC,2:ORR,3:MVN,4:EOR,8:ADD,
//! 10:ADC,11:SBC,13:SUB,14:RSB}` uniformly across the F (modified-immediate)
//! and EA/EB (shifted-register) groups; `S=(op1>>4)&1`; `Rn=op1&0xF`.
//! `ThumbExpandImm` follows the ARM ARM (rotation uses `'1':imm12[6:0]`
//! rotated by `imm12[11:7]`). IT predication: instruction j>=2 uses `cond`
//! iff mask bit (5-j) equals cond bit 0 (verified against GAS for 13
//! IT forms — identical patterns assemble to different masks per cond).
//!
//! Anything not yet implemented (SVC, coprocessor, FPU, exception entry,
//! RSC/SRS, ...) records a [`CpuFault`](super::CpuFault) and stops, so gaps
//! are loud and precisely located instead of silently wrong.

use super::{mem::Memory, Cpu};
use crate::system::WasmSystem;

pub(crate) fn len(op: u16) -> usize {
    let t = op >> 11;
    if t == 0b11101 || t == 0b11110 || t == 0b11111 {
        4
    } else {
        2
    }
}

#[inline]
fn sx(v: u32, b: u32) -> u32 {
    let s = 32 - b;
    ((v as i32) << s >> s) as u32
}
/// Signed 32-bit saturate of a 64-bit value. Returns (result, saturated).
#[inline]
fn sat32x(w: i64) -> (i32, bool) {
    if w > i32::MAX as i64 {
        (i32::MAX, true)
    } else if w < i32::MIN as i64 {
        (i32::MIN, true)
    } else {
        (w as i32, false)
    }
}
/// Saturate i64 `v` to a `bits`-wide lane (8/16), signed or unsigned.
/// Returns (masked lane value, saturated?). Used by the parallel Q/UQ ops.
#[inline]
fn sat_lane(v: i64, bits: u32, unsigned: bool) -> (u32, bool) {
    let (lo, hi) = if unsigned {
        (0i64, (1i64 << bits) - 1)
    } else {
        (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1)
    };
    if v > hi {
        (hi as u32 & ((1 << bits) - 1), true)
    } else if v < lo {
        (lo as u32 & ((1 << bits) - 1), true)
    } else {
        (v as u32 & ((1 << bits) - 1), false)
    }
}
/// One 16-bit lane combine for the parallel Q/UQ/SH/UH ops (ADD16/SUB16/
/// ASX/SAX families). `ah`/`bh` are raw 16-bit lane values; `sub` selects
/// subtract (vs add); `flavor` is the o2[7:4] nibble (1=Q signed-saturate,
/// 5=UQ unsigned-saturate, 2=SH halve-arithmetic, 6=UH halve-logical).
/// Returns (masked 16-bit result, saturated?).
#[inline]
fn lane16(ah: u32, bh: u32, sub: bool, flavor: u32) -> (u32, bool) {
    match flavor {
        1 => {
            let (a, b) = (ah as i16 as i64, bh as i16 as i64);
            sat_lane(if sub { a - b } else { a + b }, 16, false)
        }
        5 => {
            let (a, b) = (ah as i64, bh as i64);
            sat_lane(if sub { a - b } else { a + b }, 16, true)
        }
        2 => {
            let (a, b) = (ah as i16 as i32, bh as i16 as i32);
            (((if sub { a - b } else { a + b }) >> 1) as u32 & 0xFFFF, false)
        }
        _ => {
            (((if sub {
                ah.wrapping_sub(bh)
            } else {
                ah.wrapping_add(bh)
            }) >> 1)
                & 0xFFFF,
            false)
        }
    }
}
#[inline]
fn ror32(v: u32, s: u32) -> u32 {
    let s = s & 31;
    if s == 0 {
        v
    } else {
        (v >> s) | (v << (32 - s))
    }
}
/// VFP immediate expansion (ARM ARM VFPExpandImm, N=32): encodable value =
/// sign:exponent:fraction with exp = NOT(b):b*5:c:d. Pinned by GAS probes:
/// 0x70->1.0, 0x00->2.0, 0xE0->-0.5, 0x1B->6.75 (fpu.s/fpu7.s).
#[inline]
fn vfp_expand_imm(imm8: u32) -> u32 {
    let b = (imm8 >> 6) & 1;
    let exp = (((b ^ 1) << 7) | (b << 6) | (b << 5) | (b << 4) | (b << 3) | (b << 2)
        | (((imm8 >> 5) & 1) << 1)
        | ((imm8 >> 4) & 1)) as u32;
    (((imm8 >> 7) & 1) << 31) | (exp << 23) | ((imm8 & 0xF) << 19)
}

// ---- VFPv4-SP datapath helpers (FPU (c)) ----
// FPSCR cumulative exception flags.
const FPSCR_IOC: u32 = 1; // invalid operation
const FPSCR_DZC: u32 = 1 << 1; // divide by zero
const FPSCR_OFC: u32 = 1 << 2; // overflow
const FPSCR_UFC: u32 = 1 << 3; // underflow (v1: subnormal result; see below)
const FPSCR_IXC: u32 = 1 << 4; // inexact (v1: vcvt/f16 only, + OFC/UFC)

#[inline]
fn fpu_rmode(fpscr: u32) -> u32 {
    (fpscr >> 22) & 3 // 0 RNE, 1 +inf, 2 -inf, 3 zero
}
#[inline]
fn f32_subnormal(w: u32) -> bool {
    w & 0x7F800000 == 0 && w & 0x007F_FFFF != 0
}
#[inline]
fn f32_nan(w: u32) -> bool {
    w & 0x7F800000 == 0x7F800000 && w & 0x007F_FFFF != 0
}
#[inline]
fn f32_snan(w: u32) -> bool {
    f32_nan(w) && w & 0x0040_0000 == 0
}
#[inline]
fn f32_inf(w: u32) -> bool {
    w & 0x7FFF_FFFF == 0x7F800000
}
/// Flush-to-zero (FPSCR FZ, bit 24): subnormal inputs become signed zero.
/// Sign-preserving, per the ARM ARM flush pseudocode. Documented choice.
#[inline]
fn fpu_flush(w: u32, fpscr: u32) -> u32 {
    if fpscr & (1 << 24) != 0 && f32_subnormal(w) {
        w & 0x8000_0000
    } else {
        w
    }
}
/// Default NaN (FPSCR DN, bit 25): any NaN result is 0x7FC00000.
#[inline]
fn fpu_dn(w: u32, fpscr: u32) -> u32 {
    if fpscr & (1 << 25) != 0 && f32_nan(w) {
        0x7FC0_0000
    } else {
        w
    }
}
/// NaN-operand scan. Returns Some((result, ioc)) when any input is NaN:
/// SNaN anywhere -> IOC (+ DN ? default : quieted first SNaN); else the
/// first QNaN propagates quietly. None = no NaN inputs.
fn fpu_nan_scan(fpscr: u32, ops: &[u32]) -> Option<(u32, u32)> {
    let mut qnan: Option<u32> = None;
    for &o in ops {
        if f32_snan(o) {
            let r = if fpscr & (1 << 25) != 0 { 0x7FC0_0000 } else { o | 0x0040_0000 };
            return Some((r, FPSCR_IOC));
        }
        if qnan.is_none() && f32_nan(o) {
            qnan = Some(o);
        }
    }
    qnan.map(|q| (fpu_dn(q, fpscr), 0))
}
/// Overflow/underflow flags for a computed f32 result with all-finite,
/// non-NaN inputs (callers handle inf-input and invalid cases explicitly).
/// v1 approximation (documented): subnormal nonzero result -> UFC (+IXC);
/// +-inf result -> OFC (+IXC). Plain rounding never touches IXC here.
fn fpu_ou_flags(r: u32) -> u32 {
    if f32_inf(r) {
        FPSCR_OFC | FPSCR_IXC
    } else if f32_subnormal(r) {
        FPSCR_UFC | FPSCR_IXC
    } else {
        0
    }
}
/// Final rounding for add/sub/mul/div/sqrt. IXC fires whenever the
/// delivered result differs from the infinitely-precise value (exactness
/// via f64: add/sub/mul of f32 operands are exact in f64; div/sqrt carry
/// the standard double-rounding caveat — truth within half-f64-ulp of an
/// f32 boundary can mis-set it — documented). RNE takes the f32 result
/// directly (bit-identical to the historical path); other modes round the
/// f64-exact value.
fn fpu_final(fpscr: u32, r_rne: u32, exact: f64) -> (u32, u32) {
    if fpu_rmode(fpscr) == 0 {
        let mut fl = fpu_ou_flags(r_rne);
        if f32::from_bits(r_rne) as f64 != exact {
            fl |= FPSCR_IXC;
        }
        return (fpu_dn(r_rne, fpscr), fl);
    }
    let (rb, inexact) = fpu_round_f32(exact, fpu_rmode(fpscr));
    let mut fl = fpu_ou_flags(rb);
    if inexact {
        fl |= FPSCR_IXC;
    }
    (fpu_dn(rb, fpscr), fl)
}
/// f32 add/sub (sub flips b). Invalid (inf + -inf) -> NaN + IOC.
fn fpu_add(fpscr: u32, a: u32, b: u32, sub: bool) -> (u32, u32) {
    let a = fpu_flush(a, fpscr);
    let b = fpu_flush(b, fpscr);
    if let Some(n) = fpu_nan_scan(fpscr, &[a, b]) {
        return n;
    }
    let af = f32::from_bits(a);
    let bf = f32::from_bits(b);
    if af.is_infinite() && bf.is_infinite() && (af.is_sign_negative() != bf.is_sign_negative()) == !sub {
        return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
    }
    let r = if sub { af - bf } else { af + bf };
    let exact = if sub { af as f64 - bf as f64 } else { af as f64 + bf as f64 };
    fpu_final(fpscr, r.to_bits(), exact)
}
/// f32 multiply (neg flips an input sign up front — exact, so directed
/// rounding sees final signs). 0*inf -> NaN + IOC.
fn fpu_mul(fpscr: u32, a: u32, b: u32, neg: bool) -> (u32, u32) {
    let a = fpu_flush(a, fpscr);
    let b = fpu_flush(b, fpscr);
    let a = if neg { a ^ 0x8000_0000 } else { a };
    if let Some(n) = fpu_nan_scan(fpscr, &[a, b]) {
        return n;
    }
    let af = f32::from_bits(a);
    let bf = f32::from_bits(b);
    if (af == 0.0 && bf.is_infinite()) || (af.is_infinite() && bf == 0.0) {
        return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
    }
    fpu_final(fpscr, (af * bf).to_bits(), af as f64 * bf as f64)
}
/// f32 divide. 0/0 and inf/inf -> NaN + IOC; x/0 -> inf + DZC.
fn fpu_div(fpscr: u32, a: u32, b: u32) -> (u32, u32) {
    let a = fpu_flush(a, fpscr);
    let b = fpu_flush(b, fpscr);
    if let Some(n) = fpu_nan_scan(fpscr, &[a, b]) {
        return n;
    }
    let af = f32::from_bits(a);
    let bf = f32::from_bits(b);
    if bf == 0.0 {
        if af == 0.0 {
            return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
        }
        let r = (if af.is_sign_negative() != bf.is_sign_negative() { f32::NEG_INFINITY } else { f32::INFINITY }).to_bits();
        return (r, FPSCR_DZC);
    }
    if af.is_infinite() && bf.is_infinite() {
        return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
    }
    if bf.is_infinite() {
        // Finite/inf: exact zero, but the true quotient is tiny-nonzero.
        let r = (if af.is_sign_negative() != bf.is_sign_negative() { -0.0 } else { 0.0f32 }).to_bits();
        let fl = if af == 0.0 { 0 } else { FPSCR_UFC | FPSCR_IXC };
        return (r, fl);
    }
    if fpu_rmode(fpscr) == 0 {
        let rb = (af / bf).to_bits();
        fpu_final(fpscr, rb, af as f64 / bf as f64)
    } else {
        // Directed: exact integer long division (no f64 rounding anywhere).
        let (kept, sticky, exp, rs) = fpu_div_exact(a, b);
        fpu_pack(kept, sticky, exp, rs, fpu_rmode(fpscr))
    }
}
/// Accumulate: acc +/- (a*b), unfused (separate f32 mul then add, like
/// silicon — never mul_add). VNMLA/VNMLS negation is folded into the
/// operand signs up front (exact).
fn fpu_mla(fpscr: u32, acc: u32, a: u32, b: u32, sub: bool, neg: bool) -> (u32, u32) {
    // Negation is exact: fold it into the operand signs up front so the
    // add's zero-sign rules and directed rounding see the final signs.
    let (acc, a) = if neg { (acc ^ 0x8000_0000, a ^ 0x8000_0000) } else { (acc, a) };
    let (p, f1) = fpu_mul(fpscr, a, b, false);
    // A NaN product propagates through the add (flags OR, idempotent).
    let (r, f2) = fpu_add(fpscr, acc, p, sub);
    (fpu_dn(r, fpscr), f1 | f2)
}
/// f32 square root. Negative (nonzero) -> NaN + IOC.
fn fpu_sqrt(fpscr: u32, a: u32) -> (u32, u32) {
    let a = fpu_flush(a, fpscr);
    if let Some(n) = fpu_nan_scan(fpscr, &[a]) {
        return n;
    }
    let af = f32::from_bits(a);
    if af < 0.0 {
        return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
    }
    if af == 0.0 {
        return (a, 0); // sqrt(+-0) = +-0, exact
    }
    if fpu_rmode(fpscr) == 0 {
        let rb = af.sqrt().to_bits();
        fpu_final(fpscr, rb, (af as f64).sqrt())
    } else {
        // Directed: exact digit-recurrence sqrt (no f64 rounding anywhere).
        let (kept, sticky, exp, rs) = fpu_sqrt_exact(a);
        fpu_pack(kept, sticky, exp, rs, fpu_rmode(fpscr))
    }
}
/// Normalize a finite nonzero f32 to (sign, unbiased exp, 24-bit mantissa
/// with hidden 1 at bit 23). Subnormals are normalized (FZ flushing is the
/// caller's job); zeros/infs/NaNs must not reach here.
fn fpu_norm(w: u32) -> (u32, i32, u32) {
    let s = w >> 31;
    let e = ((w >> 23) & 0xFF) as i32;
    let f = w & 0x7FFF_FF;
    if e == 0 {
        // Subnormal f x 2^-149, top bit q = 22-sh: exp q-149, mantissa
        // normalized to bit 23 (same pattern as f16_to_f32_bits, tested).
        let sh = f.leading_zeros() - 9; // 23-bit frac in u32
        (s, -127 - sh as i32, ((f << (sh + 1)) & 0x7FFF_FF) | 0x8000_00)
    } else {
        (s, e - 127, f | 0x8000_00)
    }
}
/// Fused acc +/- (a*b): SINGLE rounding (VFPv4 VFMA/VFMS/VFNMA/VFNMS),
/// exact via u128 integer arithmetic — never f64 (which can double-round
/// near f32 rounding boundaries). Negation folded into operand signs up
/// front (exact), so zero-sign rules and directed rounding see final signs.
fn fpu_fma(fpscr: u32, acc: u32, a: u32, b: u32, sub: bool, neg: bool) -> (u32, u32) {
    let (acc, a) = if neg { (acc ^ 0x8000_0000, a ^ 0x8000_0000) } else { (acc, a) };
    let acc = fpu_flush(acc, fpscr);
    let a = fpu_flush(a, fpscr);
    let b = fpu_flush(b, fpscr);
    if let Some(n) = fpu_nan_scan(fpscr, &[acc, a, b]) {
        return n;
    }
    let rmode = fpu_rmode(fpscr);
    // 0 * inf (either order) is invalid even fused.
    if (a == 0 && f32_inf(b)) || (f32_inf(a) && b == 0) {
        return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
    }
    // Exact-zero-product path (no inf involved): acc +/- 0 is exact.
    if a == 0 || b == 0 {
        let ps = (a >> 31) ^ (b >> 31) ^ (sub as u32); // signed term sign
        if acc == 0 {
            // Exact-zero add on effective signs (negation already folded).
            let sa = acc >> 31;
            let zs = match rmode {
                2 => 1,       // toward -inf: -0
                0 => sa & ps, // RNE: -0 only if both negative
                _ => 0,       // toward +inf / zero: +0
            };
            return (zs << 31, 0);
        }
        // acc +/- 0: exact, sign of acc, no flags.
        return (acc, 0);
    }
    // Infinity propagation (no zeros reach here).
    let a_inf = f32_inf(a);
    let b_inf = f32_inf(b);
    let acc_inf = f32_inf(acc);
    if a_inf || b_inf {
        let ps = (a >> 31) ^ (b >> 31) ^ (sub as u32);
        if acc_inf {
            if (acc >> 31) != ps {
                return (fpu_dn(0x7FC0_0000, fpscr), FPSCR_IOC);
            }
            return (acc & 0x8000_0000 | 0x7F80_0000, 0);
        }
        return (ps << 31 | 0x7F80_0000, 0);
    }
    if acc_inf {
        return (acc, 0);
    }
    if acc == 0 {
        // Fused with a zero addend is exactly the correctly-rounded
        // product (single rounding) — and keeps fpu_norm away from zero.
        return fpu_mul(fpscr, a, b, sub);
    }
    // All finite nonzero: exact integer accumulation.
    let (sa, ea, ma) = fpu_norm(acc);
    let (s1, e1, m1) = fpu_norm(a);
    let (s2, e2, m2) = fpu_norm(b);
    let sb = s1 ^ s2 ^ (sub as u32); // effective product sign
    let p_exp = e1 + e2;
    let p_mant = m1 as u128 * m2 as u128; // <= 2^48
    // Align to the larger exponent, sticky-shifting the smaller. Scales:
    // p_mant is 48-bit (value x 2^(p_exp-46)); the addend is lifted to the
    // same 46-scale via <<23 (value ma x 2^(ea-23) = (ma<<23) x 2^(ea-46)).
    let shr_sticky = |v: u128, sh: i32| -> (u128, bool) {
        if sh <= 0 {
            (v, false)
        } else if sh >= 128 {
            (0, v != 0)
        } else {
            (v >> sh, (v & ((1u128 << sh) - 1)) != 0)
        }
    };
    let (pm, cm, base_exp, sticky0) = if p_exp >= ea {
        let (c, st) = shr_sticky((ma as u128) << 23, p_exp - ea);
        (p_mant, c, p_exp, st)
    } else {
        let (p, st) = shr_sticky(p_mant, ea - p_exp);
        (p, (ma as u128) << 23, ea, st)
    };
    // Add/sub magnitudes (A=acc/cm, B=term/pm, same scale now).
    let (mag, rs) = if sa == sb {
        (pm + cm, sa)
    } else if cm >= pm {
        (cm - pm, sa)
    } else {
        (pm - cm, sb)
    };
    if mag == 0 {
        // Exact cancellation: RNE sign rule on effective signs.
        let zs = match rmode {
            2 => 1,
            0 => sa & sb,
            _ => 0,
        };
        return (zs << 31, 0);
    }
    // Normalize to a 25-bit kept (bit 24 = hidden 1) + rest. kept's value
    // is kept x 2^(E'-46) with kept in [2^24, 2^25), i.e. unbiased exp E'-22.
    let l = 128 - mag.leading_zeros() as i32; // bit length, >= 1
    let (kept, rest, exp) = if l > 25 {
        let drop = (l - 25) as u32;
        (mag >> drop, mag & ((1u128 << drop) - 1), base_exp + drop as i32 - 22)
    } else {
        (mag << (25 - l) as u32, 0, base_exp - (25 - l) - 22)
    };
    let sticky_base = sticky0 || rest != 0;
    fpu_pack(kept, sticky_base, exp, rs, rmode)
}
/// Round a normalized 25-bit mantissa (bit 24 set) + below-sticky to f32.
///
/// Shared tail for exact single-rounding paths (fused MLA, directed
/// divide/sqrt): `kept` holds 25 significant bits with value
/// kept x 2^(exp-24) (unbiased exp), `sticky` is OR of everything below
/// kept's bit 0. Handles overflow (inf + OFC|IXC), normals (24-bit round
/// + IXC on inexact) and subnormals (via fpu_fma_subnormal).
fn fpu_pack(kept: u128, sticky: bool, exp: i32, rs: u32, rmode: u32) -> (u32, u32) {
    if exp > 127 {
        return fpu_overflow(rs, rmode);
    }
    if exp >= -126 {
        // Round 25 -> 24 bits (guard = bit 0, sticky below).
        let lsb = ((kept & 2) >> 1) as u32;
        let guard = (kept & 1) as u32;
        let up = fpu_round_up((guard << 1) | (sticky as u32), 2, lsb, rmode, rs != 0);
        let mut k = ((kept >> 1) as u32) + if up { 1 } else { 0 };
        let mut e = exp;
        if k == 0x100_0000 {
            k = 0x8000_00;
            e += 1;
        }
        if e > 127 {
            return fpu_overflow(rs, rmode);
        }
        let fl = if guard != 0 || sticky { FPSCR_IXC } else { 0 };
        return (rs << 31 | ((e + 127) as u32) << 23 | (k & 0x7FFF_FF), fl);
    }
    // Subnormal: round kept25 to multiples of 2^-149 (sh = -125-exp >= 2).
    let sh = -125 - exp;
    if sh >= 128 {
        return (rs << 31, FPSCR_UFC | FPSCR_IXC); // nonzero inputs -> tiny
    }
    fpu_fma_subnormal(kept, sticky, rs, rmode, sh as u32)
}
/// Overflow result per RMode: infinities round outward; directed modes
/// clamp to finite max on the bounded side (toward +inf of negative
/// overflow is -max, etc.). OFC|IXC always accompany (overflow is inexact).
fn fpu_overflow(rs: u32, rmode: u32) -> (u32, u32) {
    let inf = rs << 31 | 0x7F80_0000;
    let max = rs << 31 | 0x7F7F_FFFF;
    let r = match rmode {
        0 => inf,
        1 => {
            if rs == 0 {
                inf
            } else {
                max
            }
        }
        2 => {
            if rs == 0 {
                max
            } else {
                inf
            }
        }
        _ => max,
    };
    (r, FPSCR_OFC | FPSCR_IXC)
}
/// Round a normalized 25-bit fused-mantissa (bit 24 set) + below-sticky to
/// an f32 subnormal/zero: units of 2^-149, sh = -125-exp >= 1.
fn fpu_fma_subnormal(kept: u128, sticky_below: bool, rs: u32, rmode: u32, sh: u32) -> (u32, u32) {
    let q = (kept >> sh) as u32;
    let rem = kept & ((1u128 << sh) - 1);
    let guard = ((rem >> (sh - 1)) & 1) as u32;
    let sticky = sticky_below || (rem & ((1u128 << (sh - 1)) - 1)) != 0;
    let up = fpu_round_up((guard << 1) | (sticky as u32), 2, q & 1, rmode, rs != 0);
    let k = q + if up { 1 } else { 0 };
    if k >= 0x80_0000 {
        // Rounded up into the smallest normal.
        return (rs << 31 | 0x0080_0000, FPSCR_UFC | FPSCR_IXC);
    }
    let fl = if guard != 0 || sticky { FPSCR_UFC | FPSCR_IXC } else { 0 };
    (rs << 31 | k, fl)
}
fn fpu_round_int(v: f64, signed: bool, rmode: u32) -> (u32, bool) {
    if v.is_nan() {
        return (0, true);
    }
    let r = match rmode {
        0 => v.round_ties_even(),
        1 => v.ceil(),
        2 => v.floor(),
        _ => v.trunc(),
    };
    if signed {
        if r < -2147483648.0 || r >= 2147483648.0 {
            return (0, true);
        }
        (r as i64 as u32, false)
    } else {
        if r < 0.0 || r >= 4294967296.0 {
            return (0, true);
        }
        (r as u64 as u32, false)
    }
}
/// Integer square root (floor) for u128 via Newton iteration from a
/// power-of-two overestimate. Used by exact directed sqrt.
fn isqrt_u128(n: u128) -> u128 {
    if n == 0 {
        return 0;
    }
    let mut x = 1u128 << ((128 - n.leading_zeros() + 1) / 2);
    loop {
        let y = (x + n / x) >> 1;
        if y >= x {
            return x;
        }
        x = y;
    }
}
/// Exact directed division core: finite nonzero a/b (already flushed) into
/// the shared pack form — (kept25 with bit 24 set, below-sticky, unbiased
/// exp for kept x 2^(exp-24)). Long division to 27 quotient bits makes the
/// rounding decision exact (no f64 anywhere).
fn fpu_div_exact(a: u32, b: u32) -> (u128, bool, i32, u32) {
    let (sa, ea, ma) = fpu_norm(a);
    let (sb, eb, mb) = fpu_norm(b);
    let rs = sa ^ sb;
    // Q = (ma<<26)/mb lies in (2^25, 2^27); value = Q x 2^(ea-eb-26).
    // (ma/mb in (0.5, 2): ma >= 2^23 > mb/2 and ma < 2*mb always hold.)
    let num = (ma as u128) << 26;
    let q = num / mb as u128;
    let r = num % mb as u128;
    // Normalize to 25 bits (see fpu_pack contract).
    let (kept, rest, exp) = if q >= (1u128 << 26) {
        (q >> 2, q & 3, ea - eb)
    } else {
        (q >> 1, q & 1, ea - eb - 1)
    };
    (kept, rest != 0 || r != 0, exp, rs)
}
/// Exact directed square-root core: finite positive nonzero w (already
/// flushed; caller handles zero/sign/NaN) into the shared pack form.
/// Digit-recurrence via integer sqrt makes rounding exact.
fn fpu_sqrt_exact(w: u32) -> (u128, bool, i32, u32) {
    let (s, e, m) = fpu_norm(w); // value = m x 2^(e-23), m 24-bit
    // Even-ize the exponent so F is integral: value = M x 2^(2F) with
    // M in [2^23, 2^25). (Only even numerators are halved — Rust `/`
    // truncates, so the parity branch matters, not just the value.)
    let (mm, f) = if (e - 23) & 1 != 0 {
        ((m as u128) << 1, (e - 24) / 2)
    } else {
        (m as u128, (e - 23) / 2)
    };
    // S = floor(sqrt(M) x 2^26) lies in [2^37, 2^39).
    let big = mm << 52; // < 2^77, no overflow
    let sq = isqrt_u128(big);
    // kept25 needs bit 24 set: shift 14 if S >= 2^38 else 13.
    let (kept, sh) = if sq >= (1u128 << 38) { (sq >> 14, 14u32) } else { (sq >> 13, 13u32) };
    let rem = sq & ((1u128 << sh) - 1);
    let sticky = rem != 0 || sq * sq != big;
    // result = kept x 2^(F+sh-26) = kept x 2^(exp-24).
    (kept, sticky, f + sh as i32 - 2, s)
}
/// Round an exact f64 to f32 per RMode (int->float and fixed->float need
/// this; plain `as` is RNE-only). Returns (bits, inexact).
fn fpu_round_f32(v: f64, rmode: u32) -> (u32, bool) {
    let r = v as f32; // RNE
    if rmode == 0 || (r as f64) == v {
        return (r.to_bits(), (r as f64) != v);
    }
    let adj = match rmode {
        1 => (r as f64) < v,
        2 => (r as f64) > v,
        _ => {
            if v > 0.0 {
                (r as f64) > v
            } else {
                (r as f64) < v
            }
        }
    };
    if !adj {
        return (r.to_bits(), true);
    }
    let r2 = match rmode {
        1 => r.next_up(),
        2 => r.next_down(),
        _ => {
            if v > 0.0 {
                r.next_down()
            } else {
                r.next_up()
            }
        }
    };
    (r2.to_bits(), true)
}
/// f16 (bits) -> f32 (bits), exact widening (NaN payload preserved).
fn f16_to_f32_bits(h: u16) -> u32 {
    let s = ((h >> 15) & 1) as u32;
    let e = ((h >> 10) & 0x1F) as u32;
    let f = (h & 0x3FF) as u32;
    match e {
        31 => {
            if f == 0 {
                (s << 31) | 0x7F800000
            } else {
                // QNaN passes; SNaN (bit9 clear) is quieted (caller sets IOC).
                (s << 31) | 0x7F800000 | (f << 13) | if f & 0x200 == 0 { 0x0040_0000 } else { 0 }
            }
        }
        0 => {
            if f == 0 {
                s << 31
            } else {
                // Normalize the subnormal: f = 0x200>>sh-style value f x 2^-24
                // with top bit p = 9-sh -> 1.xxx x 2^(p-24), exp 103+p.
                let sh = f.leading_zeros() - 22; // 10-bit frac in u32
                let e32 = 112 - sh as i32;
                let m32 = (f << (sh + 1)) & 0x3FF;
                (s << 31) | ((e32 as u32) << 23) | (m32 << 13)
            }
        }
        _ => (s << 31) | ((e + 112) << 23) | (f << 13),
    }
}
/// Round-to-nearest-or-directed helper over dropped bits.
/// `kept` = value after shifting out `drop` bits, `rest` = dropped bits,
/// `lsb` = kept bit 0. Returns whether to round up (magnitude).
fn fpu_round_up(rest: u32, drop: u32, lsb: u32, rmode: u32, neg: bool) -> bool {
    if rest == 0 {
        return false;
    }
    match rmode {
        0 => {
            let guard = (rest >> (drop - 1)) & 1;
            let sticky = rest & ((1 << (drop - 1)) - 1);
            guard == 1 && (sticky != 0 || lsb == 1)
        }
        1 => !neg,
        2 => neg,
        _ => false,
    }
}
/// f32 (bits) -> f16 (bits) per RMode. Returns (half, flags).
fn f32_to_f16_bits(w: u32, rmode: u32) -> (u16, u32) {
    let s = w >> 31;
    let neg = s != 0;
    let e = ((w >> 23) & 0xFF) as i32;
    let f = w & 0x7FFF_FF;
    if e == 0xFF {
        if f == 0 {
            return ((s << 15 | 0x7C00) as u16, 0); // inf exact, no flag
        }
        let mut h = 0x7E00 | ((f >> 13) as u16 & 0x3FF);
        if f & 0x0040_0000 == 0 {
            h |= 0x0200; // quiet an SNaN (caller sets IOC)
        }
        if h & 0x3FF == 0 {
            h |= 1; // payload must stay nonzero (still NaN)
        }
        return ((s << 15) as u16 | h, 0);
    }
    // Normalized (exp, 24-bit mantissa with hidden 1).
    let (exp, mant): (i32, u32) = if e == 0 {
        if f == 0 {
            return ((s << 15) as u16, 0);
        }
        // f32 subnormal f x 2^-149, top bit q = 22-sh -> exp q-149.
        let sh = f.leading_zeros() - 9; // 23-bit frac in u32
        (-127 - sh as i32, ((f << sh) & 0x7FFF_FF) | 0x8000_00)
    } else {
        (e - 127, f | 0x8000_00)
    };
    let h_exp = exp + 15;
    if h_exp >= 31 {
        return ((s << 15 | 0x7C00) as u16, FPSCR_OFC | FPSCR_IXC);
    }
    if h_exp <= 0 {
        // Subnormal (or zero): total dropped bits = 14 - h_exp.
        let drop = (14 - h_exp) as u32;
        if drop >= 32 {
            return ((s << 15) as u16, FPSCR_UFC | FPSCR_IXC);
        }
        let kept = mant >> drop;
        let rest = mant & ((1 << drop) - 1);
        let up = fpu_round_up(rest, drop, kept & 1, rmode, neg);
        let k = kept + if up { 1 } else { 0 };
        if k >= 0x400 {
            // Rounded up into the smallest normal.
            return ((s << 15 | 0x0400) as u16, FPSCR_UFC | FPSCR_IXC);
        }
        let fl = if rest != 0 { FPSCR_UFC | FPSCR_IXC } else { 0 };
        return ((s << 15 | k) as u16, fl);
    }
    // Normal: drop 13 bits (23 -> 10).
    let kept = mant >> 13;
    let rest = mant & 0x1FFF;
    let up = fpu_round_up(rest, 13, kept & 1, rmode, neg);
    let k = kept + if up { 1 } else { 0 };
    if k >= 0x800 {
        // Mantissa overflow carries into the exponent.
        if h_exp + 1 >= 31 {
            return ((s << 15 | 0x7C00) as u16, FPSCR_OFC | FPSCR_IXC);
        }
        return ((s << 15 | (((h_exp + 1) as u32) << 10)) as u16, if rest != 0 { FPSCR_IXC } else { 0 });
    }
    ((s << 15 | ((h_exp as u32) << 10) | (k & 0x3FF)) as u16, if rest != 0 { FPSCR_IXC } else { 0 })
}
/// Register read with Thumb PC semantics: reads of R15 see `(pc+4)&!3`.
#[inline]
fn rr(c: &Cpu, n: usize, pc: u32) -> u32 {
    if n == 15 {
        (pc + 4) & !3
    } else {
        c.regs.r[n]
    }
}
#[inline]
fn adv(c: &mut Cpu, pc: u32, l: u32) {
    c.regs.r[15] = pc.wrapping_add(l) | 1;
}
fn fault(c: &mut Cpu, pc: u32, op1: u16, op2: u16, l: u8) -> bool {
    c.fault = Some(super::CpuFault { pc, op1, op2, len: l });
    false
}
/// CCR.DIV_0_TRP handling for SDIV/UDIV with a zero divisor: latch
/// DIVBYZERO (UFSR bit 9) and raise; trap clear returns 0 like silicon.
/// Returns true when the divide must not execute (caller returns the
/// fault state). PC is advanced first: the fault is precise at the next
/// instruction, matching the other synchronous raises.
fn div0_trap(cpu: &mut Cpu, sys: &WasmSystem, mem: &mut dyn Memory, pc: u32) -> bool {
    if sys.p.read(sys, 0xE000ED14, 4) & (1 << 4) == 0 {
        return false;
    }
    let cfsr = sys.p.read(sys, 0xE000ED28, 4);
    sys.p.write(sys, 0xE000ED28, 4, cfsr | (1 << 25));
    adv(cpu, pc, 4);
    cpu.raise_sync(sys, mem, Cpu::usage_target(sys));
    true
}
/// Holds the MPU unprivileged-override for exactly one access (LDRT/STRT
/// probe as-unprivileged even in handler mode). Drop clears it, so every
/// exit path — including the rt==15 faults/branches below — is covered.
struct UnprivAccess;
impl UnprivAccess {
    fn arm() -> Self {
        crate::system::set_mpu_force_unpriv(true);
        Self
    }
}
impl Drop for UnprivAccess {
    fn drop(&mut self) {
        crate::system::set_mpu_force_unpriv(false);
    }
}
/// Interworking branch. An EXC_RETURN value performs an exception return
/// through the stacked context instead. The mask ignores bit 4 (FType):
/// FP-extended returns (0xFFFFFFE9/0xFFFFFFED) must route here exactly like
/// F9/FD — masking with 0x0FFFFFF0 strands them (bx lr then "branches" to
/// wilderness with bit0 set). Branching to ARM state (bit0
/// clear, non-EXC_RETURN) is a fault on Cortex-M (no ARM state); halting
/// loudly beats silently running garbage.
fn branch(
    c: &mut Cpu,
    sys: &WasmSystem,
    mem: &mut dyn Memory,
    t: u32,
    pc: u32,
    op1: u16,
    op2: u16,
    l: u8,
) -> bool {
    if (t & 0x0FFFFFE0) == 0x0FFFFFE0 {
        return c.exception_return(sys, mem, t, pc);
    }
    if t & 1 == 0 {
        return fault(c, pc, op1, op2, l);
    }
    c.regs.r[15] = t;
    true
}

// ---- flags ----
#[inline]
fn nz(c: &mut Cpu, v: u32) {
    c.regs.xpsr = (c.regs.xpsr & !0xC0000000)
        | if v == 0 { 0x40000000 } else { 0 }
        | if v & 0x80000000 != 0 { 0x80000000 } else { 0 };
}
fn add_flags(c: &mut Cpu, a: u32, b: u32, ci: u32) -> u32 {
    let r = a.wrapping_add(b).wrapping_add(ci);
    let carry = (a as u64) + (b as u64) + (ci as u64) > 0xFFFF_FFFF;
    let over = ((a ^ r) & (b ^ r) & 0x80000000) != 0;
    c.regs.xpsr = (c.regs.xpsr & !0xF0000000)
        | if r == 0 { 0x40000000 } else { 0 }
        | if r & 0x80000000 != 0 { 0x80000000 } else { 0 }
        | if carry { 0x20000000 } else { 0 }
        | if over { 0x10000000 } else { 0 };
    r
}
fn sub_flags(c: &mut Cpu, a: u32, b: u32, ci: u32) -> u32 {
    // ci here is "carry in" (1 = no borrow). NOT carry = borrow.
    let r = a.wrapping_sub(b).wrapping_sub(1 - ci);
    let borrow = (a as u64) < (b as u64) + (1 - ci) as u64;
    let over = ((a ^ b) & (a ^ r) & 0x80000000) != 0;
    c.regs.xpsr = (c.regs.xpsr & !0xF0000000)
        | if r == 0 { 0x40000000 } else { 0 }
        | if r & 0x80000000 != 0 { 0x80000000 } else { 0 }
        | if !borrow { 0x20000000 } else { 0 }
        | if over { 0x10000000 } else { 0 };
    r
}
#[inline]
fn carry(c: &Cpu) -> u32 {
    (c.regs.xpsr >> 29) & 1
}
fn cond_ok(c: &Cpu, cc: u32) -> bool {
    let x = c.regs.xpsr;
    let n = x & 0x80000000 != 0;
    let z = x & 0x40000000 != 0;
    let cy = x & 0x20000000 != 0;
    let v = x & 0x10000000 != 0;
    match cc {
        0 => z,
        1 => !z,
        2 => cy,
        3 => !cy,
        4 => n,
        5 => !n,
        6 => v,
        7 => !v,
        8 => cy && !z,
        9 => !cy || z,
        10 => n == v,
        11 => n != v,
        12 => !z && n == v,
        13 => z || n != v,
        14 => true,
        _ => false,
    }
}
/// IT-block predication. Returns true when the current instruction executes.
/// Always consumes one IT slot. GAS-verified rule: slot j>=2 uses `cond`
/// iff mask bit (5-j) equals cond bit 0, else the inverse condition.
/// Slot 1 always uses `cond`. `n = 4 - trailing_zeros(mask)`.
fn it_ok(c: &mut Cpu, sys: &WasmSystem) -> bool {
    if c.it_n == 0 {
        return true;
    }
    c.it_idx += 1;
    let cc = if c.it_idx == 1 {
        c.it_cond
    } else {
        let b = (c.it_mask >> (5 - c.it_idx)) & 1;
        if b == (c.it_cond & 1) {
            c.it_cond
        } else {
            c.it_cond ^ 1
        }
    };
    let take = cond_ok(c, cc as u32);
    if c.it_idx >= c.it_n {
        c.it_n = 0;
        c.it_idx = 0;
    }
    if !take {
        // Predicated-skipped slot: zero guest cycles on silicon (DWT FOLD).
        sys.p.dwt_count_fold(sys);
    }
    take
}
/// ARM ARM ThumbExpandImm_C. Returns (value, carry_out).
fn expand_imm(imm12: u32, carry_in: u32) -> (u32, u32) {
    let imm8 = imm12 & 0xFF;
    if imm12 & 0xC00 == 0 {
        let v = match (imm12 >> 8) & 3 {
            0 => imm8,
            1 => (imm8 << 16) | imm8, // 0x00:imm8:0x00:imm8
            2 => (imm8 << 24) | (imm8 << 8), // imm8:0x00:imm8:0x00
            _ => imm8 | (imm8 << 8) | (imm8 << 16) | (imm8 << 24),
        };
        (v, carry_in)
    } else {
        let unrot = 0x80 | (imm12 & 0x7F); // '1' : imm12[6:0]
        let rot = (imm12 >> 7) & 0x1F;
        let v = ror32(unrot, rot);
        let co = if rot == 0 {
            carry_in
        } else {
            (unrot >> ((rot - 1) & 31)) & 1
        };
        (v, co)
    }
}
/// Shifted-register operand. Returns (result, carry_out).
/// Shifted-register operand. Returns (result, carry_out).
/// `reg` distinguishes Rs&0xFF amounts (0 = no shift, carry preserved) from
/// immediate #0 (LSR/ASR #0 = 32). Getting this wrong zeroes every
/// `(x >> (i*8))` with i==0 (DOOM printed patch id 0x120 not 0x123).
fn shift_op(v: u32, typ: u32, amt: u32, ci: u32, reg: bool) -> (u32, u32) {
    if reg && amt == 0 {
        return (v, ci);
    }
    match typ {
        0 => {
            // LSL
            if amt == 0 {
                (v, ci)
            } else if amt < 32 {
                (v.wrapping_shl(amt), (v >> (32 - amt)) & 1)
            } else if amt == 32 {
                (0, v & 1)
            } else {
                (0, 0)
            }
        }
        1 => {
            // LSR
            if amt == 0 || amt == 32 {
                (0, (v >> 31) & 1)
            } else if amt < 32 {
                (v >> amt, (v >> (amt - 1)) & 1)
            } else {
                (0, 0)
            }
        }
        2 => {
            // ASR
            if amt == 0 || amt >= 32 {
                let s = if v & 0x80000000 != 0 { 0xFFFF_FFFF } else { 0 };
                (s, (v >> 31) & 1)
            } else {
                (((v as i32) >> amt) as u32, (v >> (amt - 1)) & 1)
            }
        }
        _ => {
            // ROR / RRX
            if amt == 0 {
                ((ci << 31) | (v >> 1), v & 1)
            } else {
                let s = amt & 31;
                if s == 0 {
                    (v, (v >> 31) & 1)
                } else {
                    (ror32(v, s), (v >> (s - 1)) & 1)
                }
            }
        }
    }
}

pub fn exec16(cpu: &mut Cpu, sys: &WasmSystem, mem: &mut dyn Memory, op: u16, pc: u32) -> bool {
    let o = op as u32;
    // Snapshot predication BEFORE it_ok consumes/resets the slot.
    cpu.it_pred = cpu.it_n > 0;
    // IT predication: a not-taken instruction is still a 2-byte NOP for PC
    // purposes (and still consumes its IT slot).
    if !it_ok(cpu, sys) {
        adv(cpu, pc, 2);
        return true;
    }
    // LSL/LSR/ASR imm, ADD/SUB reg+imm3 (all flag-setting)
    if o & 0xF800 == 0x0000 {
        let (rd, rs) = ((o & 7) as usize, ((o >> 3) & 7) as usize);
        let im = (o >> 6) & 0x1F;
        let v = rr(cpu, rs, pc);
        let (r, co) = shift_op(v, 0, im, carry(cpu), false);
        cpu.regs.r[rd] = r;
        nz(cpu, r);
        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x0800 {
        let (rd, rs) = ((o & 7) as usize, ((o >> 3) & 7) as usize);
        let mut im = (o >> 6) & 0x1F;
        if im == 0 {
            im = 32;
        }
        let v = rr(cpu, rs, pc);
        let (r, co) = shift_op(v, 1, im, carry(cpu), false);
        cpu.regs.r[rd] = r;
        nz(cpu, r);
        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x1000 {
        let (rd, rs) = ((o & 7) as usize, ((o >> 3) & 7) as usize);
        let mut im = (o >> 6) & 0x1F;
        if im == 0 {
            im = 32;
        }
        let v = rr(cpu, rs, pc);
        let (r, co) = shift_op(v, 2, im, carry(cpu), false);
        cpu.regs.r[rd] = r;
        nz(cpu, r);
        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xFE00 == 0x1800 {
        // ADD (register) T1 sets flags (GAS assembles `adds` here) — except
        // predicated (in-IT), where flags are preserved (same rule as MOVS:
        // S_Start's `addle` must not kill N before `suble`'s LE test).
        let (rd, rs, rn) = ((o & 7) as usize, ((o >> 3) & 7) as usize, ((o >> 6) & 7) as usize);
        let a = rr(cpu, rs, pc);
        let b = rr(cpu, rn, pc);
        cpu.regs.r[rd] = a.wrapping_add(b);
        if !cpu.it_pred {
            let _ = add_flags(cpu, a, b, 0);
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xFE00 == 0x1A00 {
        // SUB (register) T1 likewise (`subs`; predicated preserves).
        let (rd, rs, rn) = ((o & 7) as usize, ((o >> 3) & 7) as usize, ((o >> 6) & 7) as usize);
        let a = rr(cpu, rs, pc);
        let b = rr(cpu, rn, pc);
        cpu.regs.r[rd] = a.wrapping_sub(b);
        if !cpu.it_pred {
            let _ = sub_flags(cpu, a, b, 1);
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xFE00 == 0x1C00 {
        let (rd, rn) = ((o & 7) as usize, ((o >> 3) & 7) as usize);
        let im = (o >> 6) & 7;
        let r = add_flags(cpu, rr(cpu, rn, pc), im, 0);
        cpu.regs.r[rd] = r;
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xFE00 == 0x1E00 {
        let (rd, rn) = ((o & 7) as usize, ((o >> 3) & 7) as usize);
        let im = (o >> 6) & 7;
        let r = sub_flags(cpu, rr(cpu, rn, pc), im, 1);
        cpu.regs.r[rd] = r;
        adv(cpu, pc, 2);
        return true;
    }
    // MOVS/CMP/ADDS/SUBS imm8
    if o & 0xF800 == 0x2000 {
        let rd = ((o >> 8) & 7) as usize;
        cpu.regs.r[rd] = o & 0xFF;
        // Predicated (in-IT) T1 MOVS preserves flags (matches Unicorn and
        // GCC's expectation: D_PageTicker's `itt lt; movlt; strlt` needs N
        // live for strlt; clobbering it hangs the title forever). Bare movs
        // still sets N/Z (V cleared, C preserved).
        if !cpu.it_pred {
            nz(cpu, o & 0xFF);
            cpu.regs.xpsr &= !0x10000000;
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x2800 {
        let rn = ((o >> 8) & 7) as usize;
        sub_flags(cpu, rr(cpu, rn, pc), o & 0xFF, 1);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x3000 {
        let rd = ((o >> 8) & 7) as usize;
        let r = add_flags(cpu, rr(cpu, rd, pc), o & 0xFF, 0);
        cpu.regs.r[rd] = r;
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x3800 {
        let rd = ((o >> 8) & 7) as usize;
        let r = sub_flags(cpu, rr(cpu, rd, pc), o & 0xFF, 1);
        cpu.regs.r[rd] = r;
        adv(cpu, pc, 2);
        return true;
    }
    // ALU ops
    if o & 0xFC00 == 0x4000 {
        let sop = (o >> 6) & 0xF;
        let (rs, rd) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let a = rr(cpu, rd, pc);
        let b = rr(cpu, rs, pc);
        match sop {
            0 => {
                let r = a & b;
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
            1 => {
                let r = a ^ b;
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
            2 => {
                let (r, co) = shift_op(a, 0, b & 0xFF, carry(cpu), true);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
            }
            3 => {
                let (r, co) = shift_op(a, 1, b & 0xFF, carry(cpu), true);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
            }
            4 => {
                let (r, co) = shift_op(a, 2, b & 0xFF, carry(cpu), true);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
            }
            5 => {
                let r = add_flags(cpu, a, b, carry(cpu));
                cpu.regs.r[rd] = r;
            }
            6 => {
                let r = sub_flags(cpu, a, b, carry(cpu));
                cpu.regs.r[rd] = r;
            }
            7 => {
                let (r, co) = shift_op(a, 3, b & 0xFF, carry(cpu), true);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
            }
            8 => {
                sub_flags(cpu, a, b, 1);
            }
            9 => {
                // RSB (negate): Rd = 0 - Rs, with flags
                let r = sub_flags(cpu, 0, b, 1);
                cpu.regs.r[rd] = r;
            }
            10 => {
                sub_flags(cpu, a, b, 1);
            }
            11 => {
                add_flags(cpu, a, b, 0);
            }
            12 => {
                let r = a | b;
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
            13 => {
                let r = a.wrapping_mul(b);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
            14 => {
                let r = a & !b;
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
            _ => {
                let r = !b;
                cpu.regs.r[rd] = r;
                nz(cpu, r);
            }
        }
        adv(cpu, pc, 2);
        return true;
    }
    // high-register ops + BX/BLX (op select is bits[9:8])
    if o & 0xFC00 == 0x4400 {
        let h = (o >> 8) & 3;
        let rs = ((o >> 3) & 0xF) as usize;
        let rd = ((o & 7) | ((o >> 4) & 8)) as usize;
        match h {
            0 => {
                let r = rr(cpu, rd, pc).wrapping_add(rr(cpu, rs, pc));
                if rd == 15 {
                    return branch(cpu, sys, mem, r, pc, op, 0, 2);
                }
                cpu.regs.r[rd] = r;
            }
            1 => {
                sub_flags(cpu, rr(cpu, rd, pc), rr(cpu, rs, pc), 1);
            }
            2 => {
                let v = rr(cpu, rs, pc);
                if rd == 15 {
                    return branch(cpu, sys, mem, v, pc, op, 0, 2);
                }
                cpu.regs.r[rd] = v;
            }
            _ => {
                let t = rr(cpu, rs, pc);
                if o & 0x80 != 0 {
                    // BLX reg: LR = next addr; target must stay Thumb
                    cpu.regs.r[14] = (pc + 2) | 1;
                }
                return branch(cpu, sys, mem, t, pc, op, 0, 2);
            }
        }
        adv(cpu, pc, 2);
        return true;
    }
    // LDR literal
    if o & 0xF800 == 0x4800 {
        let rt = ((o >> 8) & 7) as usize;
        let base = (pc + 4) & !3;
        cpu.regs.r[rt] = mem.read32(base.wrapping_add((o & 0xFF) * 4));
        adv(cpu, pc, 2);
        return true;
    }
    // STR/LDR register-offset (class is op[11:9]; Ro=op[8:6], Rn=op[5:3], Rt=op[2:0])
    if o & 0xF000 == 0x5000 {
        let (ro, rn, rt) = (((o >> 6) & 7) as usize, ((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add(rr(cpu, ro, pc));
        match (o >> 9) & 7 {
            0 => mem.write32(addr, rr(cpu, rt, pc)),
            1 => mem.write16(addr, (rr(cpu, rt, pc) & 0xFFFF) as u16),
            2 => mem.write8(addr, (rr(cpu, rt, pc) & 0xFF) as u8),
            3 => {
                let v = mem.read8(addr) as u32;
                cpu.regs.r[rt] = sx(v, 8);
            }
            4 => cpu.regs.r[rt] = mem.read32(addr),
            5 => cpu.regs.r[rt] = mem.read16(addr) as u32,
            6 => cpu.regs.r[rt] = mem.read8(addr) as u32,
            _ => {
                let v = mem.read16(addr) as u32;
                cpu.regs.r[rt] = sx(v, 16);
            }
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x6000 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add(((o >> 6) & 0x1F) * 4);
        mem.write32(addr, rr(cpu, rt, pc));
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x6800 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add(((o >> 6) & 0x1F) * 4);
        cpu.regs.r[rt] = mem.read32(addr);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x7000 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add((o >> 6) & 0x1F);
        mem.write8(addr, (rr(cpu, rt, pc) & 0xFF) as u8);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x7800 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add((o >> 6) & 0x1F);
        cpu.regs.r[rt] = mem.read8(addr) as u32;
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x8000 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add(((o >> 6) & 0x1F) * 2);
        mem.write16(addr, (rr(cpu, rt, pc) & 0xFFFF) as u16);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x8800 {
        let (rn, rt) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let addr = rr(cpu, rn, pc).wrapping_add(((o >> 6) & 0x1F) * 2);
        cpu.regs.r[rt] = mem.read16(addr) as u32;
        adv(cpu, pc, 2);
        return true;
    }
    // SP-relative + ADR
    if o & 0xF800 == 0x9000 {
        let rt = ((o >> 8) & 7) as usize;
        mem.write32(cpu.regs.r[13].wrapping_add((o & 0xFF) * 4), rr(cpu, rt, pc));
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0x9800 {
        let rt = ((o >> 8) & 7) as usize;
        cpu.regs.r[rt] = mem.read32(cpu.regs.r[13].wrapping_add((o & 0xFF) * 4));
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0xA000 {
        let rd = ((o >> 8) & 7) as usize;
        cpu.regs.r[rd] = ((pc + 4) & !3).wrapping_add((o & 0xFF) * 4);
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0xA800 {
        let rd = ((o >> 8) & 7) as usize;
        cpu.regs.r[rd] = cpu.regs.r[13].wrapping_add((o & 0xFF) * 4);
        adv(cpu, pc, 2);
        return true;
    }
    // ADD/SUB SP imm (1011_0000_0/1_xxxxxxx)
    if o & 0xFF00 == 0xB000 {
        let im = (o & 0x7F) * 4;
        if o & 0x80 == 0 {
            cpu.regs.r[13] = cpu.regs.r[13].wrapping_add(im);
        } else {
            cpu.regs.r[13] = cpu.regs.r[13].wrapping_sub(im);
        }
        adv(cpu, pc, 2);
        return true;
    }
    // SXTH/SXTB/UXTH/UXTB (B200-B2FF)
    if o & 0xFF00 == 0xB200 {
        let (rs, rd) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let v = rr(cpu, rs, pc);
        cpu.regs.r[rd] = match (o >> 6) & 3 {
            0 => sx(v & 0xFFFF, 16),
            1 => sx(v & 0xFF, 8),
            2 => v & 0xFFFF,
            _ => v & 0xFF,
        };
        adv(cpu, pc, 2);
        return true;
    }
    // CPS (PRIMASK/FAULTMASK). Bit 4 is the value (id=1/ie=0), bit 0 the
    // target (i=PRIMASK/f=FAULTMASK): cpsid f is B673, cpsie i is B662.
    // SETEND faults (never emitted by firmware).
    if o & 0xFF00 == 0xB600 {
        if o & 0x20 != 0 {
            let v = (o >> 4) & 1;
            if o & 1 != 0 {
                cpu.regs.faultmask = v != 0;
            } else {
                cpu.regs.primask = v;
            }
            adv(cpu, pc, 2);
            return true;
        }
        return fault(cpu, pc, op, 0, 2);
    }
    // PUSH / POP
    if o & 0xFE00 == 0xB400 {
        let list = o & 0xFF;
        let nl = list.count_ones() + if o & 0x100 != 0 { 1 } else { 0 };
        let mut sp = cpu.regs.r[13].wrapping_sub(nl * 4);
        cpu.regs.r[13] = sp;
        for i in 0..8 {
            if (list >> i) & 1 == 1 {
                mem.write32(sp, cpu.regs.r[i as usize]);
                sp += 4;
            }
        }
        if o & 0x100 != 0 {
            mem.write32(sp, cpu.regs.r[14]);
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xFE00 == 0xBC00 {
        let list = o & 0xFF;
        let topc = o & 0x100 != 0;
        let mut sp = cpu.regs.r[13];
        for i in 0..8 {
            if (list >> i) & 1 == 1 {
                cpu.regs.r[i as usize] = mem.read32(sp);
                sp += 4;
            }
        }
        cpu.regs.r[13] = sp.wrapping_add(if topc { 4 } else { 0 });
        if topc {
            let t = mem.read32(sp);
            return branch(cpu, sys, mem, t, pc, op, 0, 2);
        }
        adv(cpu, pc, 2);
        return true;
    }
    // REV / REV16 / REVSH (BA00-BAFF, op is bits[7:6])
    if o & 0xFF00 == 0xBA00 {
        let (rs, rd) = (((o >> 3) & 7) as usize, (o & 7) as usize);
        let v = rr(cpu, rs, pc);
        cpu.regs.r[rd] = match (o >> 6) & 3 {
            0 => v.swap_bytes(),
            1 => {
                ((v & 0xFF) << 8)
                    | ((v >> 8) & 0xFF)
                    | ((v & 0xFF0000) << 8)
                    | ((v & 0xFF000000) >> 8)
            }
            3 => sx(((v & 0xFF) << 8) | ((v >> 8) & 0xFF), 16),
            _ => return fault(cpu, pc, op, 0, 2),
        };
        adv(cpu, pc, 2);
        return true;
    }
    // BKPT: stays a loud fault (debug breakpoints; no firmware here uses them
    // for control flow — FreeRTOS configASSERT loops, it doesn't trap).
    if o & 0xFF00 == 0xBE00 {
        return fault(cpu, pc, op, 0, 2);
    }
    // SVC: synchronous exception. With delivery on, raise it through the
    // priority gate (an SVC that cannot preempt escalates to HardFault,
    // silicon rule); otherwise loud fault (polling firmware never SVCs,
    // so hitting one is a bug worth surfacing).
    if o & 0xFF00 == 0xDF00 {
        if !cpu.deliver_irqs {
            return fault(cpu, pc, op, 0, 2);
        }
        adv(cpu, pc, 2);
        cpu.raise_sync(sys, mem, -5);
        return cpu.fault.is_none();
    }
    if o & 0xFF00 == 0xDE00 {
        return fault(cpu, pc, op, 0, 2);
    }
    // CBZ / CBNZ (base pc+4; offset is imm5*2 + op[9]*64, GAS-verified)
    if o & 0xF500 == 0xB100 {
        let nz = ((o >> 3) & 0x1F) * 2 + ((o >> 9) & 1) * 64;
        let rn = (o & 7) as usize;
        let take = if o & 0x800 == 0 {
            rr(cpu, rn, pc) == 0
        } else {
            rr(cpu, rn, pc) != 0
        };
        if take {
            return branch(cpu, sys, mem, (pc.wrapping_add(4).wrapping_add(nz)) | 1, pc, op, 0, 2);
        }
        adv(cpu, pc, 2);
        return true;
    }
    // hints + IT
    if o & 0xFF00 == 0xBF00 {
        if op == 0xBF00 {
            adv(cpu, pc, 2);
            return true;
        }
        // WFI: with delivery on, halt until an interrupt is pending
        // (JS advances virtual time and wakes us). The instruction is
        // complete at halt, so on wake we resume AFTER it. Without
        // delivery this is a plain nop (polling path).
        if op == 0xBF30 {
            adv(cpu, pc, 2);
            if cpu.deliver_irqs {
                // A pending exception that could preempt right now means no
                // sleep (it is taken on the next run-loop iteration instead).
                if cpu.select_pending_for_sleep(sys) {
                    cpu.sleeping = true;
                }
            }
            return true;
        }
        // WFE: like WFI, but a registered event (SEV, or a recent exception
        // entry/return) means clear-and-continue with no sleep. SEVONPEND
        // (SCR bit 4) additionally wakes on any enabled-pending exception,
        // even one that could never be entered from here.
        if op == 0xBF20 {
            adv(cpu, pc, 2);
            if cpu.deliver_irqs {
                if cpu.event_register {
                    cpu.event_register = false;
                } else {
                    let sevonpend = sys.p.read(sys, 0xE000ED10, 4) & (1 << 4) != 0;
                    let pending_wake =
                        sevonpend && sys.p.nvic.borrow().has_pending();
                    if !pending_wake && cpu.select_pending_for_sleep(sys) {
                        cpu.sleeping = true;
                    }
                }
            }
            return true;
        }
        // SEV: set the event register (a following WFE clears it and skips
        // sleep; single core, so there is no cross-core event to send).
        if op == 0xBF40 {
            adv(cpu, pc, 2);
            cpu.event_register = true;
            return true;
        }
        // YIELD/WFE/WFI/SEV/SEVL(/other reserved hints): no-op for the
        // polling firmware the wasm core runs today.
        if o & 0x0F == 0 && (o & 0xF0) <= 0x50 {
            adv(cpu, pc, 2);
            return true;
        }
        if o & 0x0F == 0 {
            return fault(cpu, pc, op, 0, 2);
        }
        cpu.it_cond = ((o >> 4) & 0xF) as u8;
        cpu.it_mask = (o & 0xF) as u8;
        cpu.it_n = 4 - cpu.it_mask.trailing_zeros() as u8;
        cpu.it_idx = 0;
        adv(cpu, pc, 2);
        return true;
    }
    // STMIA / LDMIA
    if o & 0xF800 == 0xC000 {
        let rn = ((o >> 8) & 7) as usize;
        let list = o & 0xFF;
        let mut a = rr(cpu, rn, pc);
        for i in 0..8 {
            if (list >> i) & 1 == 1 {
                mem.write32(a, cpu.regs.r[i as usize]);
                a += 4;
            }
        }
        // T1 STM always writes back (Rn==PC excluded by construction)
        cpu.regs.r[rn] = a;
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0xC800 {
        let rn = ((o >> 8) & 7) as usize;
        let list = o & 0xFF;
        let mut a = cpu.regs.r[rn];
        for i in 0..8 {
            if (list >> i) & 1 == 1 {
                cpu.regs.r[i as usize] = mem.read32(a);
                a += 4;
            }
        }
        if (list >> rn) & 1 == 0 {
            cpu.regs.r[rn] = a;
        }
        adv(cpu, pc, 2);
        return true;
    }
    // B.cond / B (base is pc+4: Thumb PC reads as addr+4)
    if o & 0xF000 == 0xD000 {
        let cc = (o >> 8) & 0xF;
        let imm = sx(((o & 0xFF) << 1) as u32, 9);
        if cond_ok(cpu, cc) {
            return branch(cpu, sys, mem, (pc.wrapping_add(4).wrapping_add(imm)) | 1, pc, op, 0, 2);
        }
        adv(cpu, pc, 2);
        return true;
    }
    if o & 0xF800 == 0xE000 {
        let imm = sx(((o & 0x7FF) << 1) as u32, 12);
        return branch(cpu, sys, mem, (pc.wrapping_add(4).wrapping_add(imm)) | 1, pc, op, 0, 2);
    }
    fault(cpu, pc, op, 0, 2)
}

// Apply a data-processing ALU op. Returns Some(writeback value) or None for
// test ops (TST/TEQ/CMP/CMN: S=1 and Rd=15). Flags set iff `s`.
fn alu_op(
    cpu: &mut Cpu,
    op: u32,
    s: bool,
    a: u32,
    b: u32,
    ci: u32,
    co: u32,
    rd_is_test: bool,
) -> Option<u32> {
    match op {
        0 => {
            let r = a & b;
            if s {
                nz(cpu, r);
            }
            if rd_is_test {
                None
            } else {
                Some(r)
            }
        }
        1 => {
            let r = a & !b;
            if s {
                nz(cpu, r);
            }
            Some(r)
        }
        2 => {
            let r = a | b;
            if s {
                nz(cpu, r);
            }
            Some(r)
        }
        3 => {
            let r = a | !b;
            if s {
                nz(cpu, r);
            }
            Some(r)
        }
        4 => {
            let r = a ^ b;
            if s {
                nz(cpu, r);
            }
            if rd_is_test {
                None
            } else {
                Some(r)
            }
        }
        8 => {
            let r = if s {
                add_flags(cpu, a, b, 0)
            } else {
                a.wrapping_add(b)
            };
            if rd_is_test {
                None
            } else {
                Some(r)
            }
        }
        10 => {
            // ADC: a + b + carry
            let r = if s {
                add_flags(cpu, a, b, ci)
            } else {
                a.wrapping_add(b).wrapping_add(ci)
            };
            Some(r)
        }
        11 => {
            // SBC: a - b - !carry
            let r = if s {
                sub_flags(cpu, a, b, ci)
            } else {
                a.wrapping_sub(b).wrapping_sub(1 - ci)
            };
            Some(r)
        }
        13 => {
            let r = if s {
                sub_flags(cpu, a, b, 1)
            } else {
                a.wrapping_sub(b)
            };
            if rd_is_test {
                None
            } else {
                Some(r)
            }
        }
        14 => {
            // RSB: b - a... note operand order: RSB Rd,Rn,op2 = op2 - Rn.
            // Callers pass (a=Rn, b=op2) so RSB must compute b-a.
            let r = if s {
                sub_flags(cpu, b, a, 1)
            } else {
                b.wrapping_sub(a)
            };
            Some(r)
        }
        _ => {
            // 5,6,7,9,12,15 are unallocated in the integer subset
            let _ = co;
            None // caller turns this into a fault via a flag; see below
        }
    }
}

pub fn exec32(
    cpu: &mut Cpu,
    sys: &WasmSystem,
    mem: &mut dyn Memory,
    op1: u16,
    op2: u16,
    pc: u32,
) -> bool {
    let o1 = op1 as u32;
    let o2 = op2 as u32;
    cpu.it_pred = cpu.it_n > 0;
    if !it_ok(cpu, sys) {
            adv(cpu, pc, 4);
            return true;
        }
        // USAT / SSAT (saturate). Shares hw1 with MSR (0xF380|Rn) but o2[15]
        // is always 0 here (imm5 lives in o2[14:10]); MSR needs o2>=0x8800,
        // so the two are disjoint. GAS-verified: usat=F380, ssat=F300/F322.
        if o1 & 0xFFF0 == 0xF380 && o2 < 0x8000 {
            // USAT Rd, #sat, Rn [, LSL #sh]. The input is SIGNED (negatives
            // clamp to 0) — the old code zero-extended, so USAT8(-129) came
            // out 255 instead of 0 (fuzz-found).
            let sat = (o2 & 0x1F) as u32;
            // Shift is imm3:imm2 = o2[14:12]:o2[7:6] (GAS: lsl#3=0x08C7,
            // lsl#7=0x18C7, lsl#15=0x38C7) — NOT contiguous o2[14:10].
            // The old contiguous read added Rd[3]/o2[10] into the amount,
            // so any Rd>=8 shifted by garbage (SSAT16(127) came out 508).
            let sh = (((o2 >> 12) & 0x7) << 2) | ((o2 >> 6) & 0x3);
            let v = (rr(cpu, (o1 & 0xF) as usize, pc) as i32).wrapping_shl(sh);
            let max: i64 = if sat >= 32 { 0xFFFF_FFFF } else { (1i64 << sat) - 1 };
            let s = v as i64;
            let r = s.clamp(0, max) as i32 as u32;
            if s < 0 || s > max {
                cpu.regs.xpsr |= 0x08000000; // Q sticky
            }
            cpu.regs.r[((o2 >> 8) & 0xF) as usize] = r;
            adv(cpu, pc, 4);
            return true;
        }
        if o1 & 0xFFC0 == 0xF300 && o2 < 0x8000 {
            // SSAT Rd, #sat, Rn [, LSL/ASL #sh] (sh-type is o1[5]).
            // o2[15]==0 keeps B.W/Bcc.W/BL (op2[15]=1) falling through.
            // Unlike USAT, the sat field encodes N-1 (GAS: ssat#8=o2:0x07,
            // ssat#16=0x0F; usat#16=0x10 direct). Shift is imm3:imm2 like
            // USAT (see above) — the old contiguous o2[14:10] read shifted
            // by garbage whenever Rd>=8.
            let sat = ((o2 & 0x1F) + 1) as u32;
            let sh = (((o2 >> 12) & 0x7) << 2) | ((o2 >> 6) & 0x3);
            let a = rr(cpu, (o1 & 0xF) as usize, pc);
            let v = if (o1 >> 5) & 1 == 0 {
                a.wrapping_shl(sh)
            } else {
                ((a as i32).wrapping_shr(sh.min(31))) as u32
            };
            let (lo, hi): (i64, i64) = if sat == 0 || sat >= 32 {
                (i64::MIN, i64::MAX)
            } else {
                (-(1i64 << (sat - 1)), (1i64 << (sat - 1)) - 1)
            };
            let s = v as i32 as i64;
            let r = s.clamp(lo, hi) as i32 as u32;
            if s < lo || s > hi {
                cpu.regs.xpsr |= 0x08000000; // Q sticky
            }
            cpu.regs.r[((o2 >> 8) & 0xF) as usize] = r;
            adv(cpu, pc, 4);
            return true;
        }
    // ---- F3: misc (hints, barriers, MRS/MSR, bitfield) ----
    // NOTE: F3xx overlaps Bcc.W's op1 range (F000-F3FF), so this must only
    // claim exact/shape-checked F3 forms and let Bcc.W-shaped op2 fall
    // through to the branch decoder below. MRS/MSR are safe to prioritize:
    // Bcc.W never validly uses their op1 (cond would be NV/AL, which
    // assemblers don't emit — B.W covers AL).
    if o1 & 0xFF00 == 0xF300 {
        if o1 == 0xF3AF && o2 == 0x8000 {
            adv(cpu, pc, 4); // NOP.W
            return true;
        }
        if o1 == 0xF3BF && (o2 & 0xFF00) == 0x8F00 {
            adv(cpu, pc, 4); // DMB/DSB/ISB/CLREX
            return true;
        }
        if o1 & 0xFFF0 == 0xF3E0 && o2 & 0xF000 == 0x8000 {
            // MRS Rd, SYSm
            let sysm = (o2 & 0xFF) as u32;
            let rd = ((o2 >> 8) & 0xF) as usize;
                cpu.regs.r[rd] = match sysm {
                    // MRS APSR/IAPSR/EAPSR returns NZCVQ **and GE[19:16]**
                    // (masking to NZCVQ hid GE and made UADD8/USUB8 look
                    // broken when only the READ was — fuzz-found).
                    0 | 1 | 2 => cpu.regs.xpsr & 0xF80F0000,
                3 => cpu.regs.xpsr,                     // XPSR
                5 => cpu.ipsr,                          // IPSR (live exception number)
                6 | 7 => cpu.regs.xpsr & 0x0700FC00,    // EPSR/IEPSR
                8 => cpu.read_msp(),
                9 => cpu.read_psp(),
                16 => cpu.regs.primask,                 // PRIMASK
                17 => cpu.regs.faultmask as u32,        // FAULTMASK (real state now)
                18 | 19 => cpu.regs.basepri as u32,     // BASEPRI(+_MAX reads BASEPRI)
                20 => cpu.regs.control,                 // CONTROL
                _ => return fault(cpu, pc, op1, op2, 4),
            };
            adv(cpu, pc, 4);
            return true;
        }
        if o1 & 0xFFF0 == 0xF380 && o2 & 0xFF00 == 0x8800 {
            // MSR SYSm, Rn
            let sysm = (o2 & 0xFF) as u32;
            let v = rr(cpu, (o1 & 0xF) as usize, pc);
            match sysm {
                0 | 1 | 2 | 3 => {
                    cpu.regs.xpsr = (cpu.regs.xpsr & !0xF8000000) | (v & 0xF8000000)
                }
                8 => cpu.write_msp(v),
                9 => cpu.write_psp(v),
                16 => cpu.regs.primask = v & 1,
                17 => cpu.regs.faultmask = v & 1 != 0, // FAULTMASK
                18 => cpu.regs.basepri = (v & 0xFF) as u8, // BASEPRI
                // BASEPRI_MAX raises the mask only (never lowers it).
                19 => {
                    let b = (v & 0xFF) as u8;
                    if b > cpu.regs.basepri {
                        cpu.regs.basepri = b;
                    }
                }
                20 => {
                    // MSR CONTROL: an SPSEL change switches the current stack
                    // (hardware swaps r13 with the other bank). From
                    // unprivileged Thread mode the nPRIV/SPSEL bits are
                    // ignored (no escalation by MSR); privileged writes go
                    // through (the MPU-test unpriv/priv dance proves both).
                    let mut v = v & 3;
                    if cpu.ipsr == 0 && cpu.regs.control & 1 != 0 {
                        v = cpu.regs.control & 3;
                    }
                    if (v ^ cpu.regs.control) & 2 != 0 {
                        if v & 2 != 0 {
                            cpu.regs.msp = cpu.regs.r[13];
                            cpu.regs.r[13] = cpu.regs.psp;
                        } else {
                            cpu.regs.psp = cpu.regs.r[13];
                            cpu.regs.r[13] = cpu.regs.msp;
                        }
                    }
                    cpu.regs.control = v;
                    // Privilege may have changed (nPRIV bit): refresh the
                    // MPU cache (handlers stay privileged regardless).
                    crate::system::set_cpu_context(
                        cpu.ipsr != 0 || (v & 1) == 0,
                        cpu.ipsr == 2 || cpu.ipsr == 3,
                    );
                }
                _ => return fault(cpu, pc, op1, op2, 4),
            }
            adv(cpu, pc, 4);
            return true;
        }
        if o2 < 0x8000 {
            // Bitfield with op2[15]=0 (Bcc.W/BL always have op2[15]=1).
            if o1 & 0xFFF0 == 0xF3C0 || o1 & 0xFFF0 == 0xF340 {
                // UBFX / SBFX
                let rn = (o1 & 0xF) as usize;
                let rd = ((o2 >> 8) & 0xF) as usize;
                let lsb = (((o2 >> 12) & 7) << 2) | ((o2 >> 6) & 3);
                let w = (o2 & 0x1F) + 1;
                let v = rr(cpu, rn, pc).wrapping_shr(lsb);
                let v = if w >= 32 { v } else { v & ((1u32 << w) - 1) };
                cpu.regs.r[rd] = if o1 & 0xFFF0 == 0xF340 { sx(v, w) } else { v };
                adv(cpu, pc, 4);
                return true;
            }
            if o1 & 0xFFF0 == 0xF360 {
                // BFI / BFC (Rn==15)
                let rn = (o1 & 0xF) as usize;
                let rd = ((o2 >> 8) & 0xF) as usize;
                let lsb = (((o2 >> 12) & 7) << 2) | ((o2 >> 6) & 3);
                let msb = o2 & 0x1F;
                if msb < lsb || msb >= 32 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let w = msb - lsb + 1;
                let mask = if w >= 32 {
                    0xFFFF_FFFF
                } else {
                    ((1u32 << w) - 1) << lsb
                };
                if rn == 15 {
                    cpu.regs.r[rd] &= !mask;
                } else {
                    cpu.regs.r[rd] =
                        (cpu.regs.r[rd] & !mask) | ((rr(cpu, rn, pc) << lsb) & mask);
                }
                adv(cpu, pc, 4);
                return true;
            }
            return fault(cpu, pc, op1, op2, 4);
        }
        // else: op2 >= 0x8000 with F3xx op1 and no exact match above.
        // Falls through to the F-bucket branch decoder (Bcc.W with F3xx
        // op1) or faults there. Do NOT return here.
    }
    // ---- F000-F7FF prefix: branches, MOVW/MOVT, modified-immediate ----
    if o1 & 0xF800 == 0xF000 {
        // branches (op2[15:14] == 10)
        if o2 & 0xC000 == 0x8000 {
            let s = (o1 >> 10) & 1;
            let imm10 = o1 & 0x3FF;
            let imm11 = o2 & 0x7FF;
            let j1 = (o2 >> 13) & 1;
            let j2 = (o2 >> 11) & 1;
            let i1 = j1 ^ s ^ 1;
            let i2 = j2 ^ s ^ 1;
            let off = sx((s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1), 25);
            if o2 & 0x1000 != 0 {
                // B.W unconditional
                return branch(cpu, sys, mem, pc.wrapping_add(4).wrapping_add(off) | 1, pc, op1, op2, 4);
            }
            // Bcc.W: cond in op1[9:6]. 21-bit offset S:J1:J2:imm6:imm11:0
            // with J used DIRECTLY as the offset bits (no S inversion —
            // GAS-verified incl. an S=1 backward bne.w: I1=I2=J1=J2=1).
            let cc = (o1 >> 6) & 0xF;
            if cc == 0xF {
                return fault(cpu, pc, op1, op2, 4);
            }
            let imm6 = o1 & 0x3F;
            let off = sx(
                (s << 20) | (j1 << 19) | (j2 << 18) | (imm6 << 12) | (imm11 << 1),
                21,
            );
            if cc == 0xE || cond_ok(cpu, cc) {
                return branch(cpu, sys, mem, pc.wrapping_add(4).wrapping_add(off) | 1, pc, op1, op2, 4);
            }
            adv(cpu, pc, 4);
            return true;
        }
        // BL (Fxxx) / BLX-imm (Exxx, ARM state: impossible on Cortex-M)
        if o2 & 0xC000 == 0xC000 {
            if o2 & 0xF000 == 0xF000 {
                let s = (o1 >> 10) & 1;
                let imm10 = o1 & 0x3FF;
                let imm11 = o2 & 0x7FF;
                let j1 = (o2 >> 13) & 1;
                let j2 = (o2 >> 11) & 1;
                let i1 = j1 ^ s ^ 1;
                let i2 = j2 ^ s ^ 1;
                let off = sx((s << 24) | (i1 << 23) | (i2 << 22) | (imm10 << 12) | (imm11 << 1), 25);
                cpu.regs.r[14] = (pc + 4) | 1;
                return branch(cpu, sys, mem, pc.wrapping_add(4).wrapping_add(off) | 1, pc, op1, op2, 4);
            }
            // BLX-imm (Exxx) targets ARM state, Cxxx/Dxxx unallocated here.
            return fault(cpu, pc, op1, op2, 4);
        }
        // MOVW / MOVT (F2 group, exact masks so F6/F7 never match)
        if o1 & 0xFBF0 == 0xF240 {
            let rd = ((o2 >> 8) & 0xF) as usize;
            let i = (o1 >> 10) & 1;
            let imm = (i << 11) | ((o1 & 0xF) << 12) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF);
            cpu.regs.r[rd] = imm;
            adv(cpu, pc, 4);
            return true;
        }
        if o1 & 0xFBF0 == 0xF2C0 {
            let rd = ((o2 >> 8) & 0xF) as usize;
            let i = (o1 >> 10) & 1;
            let imm = (i << 11) | ((o1 & 0xF) << 12) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF);
            cpu.regs.r[rd] = (cpu.regs.r[rd] & 0xFFFF) | (imm << 16);
            adv(cpu, pc, 4);
            return true;
        }
        // ADDW / SUBW (F2 group): plain 12-bit immediate, NO ThumbExpand,
        // no flags. GAS-verified: addw=F20x, subw=F2Ax (i-bit in o1[10]
        // flips F2->F6, e.g. subw#4095=F6A1). Must precede modified-imm:
        // ADDW's o1[8:5]=0 decodes as AND-imm (silently wrong), SUBW's =5
        // faults. (ADDSW/SUBSW F3 S-variants still fault loudly.)
        if o1 & 0xFBF0 == 0xF200 || o1 & 0xFBF0 == 0xF2A0 {
            let sub = (o1 & 0xFBF0) == 0xF2A0;
            let rn = (o1 & 0xF) as usize;
            let rd = ((o2 >> 8) & 0xF) as usize;
            let imm = (((o1 >> 10) & 1) << 11) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF);
            cpu.regs.r[rd] = if sub {
                rr(cpu, rn, pc).wrapping_sub(imm)
            } else {
                rr(cpu, rn, pc).wrapping_add(imm)
            };
            adv(cpu, pc, 4);
            return true;
        }
        // F6/F7 + data op2: SSAT/USAT/coprocessor zone (no samples) -> fault
        if o1 >= 0xF600 {
            return fault(cpu, pc, op1, op2, 4);
        }
        // modified-immediate data processing (F000-F5FF, op2 < 0x8000)
        if o2 & 0x8000 == 0 {
            let b8 = (o1 >> 8) & 1;
            let b7 = (o1 >> 7) & 1;
            let b6 = (o1 >> 6) & 1;
            let b5 = (o1 >> 5) & 1;
            let s = (o1 & 0x10) != 0;
            // GAS-verified op table (uniform across F and EA/EB groups)
            let op = match (b8 << 3) | (b7 << 2) | (b6 << 1) | b5 {
                0b0000 => 0,  // AND
                0b0001 => 1,  // BIC
                0b0010 => 2,  // ORR
                0b0011 => 3,  // MVN/ORN
                0b0100 => 4,  // EOR
                0b1000 => 8,  // ADD
                0b1010 => 10, // ADC
                0b1011 => 11, // SBC
                0b1101 => 13, // SUB
                0b1110 => 14, // RSB
                _ => return fault(cpu, pc, op1, op2, 4),
            };
            let rn = (o1 & 0xF) as usize;
            let rd = ((o2 >> 8) & 0xF) as usize;
            let imm12 = (((o1 >> 10) & 1) << 11) | (((o2 >> 12) & 7) << 8) | (o2 & 0xFF);
            let ci = carry(cpu);
            let (imm, co) = expand_imm(imm12, ci);
            let a = rr(cpu, rn, pc);
            // MOV/MVN: Rn==15 selects move form
            if op == 2 && rn == 15 {
                if rd == 15 {
                    if s {
                        // MOVS pc: exception return (not supported yet)
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    return branch(cpu, sys, mem, imm, pc, op1, op2, 4);
                }
                cpu.regs.r[rd] = imm;
                if s {
                    nz(cpu, imm);
                }
                adv(cpu, pc, 4);
                return true;
            }
            if op == 3 && rn == 15 {
                if rd == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                cpu.regs.r[rd] = !imm;
                if s {
                    nz(cpu, !imm);
                }
                adv(cpu, pc, 4);
                return true;
            }
            let test = s && rd == 15;
            match alu_op(cpu, op, s, a, imm, ci, co, test) {
                Some(r) => {
                    if rd == 15 {
                        return branch(cpu, sys, mem, r, pc, op1, op2, 4);
                    }
                    cpu.regs.r[rd] = r;
                    adv(cpu, pc, 4);
                    true
                }
                None => {
                    if test {
                        adv(cpu, pc, 4);
                        true
                    } else {
                        fault(cpu, pc, op1, op2, 4)
                    }
                }
            }
        } else {
            fault(cpu, pc, op1, op2, 4)
        }
        // ---- F8/F9: single data transfer (word/byte/half/signed) ----
    } else if o1 & 0xF800 == 0xF800 && o1 < 0xFA00 {
        let c = (o1 >> 4) & 0xF; // class nibble
        let rn = (o1 & 0xF) as usize;
        let rt = ((o2 >> 12) & 0xF) as usize;
        // size/load operation by class:
        // F8 T3 (c 0..5): 0 STRB,1 LDRB,2 STRH,3 LDRH,4 STR,5 LDR (imm8-PUW or reg)
        // F8 T2 (c 8..13): 8 STRB,9 LDRB,10 STRH,11 LDRH,12 STR,13 LDR (imm12)
        // F9 T1 (c 9,11): 9 LDRSB,11 LDRSH (imm12); F9 T2 (c 1,3): PUW forms
        let f9 = o1 >= 0xF900;
        let (is_load, size, signed) = if !f9 {
            match c {
                0 => (false, 1, false),
                1 => (true, 1, false),
                2 => (false, 2, false),
                3 => (true, 2, false),
                4 => (false, 4, false),
                5 => (true, 4, false),
                8 => (false, 1, false),
                9 => (true, 1, false),
                10 => (false, 2, false),
                11 => (true, 2, false),
                12 => (false, 4, false),
                13 => (true, 4, false),
                _ => return fault(cpu, pc, op1, op2, 4),
            }
        } else {
            match c {
                1 => (true, 1, true),
                3 => (true, 2, true),
                9 => (true, 1, true),
                11 => (true, 2, true),
                _ => return fault(cpu, pc, op1, op2, 4),
            }
        };
        // addressing: T2-imm12 (c>=8) vs T3-PUW/register (c<8)
        if c >= 8 {
            let imm12 = o2 & 0xFFF;
            if rn == 15 {
                // literal pool
                if !is_load {
                    return fault(cpu, pc, op1, op2, 4);
                }
                if rt == 15 {
                    adv(cpu, pc, 4); // PLD/PLI
                    return true;
                }
                let base = (pc + 4) & !3;
                let v = mem.read32(base.wrapping_add(imm12));
                cpu.regs.r[rt] = match (size, signed) {
                    (1, false) => v & 0xFF,
                    (2, false) => v & 0xFFFF,
                    (1, true) => sx(v & 0xFF, 8),
                    (2, true) => sx(v & 0xFFFF, 16),
                    _ => v,
                };
                adv(cpu, pc, 4);
                return true;
            }
            if rt == 15 {
                if is_load {
                    adv(cpu, pc, 4); // PLD/PLI
                    return true;
                }
                return fault(cpu, pc, op1, op2, 4);
            }
            let addr = rr(cpu, rn, pc).wrapping_add(imm12);
            if is_load {
                let v = match size {
                    1 => mem.read8(addr) as u32,
                    2 => mem.read16(addr) as u32,
                    _ => mem.read32(addr),
                };
                cpu.regs.r[rt] = if signed {
                    sx(v, size * 8)
                } else {
                    v
                };
                if rt == 15 {
                    // LDR pc literal already handled; imm12 LDR pc: interwork
                    let t = cpu.regs.r[15];
                    return branch(cpu, sys, mem, t, pc, op1, op2, 4);
                }
            } else {
                let v = rr(cpu, rt, pc);
                match size {
                    1 => mem.write8(addr, (v & 0xFF) as u8),
                    2 => mem.write16(addr, (v & 0xFFFF) as u16),
                    _ => mem.write32(addr, v),
                }
            }
            adv(cpu, pc, 4);
            return true;
        }
        // T3: c<8. Register-offset iff op2[11:10]==00 (GAS-verified:
        // strh [r9,r3,lsl#1]=o2:0x2013, str [r4,r7,lsl#2]=0x0027,
        // strb [r9,r3]=0x2003, ldr [r3,r0,lsl#2]=0x4020; imm forms like
        // str [r4],#4 (0x0B04) have [11:10]!=00). Applies to every data
        // class (STRB/LDRB/STRH/LDRH/STR/LDR), not just words — routing
        // only c4/5 here sent strh-reg into imm8 post-indexed writeback
        // (r9 -= imm8 per store), which corrupted DOOM's collump pointer.
        if (o2 & 0xC00) == 0 {
            // Register-offset (also F9 LDRSB/LDRSH-reg, e.g. DOOM's vertex
            // loads; same o2[11:10]==00 discriminator, GAS-verified).
            // Signed word is unallocated (F8 word loads are unsigned).
            if signed && size == 4 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let rm = (o2 & 0xF) as usize;
            let sh = (o2 >> 4) & 3;
            let off = rr(cpu, rm, pc).wrapping_shl(sh);
            let addr = rr(cpu, rn, pc).wrapping_add(off);
            if is_load {
                if rt == 15 {
                    if size != 4 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    cpu.regs.r[15] = mem.read32(addr);
                    let t = cpu.regs.r[15];
                    return branch(cpu, sys, mem, t, pc, op1, op2, 4);
                }
                cpu.regs.r[rt] = match (size, signed) {
                    (1, false) => mem.read8(addr) as u32,
                    (2, false) => mem.read16(addr) as u32,
                    (1, true) => sx(mem.read8(addr) as u32, 8),
                    (2, true) => sx(mem.read16(addr) as u32, 16),
                    _ => mem.read32(addr),
                };
            } else {
                if rt == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let v = rr(cpu, rt, pc);
                match size {
                    1 => mem.write8(addr, (v & 0xFF) as u8),
                    2 => mem.write16(addr, (v & 0xFFFF) as u16),
                    _ => mem.write32(addr, v),
                }
            }
            adv(cpu, pc, 4);
            return true;
        }
        // imm8 P/U/W form. op2[11:8]==0xE marks the unprivileged T-variants
        // (LDRBT/LDRHT/STRBT/STRHT/LDRT/STRT/LDRSBT/LDRSHT — GAS tform.s:
        // privileged PUW uses 0xB/0xC/0xF, never 0xE), which probe memory
        // as-unprivileged even in handler mode. Address/direction decode
        // identically; only the MPU privilege differs.
        let p = (o2 >> 10) & 1;
        let u = (o2 >> 9) & 1;
        let w = (o2 >> 8) & 1;
        let imm8 = o2 & 0xFF;
        let off = if u == 1 { imm8 } else { imm8.wrapping_neg() };
        let base = rr(cpu, rn, pc);
        let addr = if p == 1 { base.wrapping_add(off) } else { base };
        let _unpriv = if (o2 & 0xF00) == 0xE00 { Some(UnprivAccess::arm()) } else { None };
        if is_load {
            let v = match size {
                1 => mem.read8(addr) as u32,
                2 => mem.read16(addr) as u32,
                _ => mem.read32(addr),
            };
            let v = if signed { sx(v, size * 8) } else { v };
            if rt == 15 {
                if size != 4 || signed {
                    return fault(cpu, pc, op1, op2, 4);
                }
                cpu.regs.r[15] = v;
                if w == 1 || p == 0 {
                    cpu.regs.r[rn] = base.wrapping_add(off);
                }
                return branch(cpu, sys, mem, v, pc, op1, op2, 4);
            }
            cpu.regs.r[rt] = v;
        } else {
            if rt == 15 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let v = rr(cpu, rt, pc);
            match size {
                1 => mem.write8(addr, (v & 0xFF) as u8),
                2 => mem.write16(addr, (v & 0xFFFF) as u16),
                _ => mem.write32(addr, v),
            }
        }
        if w == 1 || p == 0 {
            cpu.regs.r[rn] = base.wrapping_add(off);
        }
        adv(cpu, pc, 4);
        return true;
        // ---- FA: shifted-reg, extend, CLZ/RBIT/REV ----
    } else if o1 & 0xFF00 == 0xFA00 {
        let op = (o1 >> 4) & 0xF;
        let rn = (o1 & 0xF) as usize;
        let rd = ((o2 >> 8) & 0xF) as usize;
        let rm = (o2 & 0xF) as usize;
        match op {
            0 => {
                // LSL-reg (op2[7:4]==0) or SXT AH/SXTH (op2[7:4]==8)
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                match (o2 >> 4) & 0xF {
                    0 => {
                        // Rd = Rn << (Rm & 0xFF): value is Rn (op1), amount Rm (op2)
                        let amt = rr(cpu, rm, pc) & 0xFF;
                        let (r, _) = shift_op(rr(cpu, rn, pc), 0, amt, carry(cpu), true);
                        cpu.regs.r[rd] = r;
                        adv(cpu, pc, 4);
                        return true;
                    }
                    8 => {
                        if o2 & 0xC0 != 0x80 {
                            return fault(cpu, pc, op1, op2, 4);
                        }
                        let rot = ((o2 >> 4) & 3) * 8;
                        let v = sx(ror32(rr(cpu, rm, pc), rot) & 0xFFFF, 16);
                        cpu.regs.r[rd] = if rn == 15 {
                            v
                        } else {
                            rr(cpu, rn, pc).wrapping_add(v)
                        };
                        adv(cpu, pc, 4);
                        return true;
                    }
                    _ => return fault(cpu, pc, op1, op2, 4),
                }
            }
            1 => {
                // LSLS-reg (sub 0) / UXT AH/UXTH (sub 8, o2 = F:Rd:10:rot:Rm).
                // GAS: `lsls.w r0,r1,r2`=fa11 f002.
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                match (o2 >> 4) & 0xF {
                    0 => {
                        let amt = rr(cpu, rm, pc) & 0xFF;
                        let (r, co) =
                            shift_op(rr(cpu, rn, pc), 0, amt, carry(cpu), true);
                        cpu.regs.r[rd] = r;
                        nz(cpu, r);
                        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
                        adv(cpu, pc, 4);
                        return true;
                    }
                    8 => {
                        // UXT AH / UXTH (op2 = F:Rd:10:rot:Rm)
                        if o2 & 0xF0C0 != 0xF080 {
                            return fault(cpu, pc, op1, op2, 4);
                        }
                        let rot = ((o2 >> 4) & 3) * 8;
                        let v = ror32(rr(cpu, rm, pc), rot) & 0xFFFF;
                        cpu.regs.r[rd] = if rn == 15 {
                            v
                        } else {
                            rr(cpu, rn, pc).wrapping_add(v)
                        };
                        adv(cpu, pc, 4);
                        return true;
                    }
                    _ => return fault(cpu, pc, op1, op2, 4),
                }
            }
            2 | 6 => {
                // LSR / ROR (register): Rd = Rn <op> (Rm & 0xFF).
                // SXTAB16 shares op 2 (o2 = F:Rd:10:rot:Rm, sub 8/9;
                // GAS: `sxtab16 r0,r1,r2`=fa21 f082, `,ror #8`=fa21 f092).
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let sub = (o2 >> 4) & 0xF;
                if op == 2 && (sub == 8 || sub == 9) {
                    if o2 & 0xF0C0 != 0xF080 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    let rot = ((o2 >> 4) & 3) * 8;
                    let m = ror32(rr(cpu, rm, pc), rot);
                    let an = rr(cpu, rn, pc);
                    let lo =
                        (an & 0xFFFF).wrapping_add(sx(m & 0xFFFF, 16)) & 0xFFFF;
                    let hi = ((an >> 16)
                        .wrapping_add(sx((m >> 16) & 0xFFFF, 16)))
                        & 0xFFFF;
                    cpu.regs.r[rd] = (hi << 16) | lo;
                    adv(cpu, pc, 4);
                    return true;
                }
                if sub != 0 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let typ = op >> 1; // 1, 3
                let amt = rr(cpu, rm, pc) & 0xFF;
                let (r, _) = shift_op(rr(cpu, rn, pc), typ, amt, carry(cpu), true);
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                return true;
            }
            3 => {
                // LSRS-reg (sub 0) / UXTAB16 (sub 8/9, o2 = F:Rd:10:rot:Rm).
                // GAS: `lsrs.w r0,r1,r2`=fa31 f002,
                // `uxtab16 r0,r1,r2`=fa31 f082.
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                match (o2 >> 4) & 0xF {
                    0 => {
                        let amt = rr(cpu, rm, pc) & 0xFF;
                        let (r, co) =
                            shift_op(rr(cpu, rn, pc), 1, amt, carry(cpu), true);
                        cpu.regs.r[rd] = r;
                        nz(cpu, r);
                        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
                        adv(cpu, pc, 4);
                        return true;
                    }
                    8 | 9 => {
                        if o2 & 0xF0C0 != 0xF080 {
                            return fault(cpu, pc, op1, op2, 4);
                        }
                        let rot = ((o2 >> 4) & 3) * 8;
                        let m = ror32(rr(cpu, rm, pc), rot);
                        let an = rr(cpu, rn, pc);
                        let lo =
                            (an & 0xFFFF).wrapping_add(m & 0xFFFF) & 0xFFFF;
                        let hi = ((an >> 16).wrapping_add((m >> 16) & 0xFFFF))
                            & 0xFFFF;
                        cpu.regs.r[rd] = (hi << 16) | lo;
                        adv(cpu, pc, 4);
                        return true;
                    }
                    _ => return fault(cpu, pc, op1, op2, 4),
                }
            }
            7 => {
                // RORS-reg (sub 0). GAS: `rors.w r0,r1,r2`=fa71 f002.
                if (o2 & 0xF0F0) != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let amt = rr(cpu, rm, pc) & 0xFF;
                let (r, co) = shift_op(rr(cpu, rn, pc), 3, amt, carry(cpu), true);
                cpu.regs.r[rd] = r;
                nz(cpu, r);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
                adv(cpu, pc, 4);
                return true;
            }
            4 => {
                // ASR-reg (op2[7:4]==0) or SXTAB/SXTB (op2[7:4]==8)
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                match (o2 >> 4) & 0xF {
                    0 => {
                        let amt = rr(cpu, rm, pc) & 0xFF;
                        let (r, _) = shift_op(rr(cpu, rn, pc), 2, amt, carry(cpu), true);
                        cpu.regs.r[rd] = r;
                        adv(cpu, pc, 4);
                        return true;
                    }
                    8 => {
                        if o2 & 0xC0 != 0x80 {
                            return fault(cpu, pc, op1, op2, 4);
                        }
                        let rot = ((o2 >> 4) & 3) * 8;
                        let v = sx(ror32(rr(cpu, rm, pc), rot) & 0xFF, 8);
                        cpu.regs.r[rd] = if rn == 15 {
                            v
                        } else {
                            rr(cpu, rn, pc).wrapping_add(v)
                        };
                        adv(cpu, pc, 4);
                        return true;
                    }
                    _ => return fault(cpu, pc, op1, op2, 4),
                }
            }
            5 => {
                // ASRS-reg (sub 0) / UXTAB/UXTB (sub 8, o2 = F:Rd:10:rot:Rm).
                // GAS: `asrs.w r0,r1,r2`=fa51 f002.
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                match (o2 >> 4) & 0xF {
                    0 => {
                        let amt = rr(cpu, rm, pc) & 0xFF;
                        let (r, co) =
                            shift_op(rr(cpu, rn, pc), 2, amt, carry(cpu), true);
                        cpu.regs.r[rd] = r;
                        nz(cpu, r);
                        cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
                        adv(cpu, pc, 4);
                        return true;
                    }
                    8 => {
                        // UXTAB / UXTB (op2 = F:Rd:10:rot:Rm)
                        if o2 & 0xF0C0 != 0xF080 {
                            return fault(cpu, pc, op1, op2, 4);
                        }
                        let rot = ((o2 >> 4) & 3) * 8;
                        let v = ror32(rr(cpu, rm, pc), rot) & 0xFF;
                        cpu.regs.r[rd] = if rn == 15 {
                            v
                        } else {
                            rr(cpu, rn, pc).wrapping_add(v)
                        };
                        adv(cpu, pc, 4);
                        return true;
                    }
                    _ => return fault(cpu, pc, op1, op2, 4),
                }
            }
            9 => {
                // REV.W / REV16.W / REVSH.W / RBIT (op2[7:4] selects)
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                // Parallel ADD16 (sub 1/2/5/6 = Q/SH/UQ/UH lane flavor).
                // GAS: `qadd16 r0,r1,r2`=fa91 f012, `uqadd16`=fa91 f052,
                // `shadd16`=fa91 f022, `uhadd16`=fa91 f062. Two 16-bit lanes.
                let sub9 = (o2 >> 4) & 0xF;
                if sub9 == 1 || sub9 == 2 || sub9 == 5 || sub9 == 6 {
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let (hi, q1) =
                        lane16(an >> 16, am >> 16, false, sub9);
                    let (lo, q2) =
                        lane16(an & 0xFFFF, am & 0xFFFF, false, sub9);
                    cpu.regs.r[rd] = (hi << 16) | lo;
                    if q1 || q2 {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                let v = rr(cpu, rm, pc);
                cpu.regs.r[rd] = match (o2 >> 4) & 0xF {
                    8 => v.swap_bytes(),
                    9 => {
                        ((v & 0xFF) << 8)
                            | ((v >> 8) & 0xFF)
                            | ((v & 0xFF0000) << 8)
                            | ((v & 0xFF000000) >> 8)
                    }
                    10 => v.reverse_bits(),
                    11 => sx(((v & 0xFF) << 8) | ((v >> 8) & 0xFF), 16),
                    _ => return fault(cpu, pc, op1, op2, 4),
                };
                adv(cpu, pc, 4);
                return true;
            }
            11 => {
                // CLZ / RBIT
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let v = rr(cpu, rm, pc);
                cpu.regs.r[rd] = match (o2 >> 4) & 0xF {
                    8 => v.leading_zeros(),
                    10 => v.reverse_bits(),
                    _ => return fault(cpu, pc, op1, op2, 4),
                };
                adv(cpu, pc, 4);
                return true;
            }
            8 => {
                // QADD/QSUB/QDADD/QDSUB (op2[7:4]: 8/A/9/B; Rd=o2[11:8] is
                // variable — e.g. `qadd sl,r1,r0` is fa80 fa81, so gate
                // only o2[15:12]==F). Field layout is positional like
                // normal data-processing (Rn=op1-field, Rm=op2-field);
                // only the assembly TEXT lists Rm first (QADD Rd, Rm, Rn),
                // which once caused these to be computed swapped (QSUB
                // gave Rn-Rm: fuzz-found, 0x80000002 vs 0x7FFFFFFE).
                // Saturate + set Q on any saturation. (Fuzz-found hole.)
                if o2 & 0xF000 != 0xF000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let sub = (o2 >> 4) & 0xF;
                if sub == 4 {
                    // UADD8: per-byte add, GE[i] = carry out of lane i.
                    // (Drives memchr's SIMD search via SEL below; fuzz-found
                    // hole: faulted here.)
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let mut r: u32 = 0;
                    let mut ge: u32 = 0;
                    for i in 0..4 {
                        let s = ((an >> (i * 8)) & 0xFF) + ((am >> (i * 8)) & 0xFF);
                        r |= (s & 0xFF) << (i * 8);
                        ge |= ((s >> 8) & 1) << i;
                    }
                    cpu.regs.r[rd] = r;
                    cpu.regs.xpsr =
                        (cpu.regs.xpsr & !0xF0000) | (ge << 16);
                    adv(cpu, pc, 4);
                    return true;
                }
                // Parallel ADD8 (sub 1/2/5/6 = Q/SH/UQ/UH lane flavor).
                // GAS: `qadd8 r0,r1,r2`=fa81 f012, `uqadd8`=fa81 f052,
                // `shadd8`=fa81 f022, `uhadd8`=fa81 f062. Four 8-bit lanes.
                if sub == 1 || sub == 2 || sub == 5 || sub == 6 {
                    if o2 & 0xF000 != 0xF000 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let mut r: u32 = 0;
                    let mut q = false;
                    for i in 0..4 {
                        let a = ((an >> (i * 8)) & 0xFF) as u32;
                        let b = ((am >> (i * 8)) & 0xFF) as u32;
                        let (v, qq) = match sub {
                            // QADD8: signed saturate.
                            1 => {
                                let (sv, sq) = sat_lane(
                                    (a as i8 as i64) + (b as i8 as i64), 8, false,
                                );
                                (sv, sq)
                            }
                            // UQADD8: unsigned saturate.
                            5 => {
                                let (sv, sq) =
                                    sat_lane(a as i64 + b as i64, 8, true);
                                (sv, sq)
                            }
                            // SHADD8: (a+b)>>1 arithmetic.
                            2 => {
                                (((a as i8 as i32 + b as i8 as i32) >> 1) as u32
                                    & 0xFF, false)
                            }
                            // UHADD8: (a+b)>>1 logical.
                            _ => (((a + b) >> 1) & 0xFF, false),
                        };
                        r |= v << (i * 8);
                        q = q || qq;
                    }
                    cpu.regs.r[rd] = r;
                    if q {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                if sub != 8 && sub != 0xA && sub != 9 && sub != 0xB {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let m = rr(cpu, rm, pc) as i32 as i64;
                let n = rr(cpu, rn, pc) as i32 as i64;
                let (w, q) = match sub {
                    8 => {
                        let (r, q) = sat32x(m.wrapping_add(n));
                        (r as i64, q)
                    }
                    0xA => {
                        let (r, q) = sat32x(m.wrapping_sub(n));
                        (r as i64, q)
                    }
                    9 => {
                        let (d, q1) = sat32x(n.wrapping_mul(2));
                        let (r, q2) = sat32x(m.wrapping_add(d as i64));
                        (r as i64, q1 || q2)
                    }
                    _ => {
                        let (d, q1) = sat32x(n.wrapping_mul(2));
                        let (r, q2) = sat32x(m.wrapping_sub(d as i64));
                        (r as i64, q1 || q2)
                    }
                };
                cpu.regs.r[rd] = w as u32;
                if q {
                    cpu.regs.xpsr |= 0x08000000; // Q sticky
                }
                adv(cpu, pc, 4);
                return true;
            }
            10 => {
                // Parallel ASX (sub 1/2/5/6): exchange Rm's halves, then
                // ADD the top pair and SUBTRACT the bottom pair, per lane
                // flavor. GAS: `qasx r0,r1,r2`=faa1 f012 (UQ/SH/UH: o2
                // 0xF052/0xF022/0xF062; SAX forms live in op 0xE below).
                let suba = (o2 >> 4) & 0xF;
                if (suba == 1 || suba == 2 || suba == 5 || suba == 6)
                    && o2 & 0xF000 == 0xF000
                {
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let (hi, q1) = lane16(an >> 16, am & 0xFFFF, false, suba);
                    let (lo, q2) = lane16(an & 0xFFFF, am >> 16, true, suba);
                    cpu.regs.r[rd] = (hi << 16) | lo;
                    if q1 || q2 {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                // SEL: per-byte select on GE (set by UADD8/USUB8 above).
                // Rd[i] = GE[i] ? Rn[i] : Rm[i]. No flags affected.
                // (Census-found hole: doom's memchr SIMD needs it.)
                if (o2 & 0xF0F0) != 0xF080 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let ge = (cpu.regs.xpsr >> 16) & 0xF;
                let mut r: u32 = 0;
                for i in 0..4 {
                    let b = if (ge >> i) & 1 == 1 {
                        (an >> (i * 8)) & 0xFF
                    } else {
                        (am >> (i * 8)) & 0xFF
                    };
                    r |= b << (i * 8);
                }
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                return true;
            }
            12 => {
                // Parallel SUB8 (sub 1/2/5/6 = Q/SH/UQ/UH lane flavor).
                // GAS: `qsub8 r0,r1,r2`=fac1 f012 (UQ/SH/UH: o2
                // 0xF052/0xF022/0xF062). Four 8-bit lanes.
                let subc = (o2 >> 4) & 0xF;
                if (subc == 1 || subc == 2 || subc == 5 || subc == 6)
                    && o2 & 0xF000 == 0xF000
                {
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let mut r: u32 = 0;
                    let mut q = false;
                    for i in 0..4 {
                        let a = ((an >> (i * 8)) & 0xFF) as u32;
                        let b = ((am >> (i * 8)) & 0xFF) as u32;
                        let (v, qq) = match subc {
                            1 => {
                                let (sv, sq) = sat_lane(
                                    (a as i8 as i64) - (b as i8 as i64), 8, false,
                                );
                                (sv, sq)
                            }
                            5 => {
                                let (sv, sq) =
                                    sat_lane(a as i64 - b as i64, 8, true);
                                (sv, sq)
                            }
                            2 => {
                                (((a as i8 as i32 - b as i8 as i32) >> 1) as u32
                                    & 0xFF, false)
                            }
                            _ => (
                                ((a.wrapping_sub(b)) >> 1) & 0xFF,
                                false,
                            ),
                        };
                        r |= v << (i * 8);
                        q = q || qq;
                    }
                    cpu.regs.r[rd] = r;
                    if q {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                // USUB8 (op2[7:4]==4): per-byte subtract, GE[i] = NOT
                // borrow (Rn[i] >= Rm[i]). Sibling of UADD8 above.
                if (o2 & 0xF0F0) != 0xF040 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let mut r: u32 = 0;
                let mut ge: u32 = 0;
                for i in 0..4 {
                    let a = ((an >> (i * 8)) & 0xFF) as i32;
                    let b = ((am >> (i * 8)) & 0xFF) as i32;
                    r |= ((a.wrapping_sub(b) as u32) & 0xFF) << (i * 8);
                    ge |= ((a >= b) as u32) << i;
                }
                cpu.regs.r[rd] = r;
                cpu.regs.xpsr = (cpu.regs.xpsr & !0xF0000) | (ge << 16);
                adv(cpu, pc, 4);
                return true;
            }
            13 => {
                // Parallel SUB16 (sub 1/2/5/6 = Q/SH/UQ/UH lane flavor).
                // GAS: `qsub16 r0,r1,r2`=fad1 f012 (UQ/SH/UH: o2
                // 0xF052/0xF022/0xF062). Two 16-bit lanes, both subtract.
                let subd = (o2 >> 4) & 0xF;
                if (subd == 1 || subd == 2 || subd == 5 || subd == 6)
                    && o2 & 0xF000 == 0xF000
                {
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let (hi, q1) =
                        lane16(an >> 16, am >> 16, true, subd);
                    let (lo, q2) =
                        lane16(an & 0xFFFF, am & 0xFFFF, true, subd);
                    cpu.regs.r[rd] = (hi << 16) | lo;
                    if q1 || q2 {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                return fault(cpu, pc, op1, op2, 4);
            }
            14 => {
                // Parallel SAX (sub 1/2/5/6): exchange Rm's halves, then
                // SUBTRACT the top pair and ADD the bottom pair, per lane
                // flavor. GAS: `qsax r0,r1,r2`=fae1 f012 (UQ/SH/UH: o2
                // 0xF052/0xF022/0xF062).
                let sube = (o2 >> 4) & 0xF;
                if (sube == 1 || sube == 2 || sube == 5 || sube == 6)
                    && o2 & 0xF000 == 0xF000
                {
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let (hi, q1) = lane16(an >> 16, am & 0xFFFF, true, sube);
                    let (lo, q2) = lane16(an & 0xFFFF, am >> 16, false, sube);
                    cpu.regs.r[rd] = (hi << 16) | lo;
                    if q1 || q2 {
                        cpu.regs.xpsr |= 0x08000000; // Q sticky
                    }
                    adv(cpu, pc, 4);
                    return true;
                }
                return fault(cpu, pc, op1, op2, 4);
            }
            _ => return fault(cpu, pc, op1, op2, 4),
        }
        // ---- FB: multiply / divide ----
    } else if o1 & 0xFF00 == 0xFB00 {
        let op = (o1 >> 4) & 0xF;
        let rn = (o1 & 0xF) as usize;
        let ra = ((o2 >> 12) & 0xF) as usize;
        let rd = ((o2 >> 8) & 0xF) as usize;
        let rm = (o2 & 0xF) as usize;
        match op {
            0 => {
                let sub = (o2 >> 4) & 0xF;
                if sub == 0 {
                    if ra == 15 {
                        cpu.regs.r[rd] =
                            rr(cpu, rn, pc).wrapping_mul(rr(cpu, rm, pc));
                    } else {
                        cpu.regs.r[rd] = rr(cpu, ra, pc).wrapping_add(
                            rr(cpu, rn, pc).wrapping_mul(rr(cpu, rm, pc)),
                        );
                    }
                    adv(cpu, pc, 4);
                    return true;
                } else if sub == 1 {
                    if ra == 15 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    cpu.regs.r[rd] = rr(cpu, ra, pc).wrapping_sub(
                        rr(cpu, rn, pc).wrapping_mul(rr(cpu, rm, pc)),
                    );
                    adv(cpu, pc, 4);
                    return true;
                }
                return fault(cpu, pc, op1, op2, 4);
            }
            1 => {
                // SMLAXY (Ra!=15) / SMULXY (Ra==15). X/Y = bottom/top half
                // of Rn/Rm via op2[5]/op2[4] (0=B/low, 1=T/high).
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let hn = if (o2 >> 5) & 1 == 1 {
                    (an >> 16) as i16 as i32
                } else {
                    (an & 0xFFFF) as i16 as i32
                };
                let hm = if (o2 >> 4) & 1 == 1 {
                    (am >> 16) as i16 as i32
                } else {
                    (am & 0xFFFF) as i16 as i32
                };
                let p = hn.wrapping_mul(hm);
                cpu.regs.r[rd] = if ra == 15 {
                    p as u32
                } else {
                    (rr(cpu, ra, pc) as i32).wrapping_add(p) as u32
                };
                adv(cpu, pc, 4);
                return true;
            }
            2 => {
                // SMLAD (Ra!=15) / SMUAD (Ra==15), dual 16x16 + accumulate.
                // Only the plain form ([7:4]==0); X/SD variants fault loudly.
                // Q set on dual-sum OR accumulate signed overflow (this is
                // observable: fuzz showed Unicorn setting Q where we didn't).
                if o2 & 0xF0 != 0 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let lo = ((an & 0xFFFF) as i16 as i32)
                    .wrapping_mul((am & 0xFFFF) as i16 as i32);
                let hi = ((an >> 16) as i16 as i32)
                    .wrapping_mul((am >> 16) as i16 as i32);
                let s = lo.wrapping_add(hi);
                let mut q = (lo as i64 + hi as i64) != s as i64;
                let r = if ra == 15 {
                    s
                } else {
                    let acc = rr(cpu, ra, pc) as i32;
                    let r = acc.wrapping_add(s);
                    q |= (acc as i64 + s as i64) != r as i64;
                    r
                };
                cpu.regs.r[rd] = r as u32;
                if q {
                    cpu.regs.xpsr |= 0x08000000; // Q sticky
                }
                adv(cpu, pc, 4);
                return true;
            }
            3 => {
                // SMULW (Ra==15) / SMLAW: 32x16 -> top 32 bits.
                // Half via op2[4] (0=B/low, 1=T/high).
                let an = rr(cpu, rn, pc) as i32 as i64;
                let am = rr(cpu, rm, pc);
                let half = if (o2 >> 4) & 1 == 1 {
                    (am >> 16) as i16 as i64
                } else {
                    (am & 0xFFFF) as i16 as i64
                };
                let p = (an.wrapping_mul(half) >> 16) as u32;
                cpu.regs.r[rd] = if ra == 15 {
                    p
                } else {
                    rr(cpu, ra, pc).wrapping_add(p)
                };
                adv(cpu, pc, 4);
                return true;
            }
            4 => {
                // SMLSD (Ra!=15) / SMUSD (Ra==15): dual 16x16 SUBTRACT
                // plus accumulate. GAS: `smlsd` assembles to op 4 (e.g.
                // fb41 3002), NOT op 2 — the old code had no arm 4 and
                // faulted on it (fuzz-found stall right after SMLAD).
                // X-swapped-half variants (op2[7:4]!=0) fault loudly.
                if o2 & 0xF0 != 0 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let lo = ((an & 0xFFFF) as i16 as i32)
                    .wrapping_mul((am & 0xFFFF) as i16 as i32);
                let hi = ((an >> 16) as i16 as i32)
                    .wrapping_mul((am >> 16) as i16 as i32);
                let p = lo.wrapping_sub(hi);
                let mut q = (lo as i64 - hi as i64) != p as i64;
                let r = if ra == 15 {
                    p
                } else {
                    let acc = rr(cpu, ra, pc) as i32;
                    let r = acc.wrapping_add(p);
                    q |= (acc as i64 + p as i64) != r as i64;
                    r
                };
                cpu.regs.r[rd] = r as u32;
                if q {
                    cpu.regs.xpsr |= 0x08000000; // Q sticky
                }
                adv(cpu, pc, 4);
                return true;
            }
            8 => {
                // SMULL
                let a = rr(cpu, rn, pc) as i32 as i64;
                let b = rr(cpu, rm, pc) as i32 as i64;
                let p = a.wrapping_mul(b) as u64;
                cpu.regs.r[((o2 >> 12) & 0xF) as usize] = p as u32;
                cpu.regs.r[((o2 >> 8) & 0xF) as usize] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            9 => {
                // SDIV (1111_Rd_1111_Rm op2 shape) shares op 9 with SMLAL,
                // exactly like UDIV/UMLAL share op 11 (see arm 11 below).
                // Missing this ran every sdiv as multiply-accumulate (the
                // quotient came back as the dividend's high word, i.e. the
                // dividend itself — DOOM's (10*168/10) stayed 1680).
                if o2 & 0xF0F0 == 0xF0F0 {
                    let b = rr(cpu, rm, pc) as i32;
                    if b == 0 && div0_trap(cpu, sys, mem, pc) {
                        return cpu.fault.is_none();
                    }
                    cpu.regs.r[rd] = if b == 0 {
                        0
                    } else {
                        (rr(cpu, rn, pc) as i32).wrapping_div(b) as u32
                    };
                    adv(cpu, pc, 4);
                    return true;
                }
                // SMLAL
                let a = rr(cpu, rn, pc) as i32 as i64;
                let b = rr(cpu, rm, pc) as i32 as i64;
                let lo = ((o2 >> 12) & 0xF) as usize;
                let hi = ((o2 >> 8) & 0xF) as usize;
                let acc = ((cpu.regs.r[hi] as u64) << 32) | cpu.regs.r[lo] as u64;
                let p = (acc as i64).wrapping_add(a.wrapping_mul(b)) as u64;
                cpu.regs.r[lo] = p as u32;
                cpu.regs.r[hi] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            10 => {
                // UMULL
                let p = (rr(cpu, rn, pc) as u64).wrapping_mul(rr(cpu, rm, pc) as u64);
                cpu.regs.r[((o2 >> 12) & 0xF) as usize] = p as u32;
                cpu.regs.r[((o2 >> 8) & 0xF) as usize] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            11 => {
                // UDIV (1111_Rd_1111_Rm) or UMLAL
                if o2 & 0xF0F0 == 0xF0F0 {
                    let b = rr(cpu, rm, pc);
                    if b == 0 && div0_trap(cpu, sys, mem, pc) {
                        return cpu.fault.is_none();
                    }
                    cpu.regs.r[rd] = if b == 0 { 0 } else { rr(cpu, rn, pc) / b };
                    adv(cpu, pc, 4);
                    return true;
                }
                let lo = ((o2 >> 12) & 0xF) as usize;
                let hi = ((o2 >> 8) & 0xF) as usize;
                let acc = ((cpu.regs.r[hi] as u64) << 32) | cpu.regs.r[lo] as u64;
                let p = acc.wrapping_add(
                    (rr(cpu, rn, pc) as u64).wrapping_mul(rr(cpu, rm, pc) as u64),
                );
                cpu.regs.r[lo] = p as u32;
                cpu.regs.r[hi] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            12 => {
                // SMLAL (plain long MAC, o2[7:4]==0) or SMLALD (dual
                // 16x16 add into 64 bits, o2[7:4]==0xC). GAS assembles every
                // `smlal` (including high regs, e.g. `smlal r8, lr, r1, r0` =
                // fbc1 8e00) to op 12, which this decoder never had —
                // it faulted LOUDLY (fuzz-found; any firmware doing a
                // 64-bit accumulate with high regs died here). Field
                // layout mirrors the arm-9 form (lo=o2[15:12],
                // hi=o2[11:8]). Other o2[7:4] shapes fault loudly —
                // unimplemented, never silent.
                // (GAS: `smlald r0,r1,r2,r3` = fbc2 01c3.)
                let sub = (o2 >> 4) & 0xF;
                if sub != 0 && sub != 0xC {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let lo = ((o2 >> 12) & 0xF) as usize;
                let hi = ((o2 >> 8) & 0xF) as usize;
                let acc = ((cpu.regs.r[hi] as u64) << 32) | cpu.regs.r[lo] as u64;
                let p = if sub == 0 {
                    let a = rr(cpu, rn, pc) as i32 as i64;
                    let b = rr(cpu, rm, pc) as i32 as i64;
                    (acc as i64).wrapping_add(a.wrapping_mul(b)) as u64
                } else {
                    // SMLALD: acc += lo*lo + hi*hi (signed 16-bit halves).
                    let an = rr(cpu, rn, pc);
                    let am = rr(cpu, rm, pc);
                    let plo = (an as i16) as i64 * ((am as i16) as i64);
                    let phi = ((an >> 16) as i16) as i64 * (((am >> 16) as i16) as i64);
                    (acc as i64).wrapping_add(plo).wrapping_add(phi) as u64
                };
                cpu.regs.r[lo] = p as u32;
                cpu.regs.r[hi] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            13 => {
                // SDIV or SMLSLD: SDIV has the same F:F op2 shape
                if o2 & 0xF0F0 == 0xF0F0 {
                    let b = rr(cpu, rm, pc) as i32;
                    if b == 0 && div0_trap(cpu, sys, mem, pc) {
                        return cpu.fault.is_none();
                    }
                    cpu.regs.r[rd] = if b == 0 {
                        0
                    } else {
                        (rr(cpu, rn, pc) as i32).wrapping_div(b) as u32
                    };
                    adv(cpu, pc, 4);
                    return true;
                }
                // SMLSLD (dual 16x16 subtract into 64 bits, o2[7:4]==0xC;
                // GAS: `smlsld r0,r1,r2,r3` = fbd2 01c3): acc += lo*lo-hi*hi.
                // Other non-F:F shapes fault loudly — unimplemented.
                if (o2 & 0xF0) != 0xC0 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let lo = ((o2 >> 12) & 0xF) as usize;
                let hi = ((o2 >> 8) & 0xF) as usize;
                let acc = ((cpu.regs.r[hi] as u64) << 32) | cpu.regs.r[lo] as u64;
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let plo = (an as i16) as i64 * ((am as i16) as i64);
                let phi = ((an >> 16) as i16) as i64 * (((am >> 16) as i16) as i64);
                let p = (acc as i64).wrapping_add(plo).wrapping_sub(phi) as u64;
                cpu.regs.r[lo] = p as u32;
                cpu.regs.r[hi] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            14 => {
                // UMLAL (plain, o2[7:4]==0; GAS: `umlal r0,r1,r2,r3` =
                // fbe2 0103) or UMAAL (unsigned dual accumulate, o2[7:4]==6;
                // GAS: `umaal r4,r5,r6,r7` = fbe6 4567): result = Rn*Rm +
                // RdLo + RdHi (all unsigned 64-bit). Other op-14 forms fault
                // loudly. (Census-found hole: 3 sites in doom; unreached in
                // all exercised paths so far.)
                let sub = (o2 >> 4) & 0xF;
                if sub != 0 && sub != 6 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let lo = ((o2 >> 12) & 0xF) as usize;
                let hi = ((o2 >> 8) & 0xF) as usize;
                let p = if sub == 0 {
                    let acc = ((cpu.regs.r[hi] as u64) << 32) | cpu.regs.r[lo] as u64;
                    acc.wrapping_add(
                        (rr(cpu, rn, pc) as u64).wrapping_mul(rr(cpu, rm, pc) as u64),
                    )
                } else {
                    // UMAAL: result = Rn*Rm + RdLo + RdHi (the halves are
                    // added as full 64-bit values, not via acc, which would
                    // double-count them).
                    (rr(cpu, rn, pc) as u64)
                        .wrapping_mul(rr(cpu, rm, pc) as u64)
                        .wrapping_add(cpu.regs.r[lo] as u64)
                        .wrapping_add(cpu.regs.r[hi] as u64)
                };
                cpu.regs.r[lo] = p as u32;
                cpu.regs.r[hi] = (p >> 32) as u32;
                adv(cpu, pc, 4);
                return true;
            }
            5 => {
                // SMMUL (o2[15:12]==F, no accumulate) / SMMLA (elsewhere):
                // Rd = top32(Rn*Rn... precisely RoundDown(Rn*Rm) [+ Ra].
                // R=o2[4] rounds via +0x80000000 before the shift. Rd/Ra are
                // data (o2[15:8]) — the gate must NOT include them (faulted
                // every nonzero-Rd form; firmware-found).
                // GAS: `smmul r0,r1,r2`=fb51 f002, `smmulr`=fb51 f012,
                // `smmla r0,r1,r2,r3`=fb51 3002, `smmlar`=fb51 3012.
                if o2 & 0x00E0 != 0x0000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let round = (o2 & 0x10) != 0;
                let mut p = (rr(cpu, rn, pc) as i32 as i64)
                    .wrapping_mul(rr(cpu, rm, pc) as i32 as i64);
                if round {
                    p = p.wrapping_add(0x8000_0000);
                }
                let mut r = (p >> 32) as u32;
                if o2 & 0xF000 != 0xF000 {
                    r = rr(cpu, ra, pc).wrapping_add(r);
                }
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                return true;
            }
            6 => {
                // SMMLS/SMMLSR: Rd = Ra - RoundDown(Rn*Rm) (R=o2[4] rounds).
                // Rd/Ra are data — gate excludes them (see op 5 note).
                // GAS: `smmls r0,r1,r2,r3`=fb61 3002, `smmlsr`=fb61 3012.
                if o2 & 0x00E0 != 0x0000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let round = (o2 & 0x10) != 0;
                let mut p = (rr(cpu, rn, pc) as i32 as i64)
                    .wrapping_mul(rr(cpu, rm, pc) as i32 as i64);
                if round {
                    p = p.wrapping_add(0x8000_0000);
                }
                let r = rr(cpu, ra, pc).wrapping_sub((p >> 32) as u32);
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                return true;
            }
            7 => {
                // USAD8 (o2[15:12]==F: no accumulate) / USADA8 (elsewhere):
                // Rd = sum |Rn.byte[i]-Rm.byte[i]| (+ Ra). Rd/Ra are data
                // (o2[15:8]) — the gate must NOT include them (faulted every
                // nonzero-Rd form; firmware-found via usada8 r4,r1,lr,r3).
                // GAS: `usad8 r0,r1,r2`=fb71 f002, `usada8 r0,r1,r2,r3`=fb71 3002.
                if o2 & 0x00F0 != 0x0000 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let an = rr(cpu, rn, pc);
                let am = rr(cpu, rm, pc);
                let mut r = 0u32;
                for i in 0..4 {
                    let a = ((an >> (i * 8)) & 0xFF) as i32;
                    let b = ((am >> (i * 8)) & 0xFF) as i32;
                    r += (a - b).unsigned_abs();
                }
                if o2 & 0xF000 != 0xF000 {
                    r = r.wrapping_add(rr(cpu, ra, pc));
                }
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                return true;
            }
            _ => return fault(cpu, pc, op1, op2, 4),
        }
        // ---- EA/EB: shifted-register data processing ----
    } else if o1 & 0xFF00 == 0xEA00 || o1 & 0xFF00 == 0xEB00 {
        let b8 = (o1 >> 8) & 1;
        let b7 = (o1 >> 7) & 1;
        let b6 = (o1 >> 6) & 1;
        let b5 = (o1 >> 5) & 1;
        let s = (o1 & 0x10) != 0;
        let op = match (b8 << 3) | (b7 << 2) | (b6 << 1) | b5 {
            0b0010 => 2,  // ORR
            0b1000 => 8,  // ADD
            0b1010 => 10, // ADC
            0b1011 => 11, // SBC
            0b1101 => 13, // SUB
            0b1110 => 14, // RSB
            0b0000 => 0,  // AND
            0b0001 => 1,  // BIC
            0b0011 => 3,  // MVN/ORN
            0b0100 => 4,  // EOR
            0b0110 => 6,  // PKHBT/PKHTB
            _ => return fault(cpu, pc, op1, op2, 4),
        };
        let rn = (o1 & 0xF) as usize;
        let rd = ((o2 >> 8) & 0xF) as usize;
        let rm = (o2 & 0xF) as usize;
        if op == 6 {
            // PKHBT (Tb=o2[5]==0) / PKHTB (Tb==1): the BOTTOM halfword
            // always comes from Rn, the TOP from Rm (shifted). I.e. BT:
            // Rd = Rn[15:0] | (Rm LSL sh)[31:16]; TB: Rd = Rn[31:16] |
            // (Rm ASR sh)[15:0] (ASR #0 means #32). No flags affected.
            // (First version had top/bottom sources swapped — fuzz-found
            // against Unicorn: PKHBT(11223344,AABBCCDD) must be AABB3344.)
            if s {
                return fault(cpu, pc, op1, op2, 4);
            }
            let tb = (o2 >> 5) & 1;
            let sh = (((o2 >> 12) & 0x7) << 2) | ((o2 >> 6) & 0x3);
            let an = rr(cpu, rn, pc);
            let am = rr(cpu, rm, pc);
            let r = if tb == 0 {
                (an & 0x0000FFFF) | (am.wrapping_shl(sh) & 0xFFFF0000)
            } else {
                let a = if sh == 0 { 32 } else { sh };
                (an & 0xFFFF0000)
                    | ((((am as i32).wrapping_shr(a.min(32)) as u32)) & 0x0000FFFF)
            };
            cpu.regs.r[rd] = r;
            adv(cpu, pc, 4);
            return true;
        }
        let typ = (o2 >> 4) & 3;
        let amt = (((o2 >> 12) & 7) << 2) | ((o2 >> 6) & 3);
        let ci = carry(cpu);
        let (sv, co) = shift_op(rr(cpu, rm, pc), typ, amt, ci, false);
        let a = rr(cpu, rn, pc);
        if op == 2 && rn == 15 {
            // MOV (register)
            if rd == 15 {
                if s {
                    return fault(cpu, pc, op1, op2, 4);
                }
                return branch(cpu, sys, mem, sv, pc, op1, op2, 4);
            }
            cpu.regs.r[rd] = sv;
            if s {
                nz(cpu, sv);
                cpu.regs.xpsr = (cpu.regs.xpsr & !0x20000000) | (co << 29);
            }
            adv(cpu, pc, 4);
            return true;
        }
        if op == 3 && rn == 15 {
            if rd == 15 {
                return fault(cpu, pc, op1, op2, 4);
            }
            cpu.regs.r[rd] = !sv;
            if s {
                nz(cpu, !sv);
            }
            adv(cpu, pc, 4);
            return true;
        }
        let test = s && rd == 15;
        match alu_op(cpu, op, s, a, sv, ci, co, test) {
            Some(r) => {
                if rd == 15 {
                    return branch(cpu, sys, mem, r, pc, op1, op2, 4);
                }
                cpu.regs.r[rd] = r;
                adv(cpu, pc, 4);
                true
            }
            None => {
                if test {
                    adv(cpu, pc, 4);
                    true
                } else {
                    fault(cpu, pc, op1, op2, 4)
                }
            }
        }
        // ---- E8/E9: LDM/STM, STRD/LDRD, LDREX/STREX, TBB/TBH ----
    } else if (o1 & 0xF000) == 0xE000 && o1 < 0xEC00 {
        let rn = (o1 & 0xF) as usize;
        // TBB / TBH: E8D0|Rn + F000/F010 op2. The table index is the
        // VALUE in Rm (rr), not the register number — using the number
        // dispatched every switch on Rm's encoding (DOOM always played
        // demo2: tbb [pc,r3] used table[3] for any demosequence).
        // Table base is pc+4 with NO word masking: a halfword-aligned TBB
        // (like D_Display's at 0x1CA6) is followed immediately by its table;
        // masking back to 0x1CA8 reads entries shifted by 2 (case 3 went to
        // the status-bar tail instead of D_PageDrawer, so no title drew).
        if (o1 & 0x0FF0) == 0x08D0 && (o2 & 0xFFF0) == 0xF000 {
            let tab = if rn == 15 { pc.wrapping_add(4) } else { rr(cpu, rn, pc) };
            let idx = rr(cpu, rm_of(o2) as usize, pc);
            let t = pc.wrapping_add(4).wrapping_add((mem.read8(tab.wrapping_add(idx)) as u32) * 2);
            return branch(cpu, sys, mem, t | 1, pc, op1, op2, 4);
        }
        if (o1 & 0x0FF0) == 0x08D0 && (o2 & 0xFFF0) == 0xF010 {
            let tab = if rn == 15 { pc.wrapping_add(4) } else { rr(cpu, rn, pc) };
            let idx = rr(cpu, rm_of(o2) as usize, pc);
            let t = pc
                .wrapping_add(4)
                .wrapping_add((mem.read16(tab.wrapping_add(idx.wrapping_mul(2))) as u32) * 2);
            return branch(cpu, sys, mem, t | 1, pc, op1, op2, 4);
        }
        // LDREX / STREX. Store nibbles: 0x0840 (word, o2 = Rt:Rd:imm8,
        // imm scaled x4) or 0x08C0 (byte/halfword, o2 = Rt:F:size:Rd, no
        // offset). Load nibbles below. GAS: `strex r0,r1,[r2]`=e842 1000,
        // `strexb r0,r1,[r2]`=e8c2 1f40. Single-threaded, so STREX always
        // reports success (Rd-status = 0).
        if (o1 & 0x0FF0) == 0x0840 || (o1 & 0x0FF0) == 0x08C0 {
            let rt = ((o2 >> 12) & 0xF) as usize;
            if (o1 & 0x0FF0) == 0x08C0 {
                // Byte/halfword: o2 = Rt:F:size:Rd, no offset form exists.
                if o2 & 0x0F00 != 0x0F00 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let size = (o2 >> 4) & 0xF;
                if size != 4 && size != 5 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let rd = (o2 & 0xF) as usize;
                let v = rr(cpu, rt, pc);
                if size == 4 {
                    mem.write8(rr(cpu, rn, pc), (v & 0xFF) as u8);
                } else {
                    let a = rr(cpu, rn, pc);
                    mem.write8(a, (v & 0xFF) as u8);
                    mem.write8(a.wrapping_add(1), ((v >> 8) & 0xFF) as u8);
                }
                cpu.regs.r[rd] = 0;
                adv(cpu, pc, 4);
                return true;
            }
            // Word form (nibble 0x0840 only): o2 = Rt:Rd:imm8.
            let rd = ((o2 >> 8) & 0xF) as usize;
            let addr =
                rr(cpu, rn, pc).wrapping_add((o2 & 0xFF).wrapping_mul(4));
            mem.write32(addr, rr(cpu, rt, pc));
            cpu.regs.r[rd] = 0;
            adv(cpu, pc, 4);
            return true;
        }
        if (o1 & 0x0FF0) == 0x0850 || (o1 & 0x0FF0) == 0x08D0 {
            if o2 & 0x0F00 != 0x0F00 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let rt = ((o2 >> 12) & 0xF) as usize;
            if (o1 & 0x0FF0) == 0x08D0 {
                // Byte/halfword: [3:0] fixed F (no offset form exists).
                let size = (o2 >> 4) & 0xF;
                if (size != 4 && size != 5) || o2 & 0xF != 0xF {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let addr = rr(cpu, rn, pc);
                cpu.regs.r[rt] = if size == 4 {
                    mem.read8(addr) as u32
                } else {
                    (mem.read8(addr) as u32)
                        | ((mem.read8(addr.wrapping_add(1)) as u32) << 8)
                };
                adv(cpu, pc, 4);
                return true;
            }
            // Word form (nibble 0x0850): imm8 is address bits (scaled x4),
            // never a size select — even 0x4x/0x5x values are offsets.
            let addr =
                rr(cpu, rn, pc).wrapping_add((o2 & 0xFF).wrapping_mul(4));
            cpu.regs.r[rt] = mem.read32(addr);
            adv(cpu, pc, 4);
            return true;
        }
        // LDM / STM (bits[11:9]==100, bit6==0; IA iff bit8==0, W=bit5, L=bit4)
        if (o1 & 0x0E40) == 0x0800 {
            let ia = o1 & 0x0100 == 0;
            let w = o1 & 0x0020 != 0;
            let l = o1 & 0x0010 != 0;
            let list = o2;
            let n = list.count_ones() as u32;
            if n == 0 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let mut a = cpu.regs.r[rn];
            if !ia {
                a = a.wrapping_sub(n * 4);
                if w {
                    cpu.regs.r[rn] = a;
                }
            }
            let mut newpc: Option<u32> = None;
            for i in 0..16 {
                if (list >> i) & 1 == 1 {
                    if l {
                        let v = mem.read32(a);
                        if i == 15 {
                            newpc = Some(v);
                        } else {
                            // (If Rn itself is in a writeback list the
                            // loaded value wins; firmware never does this.)
                            cpu.regs.r[i as usize] = v;
                        }
                    } else {
                        let v = if i == 15 { (pc + 4) & !3 } else { cpu.regs.r[i as usize] };
                        mem.write32(a, v);
                    }
                    a += 4;
                }
            }
            if w && ia && !(l && (list >> rn) & 1 == 1) {
                cpu.regs.r[rn] = a;
            }
            if let Some(t) = newpc {
                return branch(cpu, sys, mem, t, pc, op1, op2, 4);
            }
            adv(cpu, pc, 4);
            return true;
        }
        // STRD / LDRD: same bits[11:9]==100 as LDM/STM but bit6==1
        // (complementary patterns 0x0800 vs 0x0840; GAS-verified).
        if (o1 & 0x0E40) == 0x0840 {
            let p = o1 & 0x0100 != 0;
            let u = o1 & 0x0080 != 0;
            let w = o1 & 0x0020 != 0;
            let l = o1 & 0x0010 != 0;
            if !p && !w {
                return fault(cpu, pc, op1, op2, 4); // P=0,W=0: UNDEFINED
            }
            // GAS-verified: first reg Rt = op2[15:12], second Rt2 = op2[11:8].
            let rt = ((o2 >> 12) & 0xF) as usize;
            let rt2 = ((o2 >> 8) & 0xF) as usize;
            // LDRD/STRD imm8 is word-scaled (GAS: `ldrd [r3],#8` = op2 0x02).
            let off = (o2 & 0xFF) * 4;
            let base = rr(cpu, rn, pc);
            // Post-indexed (P=0, e.g. `ldrd r1,r2,[r3],#8` — used for
            // 64-bit struct copies; faulted here before): access at the
            // unmodified base, writeback after.
            let addr = if p {
                if u {
                    base.wrapping_add(off)
                } else {
                    base.wrapping_sub(off)
                }
            } else {
                base
            };
            if l {
                cpu.regs.r[rt] = mem.read32(addr);
                cpu.regs.r[rt2] = mem.read32(addr.wrapping_add(4));
            } else {
                mem.write32(addr, rr(cpu, rt, pc));
                mem.write32(addr.wrapping_add(4), rr(cpu, rt2, pc));
            }
            if w {
                cpu.regs.r[rn] = if p {
                    addr
                } else if u {
                    base.wrapping_add(off)
                } else {
                    base.wrapping_sub(off)
                };
            }
            adv(cpu, pc, 4);
            return true;
        }
        return fault(cpu, pc, op1, op2, 4);
    } else if (o1 & 0xF000) == 0xE000 {
        // ---- EC/ED/EE/EF: FPU (VFPv4-SP, coproc 10/11) ----
        // Bitfield ground truth: docs/encodings/fpu*.s (see README there).
        // (Sd=(Vd<<1)|D, Sn=(Vn<<1)|N, Sm=(Vm<<1)|M; D-lists use D:Vd with
        // D HIGH; VLDM counts imm8 S-regs / imm8/2 D-regs; VCVT frac =
        // 32-2*imm4-opbit; VFPExpandImm pinned by vmov #1.0/#2.0/#-0.5/#6.75).
        // No other coprocessor exists on the M4F: non-0xA/0xB is a fault.
        if (o2 >> 8) & 0xF != 0xA && (o2 >> 8) & 0xF != 0xB {
            return fault(cpu, pc, op1, op2, 4);
        }
        // CPACR gate: CP10+CP11 need full access AND FPEXC.EN set, plus
        // FPCCR.USER for unprivileged use — else UsageFault NOCP (UFSR
        // bit 3 = CFSR bit 19 latched; without delivery this is a loud
        // fault like SVC — polling firmware never touches the FPU without
        // enabling it, so hitting this is a bug worth surfacing). Without
        // SHCSR.USGFAULTENA it escalates to HardFault. Each successful FPU
        // arm sets CONTROL.FPCA like hardware (drives lazy stacking on
        // exception entry).
        let cpacr_ok = sys.p.read(sys, 0xE000ED88, 4) & 0x00F0_0000 == 0x00F0_0000;
        let fpccr = sys.p.read(sys, 0xE000EF34, 4);
        let user_ok = crate::system::current_privileged() || fpccr & 2 != 0;
        if !cpacr_ok || !sys.p.fpu_fpexc_en() || !user_ok {
            let cfsr = sys.p.read(sys, 0xE000ED28, 4);
            sys.p.write(sys, 0xE000ED28, 4, cfsr | 0x0008_0000);
            if !cpu.deliver_irqs {
                return fault(cpu, pc, op1, op2, 4);
            }
            adv(cpu, pc, 4);
            let target = if sys.p.read(sys, 0xE000ED24, 4) & (1 << 18) != 0 { -10 } else { -13 };
            cpu.raise_sync(sys, mem, target);
            return cpu.fault.is_none();
        }
        // Lazy-stacking completion: a pending lazy FP context (LSPACT set
        // by exception entry) stacks S0-S15 into the FPCAR frame on the
        // FIRST FPU instruction executed in handler mode, then clears
        // LSPACT. Thread mode never takes this path (its FP state is live).
        // Keys off LSPACT alone: a reserve implies ASPEN+LSPEN held then.
        if cpu.ipsr != 0 {
            let fpccr = sys.p.read(sys, 0xE000EF34, 4);
            if fpccr & 1 != 0 {
                let fpcar = sys.p.read(sys, 0xE000EF38, 4);
                // Pre-validate like exception frames (MLSPERR + loud halt
                // on violation — same unrecoverable rule).
                if crate::system::is_mpu_enabled()
                    && sys.p.mpu_check(fpcar, 64, true, false).is_some()
                {
                    crate::system::latch_memmanage_fault(sys, 1 << 5, None);
                    return fault(cpu, pc, op1, op2, 4);
                }
                for i in 0..16 {
                    mem.write32(fpcar.wrapping_add(4 * i as u32), cpu.regs.s[i]);
                }
                sys.p.write(sys, 0xE000EF34, 4, fpccr & !1);
            }
        }
        // ---- (b) moves + MF/VMSR + VLDR/VSTR/VLDM/VSTM ----
        // S-numbering (probe-verified): Sd=(Vd<<1)|D, Sn=(Vn<<1)|N,
        // Sm=(Vm<<1)|M with D=o1[6], N=o2[7], M=o2[5]. D-lists use D:Vd
        // (D HIGH). sz=o2[8] must be 0 (no double-precision datapath on
        // FPv4-SP). Every successful arm sets CONTROL.FPCA like hardware.
        let vd4 = ((o2 >> 12) & 0xF) as usize;
        let dbit = ((o1 >> 6) & 1) as usize;
        let nbit = ((o2 >> 7) & 1) as usize;
        let mbit = ((o2 >> 5) & 1) as usize;
        let sd = (vd4 << 1) | dbit;
        let sn = (((o1 & 0xF) as usize) << 1) | nbit;
        let sm = (((o2 & 0xF) as usize) << 1) | mbit;
        // ---- EE: VMRS/VMSR + VMOV family (prefix-gated: the opc1/op
        // shapes below also match EC/ED multiples — e.g. VPOP ECBD 0A04
        // hits the VMOV-imm shape — so the EE prefix must be verified).
        if (o1 & 0xFF00) == 0xEE00 {
        // VMRS / VMSR (op1 selects the register — GAS fpu16.s — AND op2lo
        // == 0x10 AND sz == 0; Rt = Vd field, 0xF = APSR_nzcv for FPSCR).
        // op1: EEE1 = VMSR FPSCR; EEF1 = VMRS FPSCR; EEF7 = MVFR0;
        // EEF6 = MVFR1; EEF5 = MVFR2 (M4F IDs, same consts as MMIO);
        // EEF8/EEE8 = VMRS/VMSR FPEXC.
        // FPEXC: bit 31 (EX) is live LSPACT (outstanding lazy state);
        // bit 30 (EN) is CPACR-full && the VMSR-writable EN shadow (reset
        // set). Clearing EN bricks FPU access until reset — including the
        // VMSR that would re-enable it — which is exactly what silicon
        // does (a disabled coprocessor faults ALL cp10/11 insns).
        // Full-shape gate (as with VMOV-core): B-group ops with a high odd
        // dest (e.g. vsqrt s17,s18 = EEF1 8AC9) share these op1s and must
        // fall through (same lesson as VMOV-core vs VMLA below).
        if (o1 == 0xEEF1 || o1 == 0xEEE1 || o1 == 0xEEF7 || o1 == 0xEEF6
            || o1 == 0xEEF5 || o1 == 0xEEF8 || o1 == 0xEEE8)
            && (o2 & 0xFF) == 0x10
            && (o2 >> 8) & 1 == 0
        {
            if o1 == 0xEEF1 {
                if vd4 == 0xF {
                    cpu.regs.xpsr = (cpu.regs.xpsr & !0xF000_0000) | (cpu.regs.fpscr & 0xF000_0000);
                } else {
                    if vd4 == 13 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    cpu.regs.r[vd4] = cpu.regs.fpscr;
                }
            } else if o1 == 0xEEE1 {
                if vd4 == 13 || vd4 == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                cpu.regs.fpscr = (cpu.regs.fpscr & !0xFFC0_01FF)
                    | (rr(cpu, vd4, pc) & 0xFFC0_01FF);
            } else if o1 == 0xEEF8 {
                // VMRS FPEXC: EX = live LSPACT, EN = effective enable.
                if vd4 == 13 || vd4 == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let ex = (sys.p.read(sys, 0xE000EF34, 4) & 1) << 31;
                let en = u32::from(
                    sys.p.read(sys, 0xE000ED88, 4) & 0x00F0_0000 == 0x00F0_0000
                        && sys.p.fpu_fpexc_en(),
                ) << 30;
                cpu.regs.r[vd4] = ex | en;
            } else if o1 == 0xEEE8 {
                // VMSR FPEXC: only EN (bit 30) is writable.
                if vd4 == 13 || vd4 == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                sys.p.set_fpu_fpexc_en(rr(cpu, vd4, pc) & (1 << 30) != 0);
            } else {
                // MVFR0/1/2: read-only ID values (no APSR form: Rt=15
                // UNPREDICTABLE here, unlike the FPSCR form above).
                if vd4 == 13 || vd4 == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                cpu.regs.r[vd4] = if o1 == 0xEEF7 {
                    crate::peripherals::fpu::MVFR0
                } else if o1 == 0xEEF6 {
                    crate::peripherals::fpu::MVFR1
                } else {
                    crate::peripherals::fpu::MVFR2
                };
            }
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        // VMOV core<->S (Vn field varies in o1 low nibble). The op1 shape
        // (EE10/EE00) is SHARED with VNMLA/VNMLS/VMLA/VMLS, so the full
        // shape (op2lo == 0x10|N<<7, sz == 0) must match before claiming;
        // otherwise fall through to the 3-reg arm. Rt=13/15 UNPREDICTABLE.
        if ((o1 & 0xFFF0) == 0xEE10 || (o1 & 0xFFF0) == 0xEE00)
            && ((o2 & 0xFF) == 0x10 || (o2 & 0xFF) == 0x90)
            && (o2 >> 8) & 1 == 0
        {
            if vd4 == 13 || vd4 == 15 {
                return fault(cpu, pc, op1, op2, 4);
            }
            if (o1 & 0xFFF0) == 0xEE10 {
                cpu.regs.r[vd4] = cpu.regs.s[sn];
            } else {
                cpu.regs.s[sn] = rr(cpu, vd4, pc);
            }
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        // VMOV-imm (opc1-field==1D11, i.e. 0xB/0xF covering D=0/1, with
        // op2[7:4]==0): imm8 = opc2:op2[3:0]. sz must be 0. (The D bit
        // lives inside the opc1 nibble for this form, hence B|F.)
        if (((o1 >> 4) & 0xF) == 0xB || ((o1 >> 4) & 0xF) == 0xF) && (o2 & 0xF0) == 0x00 && (o2 >> 8) & 1 == 0 {
            cpu.regs.s[sd] = vfp_expand_imm(((o1 & 0xF) << 4) | (o2 & 0xF));
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        // ---- (c) EE data-processing op selector ----
        // The D bit (o1[6]) is NOT part of the opcode: opc = bit23 :
        // bits21:20 : op2[6] (4 bits). The old (o1>>4)&0xF form included
        // D and misdecoded every odd-high-reg arithmetic op (firmware hit
        // vadd s15,s13,s15 = EE76 7AA7). GAS table (fpu*.s, fpu11-13.s):
        // 0 MLA, 1 MLS, 2 NMLS, 3 NMLA, 4 MUL, 5 NMUL, 6 ADD, 7 SUB,
        // 8 DIV, 10 FNMS, 11 FNMA, 12 FMA, 13 FMS (9 = no op).
        // MLA/MLS are UNFUSED (mul then add in f32 — never mul_add);
        // fused forms use single rounding (exact u128 path).
        // sz (o2[8]) must be 0 and op2[4] must be 0 throughout (both
        // GAS-verified fixed; fault otherwise, never silent).
        let opc = (((o1 >> 7) & 1) << 3) | (((o1 >> 4) & 0x3) << 1) | ((o2 >> 6) & 1);
        let fpscr0 = cpu.regs.fpscr;
        if opc <= 8 || opc == 10 || opc == 11 || opc == 12 || opc == 13 {
            if (o2 >> 8) & 1 != 0 || (o2 & 0x10) != 0 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let (r, fl) = match opc {
                6 => fpu_add(fpscr0, cpu.regs.s[sn], cpu.regs.s[sm], false),
                7 => fpu_add(fpscr0, cpu.regs.s[sn], cpu.regs.s[sm], true),
                4 => fpu_mul(fpscr0, cpu.regs.s[sn], cpu.regs.s[sm], false),
                5 => fpu_mul(fpscr0, cpu.regs.s[sn], cpu.regs.s[sm], true),
                8 => fpu_div(fpscr0, cpu.regs.s[sn], cpu.regs.s[sm]),
                0 => fpu_mla(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], false, false),
                1 => fpu_mla(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], true, false),
                3 => fpu_mla(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], false, true),
                2 => fpu_mla(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], true, true),
                12 => fpu_fma(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], false, false),
                13 => fpu_fma(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], true, false),
                11 => fpu_fma(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], false, true),
                10 => fpu_fma(fpscr0, cpu.regs.s[sd], cpu.regs.s[sn], cpu.regs.s[sm], true, true),
                _ => return fault(cpu, pc, op1, op2, 4),
            };
            cpu.regs.s[sd] = r;
            cpu.regs.fpscr |= fl;
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        // ---- (c) B-group misc: MOV-reg/ABS/NEG/SQRT/CMP/VCVT ----
        // Zone: bit23==1 && bits21:20==3 (opc 14/15). The op selector is
        // op2[7,6,4] (opb3, 3 bits) — op2[5] is M (Sm data) and MUST NOT
        // participate: the old op2[7:4]-nibble match baked in M=1 and
        // missed every even-Sm form (faulted valid vcvt/f16/vmov/cmp).
        // sz (o2[8]) must be 0. GAS: fpu*.s + fpu12.s (vcmpe) + fpu13.s.
        {
            if opc < 14 {
                return fault(cpu, pc, op1, op2, 4);
            }
            if (o2 >> 8) & 1 != 0 {
                return fault(cpu, pc, op1, op2, 4);
            }
            let opc2 = o1 & 0xF;
            let opb3 = (((o2 >> 6) & 0x3) << 1) | ((o2 >> 4) & 0x1);
            match (opc2, opb3) {
                (0, 2) => {
                    cpu.regs.s[sd] = cpu.regs.s[sm]; // VMOV-reg
                }
                (0, 6) => {
                    cpu.regs.s[sd] = cpu.regs.s[sm] & !0x8000_0000; // VABS (no flags, even SNaN)
                }
                (1, 2) => {
                    cpu.regs.s[sd] = cpu.regs.s[sm] ^ 0x8000_0000; // VNEG (no flags)
                }
                (1, 6) => {
                    let (r, fl) = fpu_sqrt(fpscr0, cpu.regs.s[sm]);
                    cpu.regs.s[sd] = r;
                    cpu.regs.fpscr |= fl;
                }
                (4, 2) | (5, 2) | (4, 6) | (5, 6) => {
                    // VCMP reg / #0 and VCMPE reg / #0 (quiet opb3=2,
                    // signaling-E opb3=6; GAS fpu12.s). NZCV -> FPSCR (never
                    // xPSR); unordered (any NaN) = N=0,Z=0,C=1,V=1. IOC on
                    // SNaN always, and on QNaN too for the E form. #0 forms
                    // (opc2=5) have no Sm: M must be 0.
                    // NOTE: the first source lives in the Sd field
                    // (Vd<<1|D), NOT Sn — op1[19:16] is opc2=4/5 here.
                    if opc2 == 5 && mbit != 0 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    let exc = opb3 == 6;
                    let a = fpu_flush(cpu.regs.s[sd], fpscr0);
                    let b = if opc2 == 5 { 0 } else { fpu_flush(cpu.regs.s[sm], fpscr0) };
                    let mut fl = 0;
                    let nzcv = if f32_nan(a) || f32_nan(b) {
                        if f32_snan(a) || f32_snan(b) || exc {
                            fl |= FPSCR_IOC;
                        }
                        0x3
                    } else {
                        let af = f32::from_bits(a);
                        let bf = f32::from_bits(b);
                        ((af < bf) as u32) << 3 | ((af == bf) as u32) << 2 | (!(af < bf) as u32) << 1
                    };
                    cpu.regs.fpscr = (fpscr0 & !0xF000_0000) | (nzcv << 28);
                    cpu.regs.fpscr |= fl;
                }
                (8, 6) | (8, 2) => {
                    // VCVT.f32.s32/u32 (opb3: 6=signed, 2=unsigned — the
                    // sign lives in op2[7:6], M:Vm is the source). RNE via
                    // `as`; other RModes need explicit adjust.
                    let signed = opb3 == 6;
                    let iv = cpu.regs.s[sm];
                    let v: f64 = if signed { (iv as i32) as f64 } else { iv as f64 };
                    let (r, inexact) = fpu_round_f32(v, fpu_rmode(fpscr0));
                    cpu.regs.s[sd] = r;
                    if inexact {
                        cpu.regs.fpscr |= FPSCR_IXC;
                    }
                }
                (0xC, 6) | (0xD, 6) => {
                    // VCVT.s32/u32.f32 (opc2[0] = signed; opb3==6 already
                    // gates bit7=1,bit6=1,bit4=0 while M:Vm stay data).
                    // RMode rounding, saturate + IOC on invalid/overflow.
                    let signed = opc2 == 0xD;
                    let x = f32::from_bits(fpu_flush(cpu.regs.s[sm], fpscr0)) as f64;
                    let (r, invalid) = fpu_round_int(x, signed, fpu_rmode(fpscr0));
                    if invalid {
                        let sat = if x.is_nan() {
                            0
                        } else if x > 0.0 {
                            if signed { 0x7FFF_FFFF } else { 0xFFFF_FFFF }
                        } else if signed {
                            0x8000_0000
                        } else {
                            0
                        };
                        cpu.regs.s[sd] = sat;
                        cpu.regs.fpscr |= FPSCR_IOC;
                    } else {
                        cpu.regs.s[sd] = r;
                        // Inexact-but-valid rounding. Signed compare must go
                        // via i32 (r=0x80000000 as f64 is +2^31, != x=-2^31).
                        let exact = if signed { (r as i32) as f64 == x } else { r as f64 == x };
                        if !exact {
                            cpu.regs.fpscr |= FPSCR_IXC;
                        }
                    }
                }
                (0xA, _) | (0xB, _) | (0xE, _) | (0xF, _) => {
                    // VCVT fixed<->float. frac N = 32-2*imm4-opbit (GAS:
                    // 0xC8->16, 0xEF->1, 0xC0->32, 0xE0->31, 0xCF->2).
                    // opc2[2]: 0 = to-float (A/B), 1 = to-fixed (E/F);
                    // opc2[0]: 0 = signed (A/E), 1 = unsigned (B/F).
                    let hi = (o2 >> 4) & 0xF;
                    if hi != 0xC && hi != 0xE {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    let imm4 = o2 & 0xF;
                    let n = 32 - 2 * imm4 as i32 - ((o2 >> 5) & 1) as i32;
                    if n < 1 || n > 32 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    let signed = (opc2 & 1) == 0;
                    let to_fixed = (opc2 & 4) != 0;
                    // Fixed-point VCVT requires Sd == Sm (GAS rejects
                    // distinct regs), so the single operand lives in sd.
                    if !to_fixed {
                        let iv = cpu.regs.s[sd];
                        let base: f64 = if signed { (iv as i32) as f64 } else { iv as f64 };
                        let v = base / 2f64.powi(n);
                        let (r, inexact) = fpu_round_f32(v, fpu_rmode(fpscr0));
                        cpu.regs.s[sd] = r;
                        let mut fl = if inexact { FPSCR_IXC } else { 0 };
                        fl |= fpu_ou_flags(r);
                        cpu.regs.fpscr |= fl;
                    } else {
                        let x = f32::from_bits(fpu_flush(cpu.regs.s[sd], fpscr0)) as f64 * 2f64.powi(n);
                        let (r, invalid) = fpu_round_int(x, signed, fpu_rmode(fpscr0));
                        if invalid {
                            let sat = if x.is_nan() {
                                0
                            } else if x > 0.0 {
                                if signed { 0x7FFF_FFFF } else { 0xFFFF_FFFF }
                            } else if signed {
                                0x8000_0000
                            } else {
                                0
                            };
                            cpu.regs.s[sd] = sat;
                            cpu.regs.fpscr |= FPSCR_IOC;
                        } else {
                            cpu.regs.s[sd] = r;
                            let exact = if signed { (r as i32) as f64 == x } else { r as f64 == x };
                            if !exact {
                                cpu.regs.fpscr |= FPSCR_IXC;
                            }
                        }
                    }
                }
                (2, 2) | (2, 6) | (3, 2) | (3, 6) => {
                    // VCVT f16<->f32. opc2[0]: 0 = f16->f32, 1 = f32->f16;
                    // opb3: 2 = bottom half (op2[7]=0), 6 = top (op2[7]=1).
                    let from_half = opc2 == 2;
                    let top = opb3 == 6;
                    if from_half {
                        let sh = cpu.regs.s[sm];
                        let h = if top { (sh >> 16) as u16 } else { sh as u16 };
                        if (h & 0x7C00) == 0x7C00 && (h & 0x3FF) != 0 && (h & 0x200) == 0 {
                            cpu.regs.fpscr |= FPSCR_IOC; // f16 SNaN
                        }
                        let r = f16_to_f32_bits(h);
                        cpu.regs.s[sd] = fpu_dn(r, fpscr0);
                    } else {
                        let w = fpu_flush(cpu.regs.s[sm], fpscr0);
                        if f32_snan(w) {
                            cpu.regs.fpscr |= FPSCR_IOC;
                        }
                        let (h, fl) = f32_to_f16_bits(w, fpu_rmode(fpscr0));
                        // DN: a NaN narrow result becomes the default NaN
                        // (sign preserved).
                        let hn = if (h & 0x7C00) == 0x7C00 && (h & 0x3FF) != 0 && fpscr0 & (1 << 25) != 0 {
                            (h & 0x8000) | 0x7E00
                        } else {
                            h
                        };
                        let dst = cpu.regs.s[sd];
                        cpu.regs.s[sd] = if top {
                            (dst & 0xFFFF) | ((hn as u32) << 16)
                        } else {
                            (dst & 0xFFFF_0000) | hn as u32
                        };
                        cpu.regs.fpscr |= fl;
                    }
                }
                _ => return fault(cpu, pc, op1, op2, 4),
            }
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        } // end EE prefix gate
        // ---- EC/ED: VLDM/VSTM/VPUSH/VPOP (multi) vs VLDR/VSTR (single) ----
        // Selected by P/U/W, not by prefix: IA (P=0,U=1) multiples happen
        // to assemble under EC, DB (P=1,U=0, needs W=1) under ED, and the
        // offset single (P=1,U=1,W=0) under ED. GAS-probed: DB without W
        // and VLDR/VSTR writeback forms do not exist.
        if (o1 & 0xFF00) == 0xEC00 || (o1 & 0xFF00) == 0xED00 {
            // VMOV Rt,Rt2,Dm / Dm,Rt,Rt2 (EC-only, op1[20] selects direction;
            // Dm = M:Vm <= 15). Must precede the P/U/W logic (P=0,U=0 here).
            if (o1 & 0xFFF0) == 0xEC50 || (o1 & 0xFFF0) == 0xEC40 {
                if (o2 & 0xD0) != 0x10 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let dm = (mbit << 4) | (o2 & 0xF) as usize;
                let rt2 = (o1 & 0xF) as usize;
                if dm > 15 || vd4 == 13 || vd4 == 15 || rt2 == 13 || rt2 == 15 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                if (o1 & 0xFFF0) == 0xEC50 {
                    cpu.regs.r[vd4] = cpu.regs.s[2 * dm];
                    cpu.regs.r[rt2] = cpu.regs.s[2 * dm + 1];
                } else {
                    cpu.regs.s[2 * dm] = rr(cpu, vd4, pc);
                    cpu.regs.s[2 * dm + 1] = rr(cpu, rt2, pc);
                }
                cpu.regs.control |= 4;
                adv(cpu, pc, 4);
                return true;
            }
            let p = (o1 >> 8) & 1;
            let u = (o1 >> 7) & 1;
            let w = (o1 >> 5) & 1;
            let l = (o1 >> 4) & 1;
            let rn = (o1 & 0xF) as usize;
            let single = (o2 >> 8) & 1 == 0;
            let imm8 = (o2 & 0xFF) as usize;
            if p == 1 && u == 1 && w == 0 {
                // VLDR / VSTR (offset-only). Rn=15: literal (pc+4)&!3.
                let off = ((o2 & 0xFF) * 4) as u32;
                let base = rr(cpu, rn, pc);
                let addr = if u == 1 { base.wrapping_add(off) } else { base.wrapping_sub(off) };
                if single {
                    if l == 1 {
                        cpu.regs.s[sd] = mem.read32(addr);
                    } else {
                        mem.write32(addr, cpu.regs.s[sd]);
                    }
                } else {
                    let d = (dbit << 4) | vd4;
                    if d > 15 {
                        return fault(cpu, pc, op1, op2, 4);
                    }
                    if l == 1 {
                        cpu.regs.s[2 * d] = mem.read32(addr);
                        cpu.regs.s[2 * d + 1] = mem.read32(addr.wrapping_add(4));
                    } else {
                        mem.write32(addr, cpu.regs.s[2 * d]);
                        mem.write32(addr.wrapping_add(4), cpu.regs.s[2 * d + 1]);
                    }
                }
                cpu.regs.control |= 4;
                adv(cpu, pc, 4);
                return true;
            }
            if !((p == 0 && u == 1) || (p == 1 && u == 0 && w == 1)) {
                return fault(cpu, pc, op1, op2, 4);
            }
            if rn == 15 {
                return fault(cpu, pc, op1, op2, 4);
            }
            // Work in S slots: D-reg d == S(2d)/S(2d+1).
            let (s0slot, nslots) = if single {
                if imm8 == 0 || sd + imm8 > 32 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                (sd, imm8)
            } else {
                if imm8 == 0 || imm8 & 1 != 0 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                let d = (dbit << 4) | vd4;
                if d + imm8 / 2 > 16 {
                    return fault(cpu, pc, op1, op2, 4);
                }
                (2 * d, imm8)
            };
            let base = rr(cpu, rn, pc);
            let addr = if p == 0 { base } else { base.wrapping_sub(4 * nslots as u32) };
            for i in 0..nslots {
                let a = addr.wrapping_add(4 * i as u32);
                if l == 1 {
                    cpu.regs.s[s0slot + i] = mem.read32(a);
                } else {
                    mem.write32(a, cpu.regs.s[s0slot + i]);
                }
            }
            if w == 1 {
                cpu.regs.r[rn] = if p == 0 {
                    base.wrapping_add(4 * nslots as u32)
                } else {
                    addr
                };
            }
            cpu.regs.control |= 4;
            adv(cpu, pc, 4);
            return true;
        }
        // (c) EE data-processing (B-group misc + 3-reg arith) lands here.
        return fault(cpu, pc, op1, op2, 4);
    } else {
        fault(cpu, pc, op1, op2, 4)
    }
}

#[inline]
fn rm_of(o2: u32) -> u32 {
    o2 & 0xF
}

