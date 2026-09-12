# Ethernet / Networking — board support matrix

Status (2026-09-12): the Ethernet **MAC level is fully modeled** on every
board that has the silicon (F407 family + F429). F401/F411 have no Ethernet
silicon at all. What is "left" is only the physical/pin layer and a few
documented simplifications — see §4. Details live in `AGENTS.md` §29.

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
| TX/RX DMA, descriptor rings, OWN/FS/LS, poll demand, ST/SR | yes | ✅ | `eth_http`/`eth_test` netsim + gateway trio, `verify_ethernet.sh` | — | — |
| TX/RX completion IRQ 61 (TS/RS/NIS/AIS, W1C) | yes | ✅ | `eth_irq_test` (+`_f429`), `eth_dhcp`, `eth_test` | — | — |
| MDIO read/write (MIIAR/MIIDR, MB poll) | yes | ✅ | `eth_feat_test` "PHY …" (`mii_read`/`mii_write`) | wire-level MII timing | Register-level only; firmware only ever sees registers. |
| PHY BCR/BMSR/ANAR/ANLPAR/PHYSTS, AN restart + resolution, forced speed/duplex | external PHY (LAN8720/DP83848-style) | ✅ | `eth_feat_test` PHY phase; 2 native tests | — | Outcome is reported, never forced into MACCR — like silicon, the driver programs FES/DM itself. Harness drives link (`eth_set_link`); BMSR/PHYSTS follow. |
| TX checksum offload (CIC: IP / +TCP-UDP-ICMP) | yes | ✅ | `eth_feat_test` "CSUM TX insert OK" (loopback) | — | Inserted by the driver into guest buffer + capture before onTx. |
| RX checksum status (IPHCE/PCE in RDES0) | yes | ✅ | `eth_feat_test` IPHCE/PCE phases; 1 native test | extended-status RDES4 (needs enhanced descriptors) | Normal-descriptor bits cover what our firmware reads. |
| Perfect filter slots 0–3 (AE, MBC masks, SA/DA) | yes | ✅ | `eth_irq_test` uses slot 1 (`:02` identity); 1 native test | — | Slot 0 always on. **Byte order is MSB-first (IEEE)**: wire `02:00:…:01` = HR `0x0200` / LR `0x00000001`. |
| Hash filter (HU/HM, CRC32, upper 6 bits) | yes | ✅ | `eth_feat_test` "MCAST OK" (member in, non-member out); 1 native test | — | — |
| Broadcast (DBF), all-multicast (PM), promiscuous (PR), DAIF, ROD | yes | ✅ | native tests (PR, DBF); ROD on the loopback path | — | — |
| VLAN tag filter (VLANTI, 12/16-bit, invert) | yes | ✅ | `eth_feat_test` "VLAN OK" (tagged in, untagged dropped); 1 native test | TX tag insertion | F4 silicon has RX filtering only; same here. |
| MAC loopback (LM) | yes | ✅ | all `eth_feat_test` loopback phases | — | Accept filter applies in loopback, like silicon — self-tests must address frames to themselves. |
| PTP timebase (binary 2³¹, TSE, TSSTI/TSSTU) | yes | ✅ | `eth_feat_test` "PTP time OK" | — | — |
| PTP drift correction (addend accumulator, TSFCU latch) | yes | ✅ | `eth_feat_test` "PTP drift OK" (1:2 ratio over identical CYCCNT windows); 1 native test | addend as pure rate control | Real drift needs a drifting clock; ours is exact, so correction = rate scaling. |
| PTP target + interrupt (TSITE → IRQ61) | yes | ✅ | `eth_feat_test` "PTP target OK" | — | — |
| PTP TX/RX snapshots (TDES6/7, RDES6/7) | yes (enhanced descs) | ✅ | `eth_feat_test` both snap phases | RX snapshot needs 32-byte descs (polling E-layout has 8) | Driver-side write; irq path only. |
| PPS output pin | yes (pin) | 🔶 | `eth_pps_count()` scope-probe export + matrix `post` band assert (~39 edges) | the pin itself | No pin layer exists; the counter IS the sink. Guest can't see it, like silicon. |
| Wake-on-LAN magic packet (MPR + IRQ62) | yes | ✅ | `eth_feat_test` "WOL OK"; 1 native test | — | PMT status is read-to-clear (DWC_gmac, not W1C); PWRDWN drops all RX but WOL still sees; GLOBU gates unicast filter eligibility. |
| Wakeup-frame filter CRC match (RWKPR + IRQ62) | yes | ✅ | `eth_feat_test` "WOL filter OK" (+ mismatch, multicast-only, GLOBU, powerdown-drop + magic-during-PD sub-cases); 4 native tests | — | Sourced layout (Synopsys DWC_gmac via the ESP32 EMAC header): masks ptr 0–3 (bits 30:0), commands ptr 4 (bit 3/11/19/27 = multicast-only), offsets ptr 5, CRCs ptr 6–7. |
| TX wire pacing (line-rate TS delay at FES speed) | physical wire | ✅ | `eth_feat_test` "WIRE RATE OK" (1200 B, d within 10k..40k cycles) | — | Delay lands on step boundaries; rate tests use small steps. RX completions are NOT paced (RS fires at delivery — pacing it hides the wire from deferral); the RX busy window feeds deferral only ("RX RATE OK" measures the round trip). |
| Link/carrier (dead wire → NC, TS error completion) | wire + peer | ✅ | `eth_feat_test` "LINK DOWN OK" (BMSR + NC) / "LINK UP OK" | — | Harness is the peer; dead-wire TX still raises TS (error completion, like silicon). |
| Half-duplex deferral (DB while RX occupies the wire) | shared wire | ✅ | `eth_feat_test` "DEFER OK" (pipelined at 10M) + "DEFER DROP OK" (full-duplex negative) | real contention timing | Must pipeline (waiting TX#1's TS consumes the window); runs at 10M for margin, rate proven by WIRE/RX bands. |
| Half-duplex collisions | shared wire + peer | 🔶 | `eth_feat_test` "COLLIDE OK" (EC + CC=15 in half-duplex) + "COLLIDE DROP OK" (armed collision ignored full-duplex) | real contention/backoff timing | Single node, so this models the MAC's error-reporting path (armed via `eth_arm_collision`, one-shot), not backoff contention. |
| Media select (SYSCFG PMC MII/RMII) | pin mux | ✅ | `eth_feat_test` "PHY media RMII/MII OK" (select sticks, traffic flows) | — | Element exists in silicon as a mux bit; data path is mode-agnostic (no pins to mux). |
| ARP (both directions), IPv4, ICMP echo, UDP/DHCP/DNS/echo, TCP (SYN/data/FIN), HTTP, custom ethertype PING/PONG | software (LwIP on silicon) | ✅ | `eth_http`/`eth_dhcp`/`eth_test`/`eth_irq_test` + `_f429`, netsim + real gateway | ICMP/DNS/other-UDP beyond the demo services | gVisor would carry them; no demo firmware speaks them. |
| Real LwIP 2.2.1, NO_SYS=1 raw API (vendored `lwip_demo/lwip/`) | software | ✅ | `lwip_demo`: real DHCP client → DNS → TCP echo → TCP server → UDP echo | — | sys_arch (DWT-ms, rand, libc, printf), netif glue, `lwipopts.h`; needs the peer for DHCP-server/DNS/echo. Porting traps in AGENTS §29. |
| Socket API (BSD shapes) | software (needs OS threads) | ✅ (`lwip_sock.c` over real lwIP) | `lwip_demo`: same markers + "LWIP SELECT OK" + "LWIP ERR OK" | threads/pthreads/select-writefds | Single-threaded pumploop: blocking calls pump netif+timers; `select()` is level-triggered polling with timeout; real `err_t` codes flow through (misuse probes assert them). Threads genuinely need a scheduler — the only honest gap left here. |
| MII/RMII pin levels (TX_EN/CRS_DV/RXD/COL/MDIO/MDC) | pins | ✅ | `eth_pins_test` (+`_f429`): AF setup readback, TX/RX/COL/idle levels | nibble data + clocks | ORed into GPIO IDR via pin callbacks when PMC selects the mode (RMII set, MII COL). Nibble data at 25/50 MHz is unobservable by any firmware (Nyquist) — activity levels + COL events are the complete contract; COL stretches across the collided TX (the real pulse is unsampleable). |
| STOP + WOL wake | power + MAC | ✅ | `eth_feat_test` "WOKE BY WOL" (magic reply queued, STOP via WFI, sleep drain injects, IRQ62 wakes; CYCCNT>50k proves real sleep) | — | WKUP ISR must not ack status on entry (destroys the evidence); thread mode acks after observing. |
| Half-duplex collisions/backoff, MII/RMII pin modes | yes (PHY/wire) | ➖ | — | not modeled | Needs a pin layer (MII/RMII, CRS/COL); no firmware exercises them. Error reporting (EC/CC) and deferral (DB) above are modeled. |

## 3. Per-board verdict

| Board chip | Verdict | Remark |
|---|---|---|
| F401, F411 | nothing to do — no silicon | Any ETH firmware fails honestly; no presets, no matrix entries. |
| F407 (VG/VE/ZE) | complete at MAC level | All rows above ✅/🔶 proven by `eth_*` + `lwip_demo` on the monox map, netsim and real-gateway runs. |
| F429 | complete at MAC level | Same sources + family link/SP (`build_family` fams), own `*_f429.bin`, proven on the Keil map (netsim matrix; gateway trio). |

## 4. What "left" means (deliberate non-models)

1. **PPS pin metal** — no pin layer; `eth_pps_count()` + `eth_pps_level()` + a browser panel dot are the sinks. Closes if a demo ever needs PPS-driven behavior.
2. **MII/RMII wire signaling, half-duplex contention timing** — register-level MDIO + PMC select + carrier/deferral reporting only. Needs a signal-level peer nobody can observe from firmware.
3. **Full socket API (threads/select/mboxes)** — needs an OS layer; NO_SYS raw callbacks cover everything the demos need.
4. **M0+ chips are out of scope** — different core (closed per maintainer decision; see `cpu_bug.md`).

## 5. Verify it

```bash
# Matrix (netsim, includes feat + lwip on F429):
node site/test_board_matrix.mjs eth_feat_test   # 9 phases + PPS scope assert
node site/test_board_matrix.mjs lwip_demo
# Real gateway (needs openhw-gw on :5070 + HTTP server on :8092):
bash scripts/verify_ethernet.sh                 # both trios, 0 TCP fail
# Unit level:
cargo test --release peripherals::eth           # 13 tests (filter/CRC/PHY/PTP/PPS/WOL)
```
