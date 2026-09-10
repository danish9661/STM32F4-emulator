.syntax unified
.thumb
vadd.f32 s16, s0, s0
vadd.f32 s0, s16, s0
vadd.f32 s0, s0, s16
vmov.f32 s16, s17
vmov r0, s16
vadd.f32 s1, s2, s3
