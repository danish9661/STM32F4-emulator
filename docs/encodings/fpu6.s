.syntax unified
.thumb
vadd.f32 s20, s21, s22
vmov.f32 s20, s21
vcvt.s32.f32 s20, s21
vcmp.f32 s20, s21
vmov r4, s20
vmov s20, r4
vldr s4, [r5]
vcvt.f32.u32 s0, s0, #16
vcvt.u32.f32 s0, s0, #16
vcvt.f32.s32 s0, s0, #1
vldr s0, [pc, #8]
