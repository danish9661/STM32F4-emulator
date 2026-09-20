---
sidebar_position: 3
title: Networking
description: Ethernet on the emulated F407/F429 — MAC/PHY, filters, PTP, WOL, LwIP sockets, netsim and gateway.
---

# Ethernet / Networking — board support matrix

Status (2026-09-19): the Ethernet **MAC level is fully modeled** on every
board that has the silicon (F407 family + F429), including the register
positions cross-checked against CMSIS `stm32f407xx.h` (DMASR/DMAIER
AIS=15/ERS=14/FBES=13/RWTS=9, MACFFR SAF=9/SAIF=8/HPF=10 — all verified,
see §6) plus the descriptor-chain layer added this session (DMABMR EDFE,
TCH/TER + RCH/RER walks, RDES4 extended status, backoff slot probe — mock
`t_gap11`, 336 checks). F401/F411 have no Ethernet silicon at all. The
remaining rows in §2 are honest about what is simulation-complete vs
deliberately absent — see §4. Details live in `AGENTS.md` §29. Re-verified
this session: mock-model 18/18 + mock-consumer 44/44 green (see §5).

## 1. Board silicon summary

| Board builds in this repo | Chip | Ethernet MAC | On-chip PHY | Speeds | Remark |
|---|---|---|---|---|---|
| BlackPill F401CC, Nucleo F401RE | STM32F401 | none | — | — | No silicon; no firmware can use ETH (matrix has no ETH entries for these). |
| BlackPill F411CE, Nucleo F411RE | STM32F411 | none | — | — | Same as F401: no silicon, nothing to model. |
| Disco F407VG, Black F407VE/ZE, generic F407 | STM32F407Vx/Zx | 10/100, MII + RMII | no (external, e.g. LAN8720/DP83848) | 10/100, half/full | Full MAC feature set (RM0090 §33): DMA, checksum offload, PTP, VLAN, WOL, hash/perfect filtering, loopback. |
| Disco F429ZI | STM32F429ZI | 10/100, MII + RMII (same MAC as F407) | no (external) | 10/100, half/full | Same MAC block as F407; verified with its own builds (`*_f429.bin`) on the Keil SVD map. |

Note: on real silicon the PHY is always an external chip. The emulator
likewise models the PHY as a *peer* (register-level MDIO), not wires.

## 2. Feature matrix (F407 / F429 — the two chips with silicon)

Silicon column = what the real MAC does. Status: ✅ modeled + tested,
🔶 modeled with a documented simplification, ➖ not modeled (see remark).

| Feature | Silicon (F407 = F429) | Status | Proof (test) | Left / gap | Remark |
|---|---|---|---|---|---|
| TX/RX DMA, descriptor rings, OWN/FS/LS, poll demand, ST/SR | yes | ✅ | `eth_http`/`eth_test` netsim + gateway trio, `verify_ethernet.sh` | — | Single-buffer delivery: FS+LS always set together (HAL `ETH_DMARXDESC_FS/LS`); TCH/TER + RCH/RER chain stepping live per descriptor (ring wrap returns to list base, chained follows Desc3 — mock `t_gap11`); single-descriptor guests stay put (no walk without TCH/TER/RCH/RER — advancing past the only descriptor diverts the next poll wild); RDES4 extended status written when the descriptor is wide enough (stride ≥ 20, both layouts); RDES6/7 snapshot gated on stride ≥ 32 AND EDFE-clear compat (normal layout, like all in-tree firmware — EDFE-set would clobber the probe words). |
| TX/RX completion IRQ 61 (TS/RS/NIS/AIS, W1C) | yes | ✅ | `eth_irq_test` (+`_f429`), `eth_dhcp`, `eth_test` | — | Bit positions per CMSIS (AIS=15, ERS=14, FBES=13, RWTS=9); summaries recomputed inline on W1C clear. |
| MDIO read/write (MIIAR/MIIDR, MB poll) | yes | ✅ | `eth_feat_test` "PHY …" (`mii_read`/`mii_write`) | wire-level MII timing | Register-level only; firmware only ever sees registers. |
| PHY BCR/BMSR/ANAR/ANLPAR/PHYSTS, AN restart + resolution, forced speed/duplex | external PHY (LAN8720/DP83848-style) | ✅ | `eth_feat_test` PHY phase; 2 native tests | — | Outcome is reported, never forced into MACCR — like silicon, the driver programs FES/DM itself. Harness drives link (`eth_set_link`); BMSR/PHYSTS follow. |
| TX checksum offload (CIC: IP / +TCP-UDP-ICMP) | yes | ✅ | `eth_feat_test` "CSUM TX insert OK" (loopback) | — | Inserted by the driver into guest buffer + capture before onTx. |
| RX checksum status (IPHCE/PCE in RDES0) | yes | ✅ | `eth_feat_test` IPHCE/PCE phases; 1 native test | — | Normal-descriptor bits cover what our firmware reads; RDES4 extended status (IPV4PR/IPHE/IPPE/IPPT/PTPMT per HAL `ETH_DMAPTPRXDESC_*`) written alongside on wide descriptors — mock `t_gap11`. |
| Perfect filter slots 0–3 (AE, MBC masks, SA/DA) | yes | ✅ | `eth_irq_test` uses slot 1 (`:02` identity); 1 native test | — | Slot 0 always on. **Byte order is MSB-first (IEEE)**: wire `02:00:…:01` = HR `0x0200` / LR `0x00000001`. |
| Hash filter (HU/HM, CRC32, upper 6 bits) | yes | ✅ | `eth_feat_test` "MCAST OK" (member in, non-member out) + "MCAST PM OK" (PM passes with empty table); 2 native tests | — | PM (bit 0) passes all multicast incl. broadcast; BFD (bit 5) re-gates broadcast ("MCAST BFD OK"). |
| Broadcast (DBF), all-multicast (PM), promiscuous (PR), DAIF, ROD | yes | ✅ | native tests (PR, DBF, PM+BFD); ROD on the loopback path | — | MACFFR bit positions per CMSIS (SAF=9/SAIF=8/HPF=10, PCF=7:6). |
| Source-address filter + inverse (SAF/SAIF) | yes | ✅ | `eth_feat_test` "SAF SELF/DROP OK" + "SAIF INV OK"; 1 native test | — | CMSIS bits 9/8 (were off-by-one at 8/7 — every SAF/SAIF verdict passed vacuously until fixed). |
| Hash-or-perfect (HPF) | yes | ✅ | `eth_feat_test` "HPF STRICT/OR OK"; 1 native test | — | CMSIS bit 10 (was bit 9 — STRICT passed vacuously). |
| Control-frame pass (PCF) | yes | ✅ | `eth_feat_test` "PAUSE PCF00/PCF10 OK" + "PCF11 OK" (DA path) + "PCF11 PAM OK"; 1 native test | — | PCF=11 takes the normal DA path (pause DA needs PAM) — previously unasserted. |
| VLAN tag filter (VLANTI, 12/16-bit, invert) | yes | ✅ | `eth_feat_test` "VLAN OK" (tagged in, untagged dropped) + "VLAN INV OK" (12-bit inverted: tagged-7 drops, untagged passes); 1 native test | TX tag insertion: no silicon path — the F4 MAC (RM0090 §33, MACVLANTR is RX-only; CMSIS `stm32f407xx.h` has no TX-insert field, and the HAL's `ETH_TX_PACKETS_FEATURES_VLANTAG` sets only the VF status bit, never bytes) has no tag-insertion engine. Firmware inserts tags itself (standard practice: Singamsetty-style raw TX with the 4-byte 802.1Q header pre-built); the RX gate then filters them like silicon. | F4 silicon has RX filtering only; same here. Untagged under a NORMAL gate drops (no match); under an INVERTED gate it passes (absent tag never matches; invert turns that into accept). |
| MAC loopback (LM) | yes | ✅ | all `eth_feat_test` loopback phases | — | Accept filter applies in loopback, like silicon — self-tests must address frames to themselves. |
| PTP timebase (binary 2³¹, TSE, TSSTI/TSSTU) | yes | ✅ | `eth_feat_test` "PTP time OK" | — | — |
| PTP drift correction (addend accumulator, TSFCU latch) | yes | ✅ | `eth_feat_test` "PTP drift OK" (1:2 ratio over identical CYCCNT windows); 1 native test | addend as pure rate control | Real drift needs a drifting clock; ours is exact, so correction = rate scaling. |
| PTP target + interrupt (TSITE → IRQ61) | yes | ✅ | `eth_feat_test` "PTP target OK" | — | — |
| PTP TX/RX snapshots (TDES6/7, RDES6/7) | yes (enhanced descs) | ✅ | `eth_feat_test` "PTP snap gate OK" (UDP+TTSE does NOT stamp) + both snap phases on a raw 0x88F7 Sync frame | RX snapshot needs 32-byte descs (polling E-layout has 8) | Driver-side write, event messages only (0x88F7 or UDP :319/:320); irq path only. |
| PPS output pin | yes (pin) | ✅ | PB5 IDR mirror (guest-visible, TSE-gated, same advance-then-read path as the counter — mock `t_pps`) + `eth_pps_count()`/`eth_pps_level()` scope probes + matrix `post` band assert (~39 edges) + browser `#ppsDot`/`#ppsInfo` panel | package-wire metal only | The mirror IS the pin model (guest polls PB5 IDR); counter/level are scope sinks for the harness. No demo needs more. |
| Wake-on-LAN magic packet (MPR + IRQ62) | yes | ✅ | `eth_feat_test` "WOL OK"; 1 native test | — | PMT status is read-to-clear (DWC_gmac, not W1C); PWRDWN drops all RX but WOL still sees; GLOBU gates unicast filter eligibility. PMTCTL mask keeps PD/MPE/WFE/MPR/WFR/GU/WFFRPR only (CMSIS — old mask stored sham bits 7+10). |
| Wakeup-frame filter CRC match (RWKPR + IRQ62) | yes | ✅ | `eth_feat_test` "WOL filter OK" (+ mismatch, multicast-only, GLOBU, powerdown-drop + magic-during-PD sub-cases); 4 native tests | — | Sourced layout (Synopsys DWC_gmac via the ESP32 EMAC header): masks ptr 0–3 (bits 30:0), commands ptr 4 (bit 3/11/19/27 = multicast-only), offsets ptr 5, CRCs ptr 6–7. |
| TX wire pacing (line-rate TS delay at FES speed) | physical wire | ✅ | `eth_feat_test` "WIRE RATE OK" (1200 B, d within 10k..40k cycles) | — | Delay lands on step boundaries; rate tests use small steps. RX completions are NOT paced (RS fires at delivery — pacing it hides the wire from deferral); the RX busy window feeds deferral only ("RX RATE OK" measures the round trip). |
| Link/carrier (dead wire → NC, TS error completion) | wire + peer | ✅ | `eth_feat_test` "LINK DOWN OK" (BMSR + NC) / "LINK UP OK" | — | Harness is the peer; dead-wire TX still raises TS (error completion, like silicon). |
| Half-duplex deferral (DB while RX occupies the wire) | shared wire | ✅ | `eth_feat_test` "DEFER OK" (pipelined at 10M) + "DEFER DROP OK" (full-duplex negative) | real contention timing | Must pipeline (waiting TX#1's TS consumes the window); runs at 10M for margin, rate proven by WIRE/RX bands. |
| Half-duplex collisions | shared wire + peer | ✅ | `eth_feat_test` "COLLIDE OK" (EC + CC=15 in half-duplex) + "COLLIDE DROP OK" (armed collision ignored full-duplex) + deterministic backoff slot probe `eth_backoff_slots` (truncated binary exponential under a harness seed — mock `t_gap11`) | — | Single node, so the MODEL is the MAC's error-reporting path (armed via `eth_arm_collision`, one-shot) plus the IEEE 802.3 slot-count math; there is no contending peer, so the wait itself is vacuous (the wire is always free after the paced frame time). No firmware can observe more: the CC/EC/DB status words + COL stretch are the complete silicon-observable footprint. |
| Media select (SYSCFG PMC MII/RMII) | pin mux | ✅ | `eth_feat_test` "PHY media RMII/MII OK" (select sticks, traffic flows) | — | Element exists in silicon as a mux bit; data path is mode-agnostic (no pins to mux). |
| ARP (both directions), IPv4, ICMP echo, UDP/DHCP/DNS/echo, TCP (SYN/data/FIN), HTTP, custom ethertype PING/PONG | software (LwIP on silicon) | ✅ | `eth_http`/`eth_dhcp`/`eth_test`/`eth_irq_test` + `_f429`, netsim + real gateway | ICMP/DNS/other-UDP beyond the demo services | gVisor would carry them; no demo firmware speaks them. |
| Real LwIP 2.2.1, NO_SYS=1 raw API (vendored `lwip_demo/lwip/`) | software | ✅ | `lwip_demo`: real DHCP client → DNS → TCP echo → TCP server → UDP echo | — | sys_arch (DWT-ms, rand, libc, printf), netif glue, `lwipopts.h`; needs the peer for DHCP-server/DNS/echo. Porting traps in AGENTS §29. |
| Socket API (BSD shapes) | software (needs OS threads) | ✅ (`lwip_sock.c` over real lwIP) | `lwip_demo`: same markers + "LWIP SELECT OK" + "LWIP ERR OK" | pthreads/mboxes/select-writefds | Single-threaded pumploop: blocking calls pump netif+timers; `select()` is level-triggered polling with timeout; real `err_t` codes flow through (misuse probes assert them). pthreads/mboxes need an RTOS scheduler — there is no RTOS in the rig (all shipped firmware is bare-metal; a FreeRTOS port exists only as the `freertos_test` context-switch probe, not as a socket host). select-writefds is unneeded: TX never blocks (frames stage into DMA descriptors, completion via TS/OWN-clear). |
| MII/RMII pin levels (TX_EN/CRS_DV/RXD/COL/MDIO/MDC) | pins | ✅ | `eth_pins_test` (+`_f429`): AF setup readback, TX/RX/COL/idle levels | nibble data + clocks | ORed into GPIO IDR via pin callbacks when PMC selects the mode (RMII set, MII COL). Nibble data at 25/50 MHz is unobservable by any firmware (Nyquist) — activity levels + COL events are the complete contract; COL stretches across the collided TX (the real pulse is unsampleable). |
| L3/L4 + link-scope protocols (RST/RTO/MSS/window/frag/ICMP-err/DHCP-NAK/renew/IGMP/ND/LLDP/STP) | software/peer | ✅ | `eth_adv` (+`_f429`): 12 phases, one observable each, dedicated netsim peers (trigger ports 5010–5018); matrix 6000×20000 | — | Netsim-only suite (no gateway path — the gateway's gVisor stack owns those answers itself). Guest ISR consume-on-entry + OWN-fallback; `rxDesc` must be the nm-verified `rx_desc`. Details in AGENTS.md §39. |
| STOP + WOL wake | power + MAC | ✅ | `eth_feat_test` "WOKE BY WOL" (magic reply queued, STOP via WFI, sleep drain injects, IRQ62 wakes; CYCCNT>50k proves real sleep) | — | WKUP ISR must not ack status on entry (destroys the evidence); thread mode acks after observing. |
| Half-duplex collisions/backoff, MII/RMII pin modes | yes (PHY/wire) | ➖ | — | signal-level contention with a live peer | Deliberately absent: needs a second live transmitter + a pin layer (MII/RMII, CRS/COL at 25/50 MHz); no firmware exercises them, and by Nyquist no firmware could sample the wire anyway. Error reporting (EC/CC) and deferral (DB) above ARE modeled, as are the pin *levels* (TX_EN/CRS_DV/RXD/COL/MDIO/MDC in GPIO IDR) — see the row above. Only an external logic-analyzer peer could observe more — none exists in the rig. |

## 3. Per-board verdict

| Board chip | Verdict | Remark |
|---|---|---|
| F401, F411 | nothing to do — no silicon | Any ETH firmware fails honestly; no presets, no matrix entries. |
| F407 (VG/VE/ZE) | complete at MAC level | All rows above ✅/🔶 proven by `eth_*` + `lwip_demo` on the F407 vendor map, netsim and real-gateway runs. |
| F429 | complete at MAC level | Same sources + family link/SP (`build_family` fams), own `*_f429.bin`, proven on the Keil map (netsim matrix; gateway trio). |

## 4. What "left" means (deliberate non-models)

1. **PPS package-wire metal** — the complete firmware-observable path is
   modeled: PB5 IDR mirror (guest polls it like a pin) + `eth_pps_count()` /
   `eth_pps_level()` scope probes + the browser `#ppsDot`/`#ppsInfo` panel.
   Only the copper trace itself is absent — no demo can need more.
2. **Signal-level contention with a live peer** — register-level MDIO + PMC
   select + carrier/deferral reporting + GPIO IDR level mirrors
   (TX_EN/CRS_DV/RXD/COL/MDIO/MDC) + deterministic backoff slot math are all
   live. What is absent is a second live transmitter at 25/50 MHz — and by
   Nyquist no firmware could sample one anyway (mock `t_nibble` + `t_gap11`
   pin the level/slot contract; only an external logic-analyzer peer could
   observe more, and none exists).
3. **Socket pthreads/mboxes** — the BSD shapes over real lwIP
   (`lwip_demo/lwip_sock.c`: socket/bind/listen/accept/connect/send/recv/
   sendto/recvfrom/close + level-triggered `select()` with timeout + real
   `err_t` codes, proven by "LWIP SELECT OK" / "LWIP ERR OK") cover
   everything the demos need. Genuinely absent: pthreads/mboxes/select
   writefds — they need an RTOS scheduler, and there is no RTOS in the rig
   (all shipped firmware is bare-metal; TX never blocks, so writefds are
   unneeded by construction).
4. **M0+ chips are out of scope** — different core (closed per maintainer decision; see `cpu_bug.md`).
5. **Instruction-count (not wall-clock) timing** — the virtual clock counts
   executed instructions (168 MHz nominal), so `delay_ms(100)` is ~2.4M
   emulated instructions and real-time rates don't hold. This is by design
   (deterministic across machines — see progress-and-future.md Known
   limitations #4), not a gap: every rate in §2 (WIRE/RX bands, deferral
   windows, PPS edges, PTP drift ratios) is asserted in virtual-instruction
   bands, and the browser realtime lock paces the guest to wall time.

## 5. Verify it

```bash
# Mock harnesses (both sides of the driver/model seam, no firmware build):
npm run test:eth:mock   # mock-model (18 checks, fake bindings + real driver)
                        # + mock-consumer (44 checks, fake firmware + real model)
                        # + periph mock-consumer (262 checks incl. t_pps/t_nibble ETH pins)
# Matrix (netsim, includes feat + lwip on F429):
node site/test_board_matrix.mjs eth_feat_test   # 66 markers + PPS scope assert
node site/test_board_matrix.mjs lwip_demo
# Real gateway (needs openhw-gw on :5070 + HTTP server on :8092):
bash scripts/verify_ethernet.sh                 # both trios, 0 TCP fail
# Unit level:
cargo test --release peripherals::eth           # filter/CRC/PHY/PTP/PPS/WOL/MACFFR/PMTCTL
```

Mock coverage map (each CMSIS-audit gap pinned on BOTH sides):

| Gap | Mock-model (fake bindings + real driver) | Mock-consumer (fake firmware + real model) |
|---|---|---|
| 1. DMASR/DMAIER positions | NC/JT error completion words + jabber note | summaries clear, DMAIER keeps RWTIE/ERIE/FBEIE/AISE |
| 2. MACFFR SAF/SAIF/HPF | TX capture FFR-agnostic | SAF/SAIF/HPF accept + old-bit negatives |
| 3. PMTCTL mask + RWUFFR arm | WOL inspected before accept gate (RE clear) | RWUFFR arm present, sham bits dropped, controls kept |
| 4. RX FS/LS | FS+LS on both irq + polling writebacks | HAL constants + no DMASR alias |
| 5. PM/BFD/PCF-11 | accept routing (OWN clear on deliver) + dead-RX drop (TE set, RE clear: WOL first, no RS write) | PM/BFD/PCF-11 accept decisions + VLAN 12-bit/invert tagged paths |
| 6. PTP event gate | TTSS/TDES6-7 only on 0x88F7, RDES6 stride gate | csum status bits, WOL latch, pacing/collision/deferral/link/IPCO |

## 6. Register audit (2026-09-14): CMSIS cross-check + what it caught

All 56 SVD registers (20 MAC + 11 MMC + 11 PTP + 14 DMA) were checked
field-by-field against CMSIS `stm32f407xx.h` (`ETH_DMASR_*_Pos`,
`ETH_DMAIER_*_Pos`, `ETH_MACFFR_*_Pos`, `ETH_MACPMTCSR_*`,
`ETH_MACVLANTR_*`, `ETH_DMATXDESC_*`). Six real gaps, all fixed +
covered by firmware markers and/or native tests:

1. **DMASR/DMAIER bit positions** — AIS was 14 (silicon 15), ERS 12
   (silicon 14), FBE 11 (silicon 13); DMAIER had no RWTIE and wrong
   AISE/ERIE/FBEIE positions. "Pause-time status" bit 9 does not exist
   on silicon (it is RWTS, receive-watchdog); the TX-stall hold time is
   ORed live into DMASR reads from the pause atomic, never latched.
2. **MACFFR SAF/SAIF/HPF off-by-one** — SAF was bit 8 (silicon 9), SAIF
   bit 7 (silicon 8), HPF bit 9 (silicon 10). Every SAF/SAIF/HPF verdict
   passed vacuously (wrong bit read back its own write); the firmware
   now programs CMSIS positions.
3. **MACPMTCTL sham mask** — mask `0x687` stored bits 7+10 as if
   meaningful; CMSIS has no fields there (PD=0/MPE=1/WFE=2/MPR=5/WFR=6/
   GU=9/WFFRPR=31). Mask is now `0x207`.
4. **RX FS/LS never set** — delivered frames carried no First/Last
   Segment bits, so any HAL-based RX path saw a "middle fragment"
   forever. Both RX paths now set FS+LS (bit 9+8) on every delivered
   frame (single-buffer delivery).
5. **PM/BFD/PCF-11 unasserted** — stored but never proven. New markers:
   "MCAST PM/BFD OK", "PAUSE PCF11/PCF11 PAM OK" (+ native tests).
   Found live along the way: PM+BFD ordering (BFD re-gates broadcast
   after the PM pass) and the PCF=11 DA-path rule. Follow-up pins on both
   sides of the seam: mock-model gap5b (dead-receiver drop — TE set, RE
   clear: WOL inspection runs first, poll cleared, no RS write — the exact
   emulator.js branch the audit had flagged as comment-only) and
   mock-consumer gap5b (VLAN 12-bit/invert tagged paths, netsim can't emit
   tagged frames on demand).
6. **PTP snapshot on any frame** — TTSE stamped even UDP data. Silicon
   snapshots event messages only (0x88F7 or UDP :319/:320). New
   "PTP snap gate OK" negative + raw-Sync positive.

Two deeper model bugs surfaced by the same runs (fixed, with tests):
a duplicated `0x2C` PMTCTL write arm that shadowed the MACRWUFFR `0x28`
write arm (all wakeup-filter writes vanished — the WOL native test
failed), and a nested `RefCell` borrow panic in `eth_check_wol` (NVIC
borrow inside the MAC-slot borrow — deferred via a `pmt_irq62` latch
drained after the scan). Probe hygiene that kept biting: stale queued
frames satisfying silence checks (drain before negative probes) and
CPU-owned heads starving the next probe (re-arm after consuming).
