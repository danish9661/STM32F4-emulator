.syntax unified
.thumb
vadd.f32 s4, s5, s6
vadd.f32 s7, s8, s9
vmov.f32 s4, s5
vmov r4, s5
vmov s4, r5
vmov r4, r5, d6
vmov d6, r4, r5
vmrs r4, fpscr
vmsr fpscr, r4
vldr s4, [r5, #8]
vstr s4, [r5, #8]
vldmia r4, {s4-s7}
vstmia r4, {d2-d3}
vcvt.s32.f32 s4, s5
vcmp.f32 s4, s5
vcmp.f32 s20, #0
vsqrt.f32 s20, s21
vsmladummy:
