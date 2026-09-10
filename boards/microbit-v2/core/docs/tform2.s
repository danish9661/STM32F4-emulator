.syntax unified
.thumb
@ T3 PUW shape reference: privileged offset (!), pre-indexed +WB (!!),
@ post-indexed (,), and unprivileged (T) forms. op2[11:8]: 0xC offset,
@ 0xF pre+WB, 0xB post, 0xE unprivileged-only.
p_off: strb r2, [r4, #4]
p_wb: strb r2, [r4, #4]!
p_post: strb r2, [r4], #4
p_t: strbt r2, [r4, #4]
p_ld: ldr r2, [r5], #4
p_ldt: ldrt r2, [r5, #4]
