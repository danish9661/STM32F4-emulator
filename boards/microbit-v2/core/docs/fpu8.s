.syntax unified
.thumb
vldr s5, [r0]
vldr d1, [r0]
vstr d1, [r0]
vmov s20, r4
vmov r4, s20
vldmdb r4!, {s0-s3}
