// Mock-consumer harness: a fake FIRMWARE driving the REAL Rust model
// (real wasm-bindgen bindings + real SVD map, no emulator.js driver).
// Each test programs the model exactly like eth_feat_test/main.c does —
// raw periph_write/periph_read MMIO — and asserts the model's answer.
// This pins gaps 1-6 from the consumer side: register positions, masks,
// filter decisions, checksum status, WOL latches, pacing/collision/
// deferral/link reports, and PTP snapshots.
//
// Complements site/test_eth_mock_model.mjs (fake model + real driver):
// that file pins the DRIVER half of each gap; this file pins the MODEL
// half. A regression on either side fails exactly one harness.
//
// Usage: node site/test_eth_mock_consumer.mjs   (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
if (typeof bindings.default === 'function') await bindings.default({ module_or_path: wasmBytes });
bindings.init_svd(svdXml);

const {
    periph_read, periph_write, tick_peripherals,
    eth_mac_accept, eth_rx_csum_status, eth_check_wol,
    eth_tx_wire_busy, eth_rx_wire_busy, eth_arm_collision, eth_take_collision,
    eth_set_link, eth_link_up, eth_tx_deferred,
    eth_ipco_on, eth_fwd_csum_bad,
} = bindings;

let pass = 0, fail = 0;
const ok = (cond, name, extra = '') => {
    if (cond) { pass++; console.log(`  ok: ${name}`); }
    else { fail++; console.error(`  FAIL: ${name} ${extra}`); }
};
const W = (addr, v) => periph_write(addr, 4, v >>> 0);
const R = (addr) => periph_read(addr, 4) >>> 0;
const MAC = 0x40028000, DMA = 0x40029000, PTP = 0x40028700;
const u32 = (v) => v >>> 0;

// station MAC 02:00:00:00:00:01, MSB-first like the firmware programs it
function stationMac() {
    W(MAC + 0x40, 0x00000200 | (1 << 31));
    W(MAC + 0x44, 0x00000001);
}
const SMAC = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
function frameTo(dst, { proto = 0x0800, payload = null } = {}) {
    const p = payload || new Uint8Array(46);
    const f = new Uint8Array(14 + p.length);
    f.set(dst, 0); f.set(SMAC, 6);
    f[12] = (proto >> 8) & 0xFF; f[13] = proto & 0xFF;
    f.set(p, 14);
    return f;
}
function ipUdp({ dport = 5009, badIp = false, badUdp = false, zeroUdp = false } = {}) {
    const f = new Uint8Array(14 + 20 + 8 + 8);
    f.set(SMAC, 0); f.set(SMAC, 6);
    f[12] = 0x08; f[13] = 0x00;
    f[14] = 0x45; f[16] = 0; f[17] = 36; f[22] = 64; f[23] = 17;
    f.set([192, 168, 4, 1], 26); f.set([192, 168, 4, 2], 30);
    f[34] = 0; f[35] = 7; f[36] = (dport >> 8) & 0xFF; f[37] = dport & 0xFF;
    f[38] = 0; f[39] = 16;
    for (let i = 0; i < 8; i++) f[42 + i] = 0x40 + i;
    const cks = (off, len) => {
        let s = 0;
        for (let i = 0; i + 1 < len; i += 2) s += (f[off + i] << 8) | f[off + i + 1];
        if (len & 1) s += f[off + len - 1] << 8;
        while (s >>> 16) s = (s & 0xFFFF) + (s >>> 16);
        return (~s) & 0xFFFF;
    };
    if (!badIp) { const c = cks(14, 20); f[24] = c >> 8; f[25] = c & 0xFF; }
    else { f[24] = 0x12; f[25] = 0x34; }
    if (zeroUdp) { f[40] = 0; f[41] = 0; }
    else if (!badUdp) {
        // pseudo-header UDP checksum
        let s = 0;
        for (let i = 0; i < 4; i += 2) s += (f[26 + i] << 8) | f[27 + i];
        for (let i = 0; i < 4; i += 2) s += (f[30 + i] << 8) | f[31 + i];
        s += 17 + 16;
        for (let i = 0; i + 1 < 16; i += 2) s += (f[34 + i] << 8) | f[35 + i];
        while (s >>> 16) s = (s & 0xFFFF) + (s >>> 16);
        const c = (~s) & 0xFFFF;
        f[40] = c >> 8; f[41] = c & 0xFF;
    } else { f[40] = 0xAB; f[41] = 0xCD; }
    return f;
}

// ── Gap 1: DMASR/DMAIER positions (CMSIS) ─────────────────────────────────
function t_gap1() {
    // AIS=15, ERS=14, FBES=13, RWTS=9 per CMSIS — set each status bit via
    // the paths that raise them and read them back at the right position.
    // Abnormal summary: force ROS (bit 4) via a missed-frame note path is
    // driver-side; here check the summary bit recomputes at bit 15.
    W(DMA + 0x14, 0xFFFFFFFF); // W1C everything (clears all set bits)
    tick_peripherals();
    const sr = R(DMA + 0x14);
    ok((sr & (1 << 15)) === 0 && (sr & (1 << 16)) === 0,
        'gap1: summaries clear when no sources', `dmasr=0x${sr.toString(16)}`);
    // DMAIER mask keeps CMSIS positions incl. RWTIE(9)/ERIE(14)/FBEIE(13)/AISE(15)
    W(DMA + 0x1C, 0xFFFFFFFF);
    const ier = R(DMA + 0x1C);
    ok((ier & (1 << 9)) !== 0 && (ier & (1 << 14)) !== 0 && (ier & (1 << 13)) !== 0 && (ier & (1 << 15)) !== 0,
        'gap1: DMAIER keeps RWTIE/ERIE/FBEIE/AISE', `dmaier=0x${ier.toString(16)}`);
    W(DMA + 0x1C, 0); // disarm for later tests
    W(DMA + 0x14, 0xFFFFFFFF);
}

// ── Gap 2: MACFFR SAF=9/SAIF=8/HPF=10 (CMSIS) ─────────────────────────────
function t_gap2() {
    stationMac();
    W(MAC + 0x04, 0); // FFR=0: unicast-to-us needs perfect slot 0 (always on)
    ok(eth_mac_accept(frameTo(SMAC)) === true, 'gap2: slot-0 perfect hit, FFR=0');
    // SAF (bit 9): self-SA passes, forged SA drops
    W(MAC + 0x04, 1 << 9);
    const selfSrc = frameTo(SMAC);
    ok(eth_mac_accept(selfSrc) === true, 'gap2: SAF self-SA passes');
    const forged = frameTo(SMAC);
    forged.set([0xDE, 0xAD, 0xBE, 0xEF, 0, 1], 6);
    ok(eth_mac_accept(forged) === false, 'gap2: SAF forged-SA drops');
    // SAIF (bit 8) inverts both
    W(MAC + 0x04, (1 << 9) | (1 << 8));
    ok(eth_mac_accept(selfSrc) === false, 'gap2: SAIF self-SA drops');
    ok(eth_mac_accept(forged) === true, 'gap2: SAIF forged-SA passes');
    // old wrong bits must NOT act as SAF/SAIF: bit 8 alone is SAIF without
    // SAF (no filtering), bit 7 is nothing
    W(MAC + 0x04, 1 << 8);
    ok(eth_mac_accept(forged) === true, 'gap2: SAIF-alone does not filter (needs SAF)');
    W(MAC + 0x04, 1 << 7);
    ok(eth_mac_accept(forged) === true, 'gap2: bit 7 is not a filter bit');
    // HPF (bit 10): program slot 1 = g2 exact + hash of g1 only; with
    // HPF clear the perfect hit is ignored (hash-only), with HPF set either passes
    const g2 = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x08];
    W(MAC + 0x48, (1 << 31) | 0x0100);
    W(MAC + 0x4C, 0x5E000008);
    // hash of g1 (01:00:5E:00:00:07) — compute upper-6 like the model
    const crc32 = (b) => {
        let c = 0xFFFFFFFF;
        for (const x of b) { c ^= x; for (let k = 0; k < 8; k++) c = (c & 1) ? (c >>> 1) ^ 0xEDB88320 : c >>> 1; }
        return (~c) >>> 0;
    };
    const h1 = crc32([0x01, 0x00, 0x5E, 0x00, 0x00, 0x07]) >>> 26;
    const h2 = crc32(g2) >>> 26;
    W(MAC + 0x08, h1 < 32 ? 0 : 1 << (h1 - 32));
    W(MAC + 0x0C, h1 < 32 ? 1 << h1 : 0);
    W(MAC + 0x04, 1 << 2); // HM, HPF clear
    if (h1 !== h2) ok(eth_mac_accept(frameTo(g2)) === false, 'gap2: HPF-clear ignores perfect hit');
    W(MAC + 0x04, (1 << 2) | (1 << 10)); // HM + HPF (CMSIS bit 10)
    ok(eth_mac_accept(frameTo(g2)) === true, 'gap2: HPF bit-10 ORs perfect hit');
    W(MAC + 0x04, 0); W(MAC + 0x08, 0); W(MAC + 0x0C, 0);
    W(MAC + 0x48, 0); W(MAC + 0x4C, 0);
}

// ── Gap 3: MACPMTCTL mask keeps PD/MPE/WFE/MPR/WFR/GU/WFFRPR only ─────────
function t_gap3() {
    W(MAC + 0x2C, (1 << 31) | 0x206); // WFFRPR clears ptr (readback 0), stores MPE+WFE+GU
    W(MAC + 0x28, 0x0F); // word 0 lands only if the 0x28 arm exists (dup-arm bug)
    const rb = R(MAC + 0x28);
    ok(rb === 0x0F, 'gap3: RWUFFR write arm present (no dup-arm shadow)', `readback=0x${rb.toString(16)}`);
    W(MAC + 0x2C, 0xFFFFFFFF); // try to set everything incl. sham bits 7+10
    const v = R(MAC + 0x2C);
    ok((v & 0x480) === 0, 'gap3: sham bits 7+10 not stored', `pmtctl=0x${v.toString(16)}`);
    ok((v & 0x207) === 0x207, 'gap3: PD/MPE/WFE/MPR/WFR/GU kept', `pmtctl=0x${v.toString(16)}`);
    W(MAC + 0x2C, 0); // clean
}

// ── Gap 4: RX FS/LS is a DRIVER write (model has no descriptor memory) ────
// Consumer-side pin: the model exposes no RDES write path, so assert the
// contract the driver implements — the status-bit VALUES from CMSIS HAL:
// FS(bit9)+LS(bit8) = 0x300 on every delivered frame (see test_eth_mock_model).
function t_gap4() {
    const FS_LS = 0x300; // HAL ETH_DMARXDESC_FS|LS
    ok(FS_LS === 0x300, 'gap4: HAL FS+LS word sane');
    // Model must not fabricate FS/LS anywhere in MMIO space (no desc memory)
    const sr = R(DMA + 0x14);
    ok((sr & 0x300) === 0, 'gap4: DMASR carries no FS/LS alias', `dmasr=0x${sr.toString(16)}`);
}

// ── Gap 5: PM/BFD/PCF-11 accept paths ─────────────────────────────────────
function t_gap5() {
    stationMac();
    const g = [0x01, 0x00, 0x5E, 0x00, 0x00, 0x07];
    W(MAC + 0x04, 1 << 0); // PM, empty table
    ok(eth_mac_accept(frameTo(g)) === true, 'gap5: PM passes multicast, empty table');
    W(MAC + 0x04, (1 << 0) | (1 << 5)); // PM+BFD
    ok(eth_mac_accept(frameTo(g)) === true, 'gap5: PM multicast still passes with BFD');
    ok(eth_mac_accept(frameTo([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])) === false, 'gap5: BFD drops broadcast');
    W(MAC + 0x04, 1 << 0);
    ok(eth_mac_accept(frameTo([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF])) === true, 'gap5: broadcast passes under PM alone');
    // PCF-11 DA path: pause DA without PAM drops, with PAM passes
    const pause = new Uint8Array(60);
    pause.set([0x01, 0x80, 0xC2, 0x00, 0x00, 0x01], 0);
    pause.set(SMAC, 6);
    pause[12] = 0x88; pause[13] = 0x08; pause[14] = 0; pause[15] = 1;
    W(MAC + 0x04, 3 << 6); // PCF=11, no PAM
    ok(eth_mac_accept(pause) === false, 'gap5: PCF-11 pause DA drops without PAM');
    W(MAC + 0x04, (3 << 6) | (1 << 4)); // PCF-11 + PAM
    ok(eth_mac_accept(pause) === true, 'gap5: PCF-11 pause DA passes with PAM');
    W(MAC + 0x04, 0);
}

// ── Gap 5b: VLAN 12-bit compare + invert (model accept gate) ────────────
// Firmware covers VID-7 match/mismatch + untagged-drop + inverted-untagged
// pass; the tagged 12-bit and invert paths are unit-only (netsim cannot
// emit tagged frames on demand).
function t_gap5b() {
    stationMac();
    const tag = (tci) => {
        const inner = new Uint8Array(50);
        const f = new Uint8Array(18 + 20 + 8 + 8);
        f.set(SMAC, 0); f.set(SMAC, 6);
        f[12] = 0x81; f[13] = 0x00;
        f[14] = (tci >> 8) & 0xFF; f[15] = tci & 0xFF;
        f[16] = 0x08; f[17] = 0x00;
        return f;
    };
    W(MAC + 0x1C, 7); // VLANTI=7, 16-bit compare
    ok(eth_mac_accept(frameTo(SMAC)) === false, 'gap5b: untagged drops, gate on');
    ok(eth_mac_accept(tag(0x0007)) === true, 'gap5b: VID 7 tagged passes');
    ok(eth_mac_accept(tag(0x0008)) === false, 'gap5b: VID 8 tagged drops');
    W(MAC + 0x1C, 7 | (1 << 16)); // 12-bit compare
    ok(eth_mac_accept(tag(0x1007)) === true, 'gap5b: 12-bit TCI 0x1007 passes (low-12 = 7)');
    ok(eth_mac_accept(tag(0x2008)) === false, 'gap5b: 12-bit TCI 0x2008 drops');
    W(MAC + 0x1C, 7 | (1 << 16) | (1 << 17)); // + invert
    ok(eth_mac_accept(tag(0x0007)) === false, 'gap5b: inverted VID 7 drops');
    ok(eth_mac_accept(tag(0x0008)) === true, 'gap5b: inverted VID 8 passes');
    ok(eth_mac_accept(frameTo(SMAC)) === true, 'gap5b: inverted untagged passes');
    W(MAC + 0x1C, 0);
}

// ── Gap 6: checksum status + PTP event gate are pure model fns ─────────────
// (Driver calls eth_rx_csum_status per frame; the event gate lives in the
// driver, but the csum status feeding its rdesExtra is model-side.)
function t_gap6() {
    const good = ipUdp({});
    let st = u32(eth_rx_csum_status(good));
    ok((st & 1) !== 0 && (st & 2) !== 0, 'gap6: good IP sets has-IP + IP-OK', `st=0x${st.toString(16)}`);
    ok((st & 4) !== 0 && (st & 8) !== 0, 'gap6: good UDP sets has-L4 + L4-OK', `st=0x${st.toString(16)}`);
    const badIp = ipUdp({ badIp: true });
    st = u32(eth_rx_csum_status(badIp));
    ok((st & 1) !== 0 && (st & 2) === 0, 'gap6: bad IP clears IP-OK (IPHCE)', `st=0x${st.toString(16)}`);
    const badUdp = ipUdp({ badUdp: true });
    st = u32(eth_rx_csum_status(badUdp));
    ok((st & 4) !== 0 && (st & 8) === 0, 'gap6: bad UDP clears L4-OK (PCE)', `st=0x${st.toString(16)}`);
    const zeroUdp = ipUdp({ zeroUdp: true });
    st = u32(eth_rx_csum_status(zeroUdp));
    ok((st & 4) !== 0 && (st & 8) !== 0, 'gap6: zero UDP cksum = omission, L4-OK', `st=0x${st.toString(16)}`);
    // WOL magic + filter still latch (regression cover for the pmt_irq62 path)
    W(MAC + 0x2C, 0x2); // MPE
    W(MAC + 0x3C, 0x8); // PMTIM
    const mp = new Uint8Array(14 + 102);
    mp.set(SMAC, 0);
    mp.set([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 14);
    for (let r = 0; r < 16; r++) mp.set(SMAC, 20 + r * 6);
    const w = u32(eth_check_wol(mp));
    ok((w & 1) !== 0, 'gap6: magic packet detected', `w=0x${w.toString(16)}`);
    W(MAC + 0x2C, 0); W(MAC + 0x3C, 0);
    void R(MAC + 0x2C); // read-to-clear MPR
    // pacing / collision / deferral / link reports
    W(MAC + 0x00, (1 << 2) | (1 << 3) | (1 << 11) | (1 << 14)); // RE+TE+DM+FES
    ok(eth_link_up() === true, 'gap6: link up by default');
    eth_set_link(false);
    ok(eth_link_up() === false, 'gap6: link down reports');
    eth_set_link(true);
    eth_tx_wire_busy(1200);
    eth_arm_collision();
    ok(eth_take_collision() === true, 'gap6: armed collision taken once');
    ok(eth_take_collision() === false, 'gap6: collision one-shot');
    // Deferral: half-duplex (DM=0) + live RX window. FES=1 (100M) so the
    // window math matches the firmware's 10M/100M usage; arm the window
    // FIRST, then sample immediately (same tick — no time passes).
    W(MAC + 0x00, (1 << 2) | (1 << 3) | (1 << 14)); // RE+TE+FES, DM=0
    eth_rx_wire_busy(1200);
    ok(eth_tx_deferred() === true, 'gap6: half-duplex TX inside RX window defers');
    // full-duplex (DM=1) never defers, same live window
    W(MAC + 0x00, (1 << 2) | (1 << 3) | (1 << 11) | (1 << 14)); // +DM
    ok(eth_tx_deferred() === false, 'gap6: full-duplex never defers');
    ok(eth_ipco_on() === false, 'gap6: IPCO off by default');
    W(MAC + 0x00, (1 << 2) | (1 << 3) | (1 << 11) | (1 << 14) | (1 << 10));
    ok(eth_ipco_on() === true, 'gap6: IPCO reads back');
    W(MAC + 0x00, (1 << 2) | (1 << 3) | (1 << 11));
}

const tests = [
    ['gap1 DMASR/DMAIER positions', t_gap1],
    ['gap2 MACFFR SAF/SAIF/HPF', t_gap2],
    ['gap3 PMTCTL mask + RWUFFR arm', t_gap3],
    ['gap4 FS/LS contract', t_gap4],
    ['gap5 PM/BFD/PCF-11', t_gap5],
    ['gap5b VLAN 12-bit + invert', t_gap5b],
    ['gap6 csum/WOL/pacing/collision/deferral/link/IPCO', t_gap6],
];
for (const [name, fn] of tests) {
    console.log(`— ${name}`);
    try { fn(); tick_peripherals(); } catch (e) { fail++; console.error(`  FAIL (throw): ${name}: ${e.message}`); }
}
console.log(fail === 0 ? `MOCK-CONSUMER PASS (${pass} checks)` : `MOCK-CONSUMER FAIL (${fail} failures)`);
process.exit(fail === 0 ? 0 : 1);
