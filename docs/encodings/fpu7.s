.syntax unified
.thumb
vldmia r4, {s16-s19}
vldmia r4, {s0-s1}
vstmia r4!, {s16-s19}
vcvt.f32.s32 s0, s0, #1
vcvt.f32.s32 s0, s0, #2
vcvt.f32.s32 s0, s0, #8
vcvt.f32.s32 s0, s0, #31
vcvt.f32.s32 s0, s0, #32
vcvt.s32.f32 s0, s0, #2
vmov.f32 s0, #2.0
vmov.f32 s5, #-0.5
vmov.f32 s0, #6.75
vpush {d0-d1}
vpop {d2-d3}
vstr s0, [r1, #-8]
