.syntax unified
.thumb
m1: vmov.f32 s0, #1.0
m2: vmov.f32 s0, s1
m3: vmov r0, s0
m4: vmov s0, r0
m5: vmov r0, r1, d0
m6: vmov d0, r0, r1
f1: vmrs apsr_nzcv, fpscr
f2: vmrs r0, fpscr
f3: vmsr fpscr, r0
a1: vadd.f32 s0, s1, s2
a2: vsub.f32 s0, s1, s2
a3: vmul.f32 s0, s1, s2
a4: vdiv.f32 s0, s1, s2
a5: vmla.f32 s0, s1, s2
a6: vmls.f32 s0, s1, s2
a7: vnmul.f32 s0, s1, s2
a8: vnmla.f32 s0, s1, s2
a9: vnmls.f32 s0, s1, s2
s1: vsqrt.f32 s0, s1
s2: vcmp.f32 s0, s1
s3: vcmp.f32 s0, #0
s4: vabs.f32 s0, s1
s5: vneg.f32 s0, s1
c1: vcvt.f32.s32 s0, s0
c2: vcvt.s32.f32 s0, s0
c3: vcvt.f32.u32 s0, s0
c4: vcvt.u32.f32 s0, s0
c5: vcvt.f32.s32 s0, s0, #16
c6: vcvt.s32.f32 s0, s0, #16
c7: vcvtb.f32.f16 s0, s1
c8: vcvtt.f32.f16 s0, s1
c9: vcvtb.f16.f32 s0, s1
c10: vcvtt.f16.f32 s0, s1
l1: vldr s0, [r1]
l2: vldr s0, [r1, #4]
l3: vldr s0, [r1, #-4]
l4: vstr s0, [r1]
v1: vldmia r0, {s0-s3}
v2: vstmia r0, {s0-s3}
v3: vldmdb r0!, {s0-s3}
v4: vstmdb r0!, {s0-s3}
v5: vldmia r0, {d0-d1}
v6: vstmia r0, {d0-d1}
v7: vpush {s0-s3}
v8: vpop {s0-s3}
