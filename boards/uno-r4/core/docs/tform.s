.syntax unified
.thumb
@ Unprivileged T3 load/store forms (LDRT/STRT family): op2[11:8]==0xE marks
@ the as-unprivileged access. Privileged PUW uses 0xB (post-indexed),
@ 0xC (offset), 0xF (pre-indexed +WB) — never 0xE. The decoder keys the
@ MPU privilege override off exactly this nibble (see tform2.s); address
@ and direction decode identically to the privileged forms.
t_bt: strbt r2, [r4, #4]
t_bt2: ldrbt r2, [r5, #4]
t_ht: strht r2, [r4, #4]
t_ht2: ldrht r2, [r5, #4]
t_t: strt r2, [r4, #4]
t_t2: ldrt r2, [r5, #4]
t_sbt: ldrsbt r2, [r5, #4]
t_sht: ldrsht r2, [r5, #4]
