# Instruction-encoding probes (ground truth for the CPU decoder)

Each `.s` file here assembles with the exact F4 toolchain flags and the
`objdump -d` output pins the halfwords the decoder must match. Re-verify
with:

```bash
TC="$HOME/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-"
${TC}as -march=armv7e-m -mfloat-abi=hard -mfpu=fpv4-sp-d16 -o /tmp/x.o <file>.s \
  && ${TC}objdump -d /tmp/x.o
```

(DSP-only probes omit the float flags; the FPU probes need them.)

## FPU (`cpu/thumb.rs` FPU dispatch, AGENTS.md §25)

| File | What it pins |
|---|---|
| `fpu.s` | Base table: vmov-imm/reg/core/2-reg, vmrs/vmsr, add/sub/mul/div/mla/mls/nmul/nmla/nmls, sqrt, cmp(+#0), abs/neg, vcvt int/fixed/f16, vldr/vstr, vldm/vstm/vpush/vpop |
| `fpu2.s`–`fpu6.s` | Register-numbering: Sd=(Vd<<1)\|D etc. with high regs (s16–s22), D-lists, vcmp#0 on s20 |
| `fpu7.s` | VLDM-S with D=1 (`{s16-s19}`), VCVT frac map (#1/#2/#8/#31/#32 → op2lo), VMOV-imm values (#2.0/#-0.5/#6.75), vpush/vpop D-lists. Also records two GAS rejections: vcvt `#0` (range is 1–32) and vldr/vstr writeback (offset-only) |
| `fpu8.s` | vldr/vstr double (d1), vmov s20/r4, vldmdb with `!` |
| `fpu9.s` | REJECTION probe: `vldmdb` without `!` (DB requires writeback — decoder faults it) |
| `fpu10.s` | REJECTION probe: fixed-point VCVT requires Sd==Sm (decoder reads sd) |
| `fpu11.s` | Fused VFMA/VFMS/VFNMA/VFNMS encodings (opc1 0xA/0x9) |
| `fpu12.s` | VCMPE reg + #0 (opb3=6 E-form; raises IOC on any NaN) |
| `fpu13.s` | D=1 forms: vadd s17/s18/s19, vfma s21/s22/s23, vsqrt/vcmp/vmov/vcvt s17/s18, vsub s31/s30/s29 |
| `fpu14.s` | M=0 forms: vcvt.f32.s32/s4/s5, vmov/vcvtb/vcmp/vneg/vabs s0/s0, vcvt.f32.u32 s4/s5 |
| `fpu15.s` | `vmov.f32 s0, #-0.5` (EEBE 0A00) — the D=0 counterpart of fpu7's s5 form |
| `fpu16.s` | VMRS ID regs: mvfr0=EEF7, mvfr1=EEF6, mvfr2=EEF5 (via neon-fp-armv8 — fpv4-sp GAS rejects the mnemonic), fpexc=EEF8; vmsr fpexc=EEE8 |
| `regmatrix.s` | Nonzero-Rd FB/FA forms: smmla/smmls/smmul/usada8/usad8/smlad/smulwb-t/smlawb-t/smlal/qadd8/shadd16 (the Rd-gate audit) |

Key derivations are written up in AGENTS.md §25 (encoding rules + gotchas).

## DSP (`FB`/`FA`/`F3`/`E8` arms, pre-FPU work)

| File | What it pins |
|---|---|
| `batch.s`, `var.s` | SMLALD/SMLSLD/UMAAL/SMMUL/USAD8 family encodings |
| `ex.s`, `ex2.s`–`ex4.s` | LDREX/STREX size/offset forms (incl. the offset-word nibble edge case) |
| `sh.s` | Shift-register flag setting, SXTAB16/UXTAB16 shadowing |
| `tform.s`, `tform2.s` | LDRT/STRT family: op2[11:8]==0xE marks as-unprivileged (privileged PUW uses 0xB/0xC/0xF); the decoder's MPU privilege override keys off exactly this nibble |
