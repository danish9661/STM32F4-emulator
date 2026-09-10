.syntax unified
.thumb
vadd.f32 s1, s0, s0
vadd.f32 s0, s1, s0
vadd.f32 s0, s0, s1
vadd.f32 s2, s0, s0
vadd.f32 s0, s2, s0
vadd.f32 s0, s0, s2
