.syntax unified
.thumb
.long1: smlald r0, r1, r2, r3
.long2: smlsld r4, r5, r6, r7
.long3: umaal r8, r9, r10, r11
.mul1: smmul r0, r1, r2
.mul2: smmulr r0, r1, r2
.mul3: smmla r0, r1, r2, r3
.mul4: smmlar r0, r1, r2, r3
.mul5: smmls r0, r1, r2, r3
.mul6: smmlsr r0, r1, r2, r3
.usad1: usad8 r0, r1, r2
.usad2: usada8 r0, r1, r2, r3
.p8a: qadd8 r0, r1, r2
.p8b: qsub8 r0, r1, r2
.p16a: qadd16 r0, r1, r2
.p16b: qsub16 r0, r1, r2
.p8c: uqadd8 r0, r1, r2
.p8d: uqsub8 r0, r1, r2
.p16c: uqadd16 r0, r1, r2
.p16d: uqsub16 r0, r1, r2
.ph8a: shadd8 r0, r1, r2
.ph8b: shsub8 r0, r1, r2
.ph16a: shadd16 r0, r1, r2
.ph16b: shsub16 r0, r1, r2
.ph8c: uhadd8 r0, r1, r2
.ph8d: uhsub8 r0, r1, r2
.ph16c: uhadd16 r0, r1, r2
.ph16d: uhsub16 r0, r1, r2
.px1: qasx r0, r1, r2
.px2: qsax r0, r1, r2
.px3: uqasx r0, r1, r2
.px4: uqsax r0, r1, r2
.px5: shasx r0, r1, r2
.px6: shsax r0, r1, r2
.px7: uhasx r0, r1, r2
.px8: uhsax r0, r1, r2
.pe16a: sxtab16 r0, r1, r2
.pe16b: uxtab16 r0, r1, r2
.pe16c: sxtab16 r0, r1, r2, ror #8
.ex1: ldrexb r0, [r1]
.ex2: ldrexh r0, [r1]
.ex3: strexb r0, r1, [r2]
.ex4: strexh r0, r1, [r2]
