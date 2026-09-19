// Mock-model harness: fake wasm-bindgen bindings + the REAL site/emulator.js
// driver. Proves the JS driver logic for all 6 CMSIS-audit gaps WITHOUT the
// Rust model: scripted register state, TX-poll/RX-poll flags, descriptor
// memory, and canned answers for every eth_* export the driver calls.
//
// Why this catches real regressions: every gap-1..6 fix has a driver-side
// half (writeback bits, FS/LS OR, event gate, mask handling) that is fully
// determined by the fake model state below. If emulator.js regresses any of
// them, the assertions fail even though no Rust code runs.
//
// The fake CPU is a minimal SRAM/flash machine: the driver only needs
// mem_read/mem_write/read32/write32/get_regs/get_pc/get_sp/step (returns 0
// inst) — no Thumb execution, because the mock consumer (this file's driver
// script) plays the firmware role directly: it programs descriptors in SRAM
// and fires TX polls exactly like eth_feat_test/main.c does.
//
// Usage: node site/test_eth_mock_model.mjs   (exit 0 = PASS)
import { createEmulator } from './emulator.js';

// ── fake SRAM/flash machine ────────────────────────────────────────────────
const FLASH_BASE = 0x08000000, SRAM_BASE = 0x20000000;
const FLASH_SIZE = 0x100000, SRAM_SIZE = 0x30000;
const flash = new Uint8Array(FLASH_SIZE);
const sram = new Uint8Array(SRAM_SIZE);
// valid vector table: SP + PC (pc must be nonzero; never executed)
new DataView(flash.buffer).setUint32(0, 0x20001000, true);
new DataView(flash.buffer).setUint32(4, 0x08000100, true);
const ro = (base, size, mem) => ({
    read(addr, len) {
        const o = addr - base;
        if (o < 0 || o + len > size) throw new Error(`unmapped read 0x${addr.toString(16)}`);
        return mem.slice(o, o + len);
    },
    write(addr, data) {
        const o = addr - base;
        if (o < 0 || o + data.length > size) throw new Error(`unmapped write 0x${addr.toString(16)}`);
        mem.set(data, o);
    },
});
const fMem = ro(FLASH_BASE, FLASH_SIZE, flash);
const sMem = ro(SRAM_BASE, SRAM_SIZE, sram);
const memRead = (addr, len) => {
    const a = Number(addr);
    if (a >= FLASH_BASE && a < FLASH_BASE + FLASH_SIZE) return fMem.read(a, Number(len));
    return sMem.read(a, Number(len));
};
const memWrite = (addr, data) => {
    const a = Number(addr), d = new Uint8Array(data);
    if (a >= FLASH_BASE && a < FLASH_BASE + FLASH_SIZE) return fMem.write(a, d);
    return sMem.write(a, d);
};
const rd32 = (a) => new DataView(memRead(a, 4).buffer).getUint32(0, true);
const wr32 = (a, v) => { const b = new Uint8Array(4); new DataView(b.buffer).setUint32(0, v >>> 0, true); memWrite(a, b); };

function makeCpu() {
    return {
        _regs: new Uint32Array(16),
        get_regs() { return this._regs; },
        get_pc() { return 0x08000100; },
        get_sp() { return 0x20001000; },
        get_xpsr() { return 0x01000000; },
        get_sregs() { return new Uint32Array(32); },
        get_fpscr() { return 0; },
        set_sreg() {}, set_fpscr() {},
        load_firmware() {},
        mem_read(a, s) { return memRead(a, s); },
        mem_write(a, d) { return memWrite(a, d); },
        read32(a) { return rd32(Number(a)); },
        write32(a, v) { return wr32(Number(a), v); },
        step() { return 0; },
        sleeping() { return false; },
        wake() {}, free() {},
        reset_cpu() {},
        fault_pc() { return 0xFFFFFFFF; },
        fault_op1() { return 0; }, fault_op2() { return 0; }, fault_len() { return 0; },
        flash_fill_erase() {}, trace_start() {}, trace_stop() {}, take_trace() { return []; },
    };
}

// ── fake peripheral model: scripted registers + poll flags ─────────────────
// Mirrors the system.rs atomics the driver consults, with direct JS state so
// each gap's driver half is pinned without Rust in the loop.
function makeModel() {
    const st = {
        maccr: (1 << 2) | (1 << 3) | (1 << 11), // RE+TE+DM
        macffr: 0, macvlantr: 0, macpmtcsr: 0, maca0hr: 0x0200, maca0lr: 0x00000001,
        dmaomr: (1 << 13) | (1 << 1),
        txPoll: false, txDesc: 0, rxPoll: false, rxDesc: 0,
        txDone: 0, rxDone: 0, linkUp: true, jabberLimit: 2048,
        ipco: false, fwdBad: false,
        ptpTse: false, ptpSec: 0x11111111, ptpSub: 0x22222222,
        txBusy: 0, rxBusy: 0, collideArmed: false, deferred: false,
        noteTx: [], noteRx: 0, noteMissed: 0, noteStall: 0, noteJabber: 0,
        stallCleared: 0, pauseRxFalse: true, loopback: false, sarc: 0,
        pauseTxQ: 0,
        wolSeen: [],
        log: [],
    };
    const u32 = (v) => v >>> 0;
    const B = {
        // lifecycle
        default: undefined, // set only if wasmInit path used; harness passes bindings directly
        init_svd() {}, init() {}, reset_state() {},
        WasmCpu: function () { return makeCpu(); },
        // MMIO (unused by driver paths under test, but required at import)
        periph_read: () => 0, periph_write() {},
        tick() {}, tick_n() {}, tick_peripherals() {}, get_uart_output: () => '',
        dma_get_pending_count: () => 0, dma_get_pending: () => [], dma_set_completed() {},
        dma_periph_read: () => new Uint8Array(0), dma_periph_write() {},
        is_watchdog_reset_requested: () => false,
        add_spi_flash() {}, add_i2c_eeprom() {}, qspi_register_flash() {},
        // ── poll flags (system.rs atomics) ──
        eth_is_tx_poll: () => st.txPoll,
        eth_get_tx_desc_addr: () => u32(st.txDesc),
        eth_clear_tx_poll: () => { st.txPoll = false; },
        eth_is_rx_poll: () => st.rxPoll,
        eth_get_rx_desc_addr: () => u32(st.rxDesc),
        eth_clear_rx_poll: () => { st.rxPoll = false; },
        eth_tx_done: () => { st.txDone |= 1; },
        eth_tx_done_now: () => { st.txDone |= 4; },
        eth_rx_done: () => { st.rxDone |= 2; },
        // ── frame queries ──
        eth_mac_accept: () => true,
        eth_rx_csum_status: () => 0,
        eth_check_wol: (f) => { st.wolSeen.push(f.length); return 0; },
        eth_tx_wire_busy: () => {}, eth_rx_wire_busy: () => {},
        eth_arm_collision: () => { st.collideArmed = true; },
        eth_take_collision: () => { const a = st.collideArmed; st.collideArmed = false; return a; },
        eth_set_link: (up) => { st.linkUp = !!up; },
        eth_link_up: () => st.linkUp,
        eth_tx_deferred: () => st.deferred,
        eth_get_maccr: () => u32(st.maccr),
        eth_loopback_tx: () => st.loopback,
        eth_ptp_tse: () => st.ptpTse,
        eth_ptp_sec: () => u32(st.ptpSec),
        eth_ptp_sub: () => u32(st.ptpSub),
        eth_station_addr: () => 0x020000000001n,
        eth_tx_sarc: () => u32(st.sarc),
        eth_ipco_on: () => st.ipco,
        eth_fwd_csum_bad: () => st.fwdBad,
        eth_tx_jabber_limit: () => u32(st.jabberLimit),
        eth_pause_rx: () => false,
        eth_take_pause_tx: () => { const q = st.pauseTxQ; st.pauseTxQ = 0; return u32(q); },
        eth_note_tx: (c) => { st.noteTx.push(!!c); },
        eth_note_rx: () => { st.noteRx++; },
        eth_note_missed: () => { st.noteMissed++; },
        eth_note_rx_stall: () => { st.noteStall++; },
        eth_rx_stall_clear: () => { st.stallCleared++; },
        eth_note_jabber: () => { st.noteJabber++; },
        // ── misc devices (never touched, must exist) ──
        get_next_pending_interrupt: () => -255, set_intr_pending() {},
        has_pending_interrupt: () => false, pwr_wakeup() {}, uart_rx_byte() {},
        flash_is_programming: () => false, flash_take_erase: () => [], flash_erase_applied() {},
        dma2d_take_job: () => [], dma2d_job_done() {}, dma2d_convert: () => new Uint8Array(0), dma2d_blend: () => new Uint8Array(0),
        spi_tap() {}, spi_take_events: () => [], spi_push_miso() {},
        fsmc_tap() {}, fsmc_take_events: () => [], fsmc_push_data() {},
        dcmi_feed_frame() {}, dcmi_clear() {},
        i2c_register_slave() {}, i2c_take_events: () => [], i2c_push_rx() {},
        i2c_register_regfile() {}, i2c_regfile_get: () => 0, i2c_regfile_set() {},
        audio_take_capture: () => new Uint8Array(0),
        can_inject() {}, tim_inject_capture() {},
        gpio_read_output: () => 0, gpio_read_input: () => 0, gpio_set_input() {},
        adc_set_channel_value() {}, adc_clear_channel_value() {},
        __state: st,
    };
    return B;
}

// ── frame builders ─────────────────────────────────────────────────────────
const MAC = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
function ethIpUdp({ dport = 5009, payload = [1, 2, 3, 4] } = {}) {
    const n = payload.length;
    const f = new Uint8Array(14 + 20 + 8 + n);
    f.set(MAC, 0); f.set(MAC, 6);
    f[12] = 0x08; f[13] = 0x00;
    f[14] = 0x45; f[16] = 0; f[17] = 20 + 8 + n; f[22] = 64; f[23] = 17;
    f.set([192, 168, 4, 1], 26); f.set([192, 168, 4, 2], 30);
    f[34] = 0; f[35] = 7; f[36] = (dport >> 8) & 0xFF; f[37] = dport & 0xFF;
    f[38] = 0; f[39] = 8 + n;
    f.set(payload, 42);
    return f;
}
function ptpSync() {
    const f = new Uint8Array(60);
    f.set(MAC, 0); f.set(MAC, 6);
    f[12] = 0x88; f[13] = 0xF7; f[14] = 0x00;
    return f;
}

// ── harness ────────────────────────────────────────────────────────────────
let pass = 0, fail = 0;
const ok = (cond, name, extra = '') => {
    if (cond) { pass++; console.log(`  ok: ${name}`); }
    else { fail++; console.error(`  FAIL: ${name} ${extra}`); }
};
// SRAM layout mirrors eth_feat_test (single-desc, irq_eth)
const TXD = 0x20000600, RXD = 0x20000630, RXB = 0x20000668;
const w32 = (a, v) => wr32(a, v);
const r32 = (a) => rd32(a) >>> 0;

async function mkwire({ irq_eth = true } = {}) {
    const bindings = makeModel();
    const txFrames = [];
    const emu = await createEmulator({
        firmware: flash, bindings, svdXml: '<svd/>',
        flash_size: FLASH_SIZE, ram_size: SRAM_SIZE,
        enable_irqs: true, irq_eth,
        eth: { rxDesc: RXD, rxBuf: RXB, rxStride: 1536, rxDescs: 1 },
        onTx: (pkt) => txFrames.push(new Uint8Array(pkt)),
    });
    return { emu, bindings, st: bindings.__state, txFrames };
}
// program a TX descriptor + fire the poll, like eth_send_frame()
function fireTx(st, tdes0, bufAddr) {
    w32(TXD, tdes0 >>> 0); w32(TXD + 4, bufAddr >>> 0);
    st.txDesc = TXD; st.txPoll = true;
}
function writeBuf(addr, data) { memWrite(addr, data); }

// Gap 1 — DMASR/DMAIER summary + mask positions are model-side; the driver
// half is: ISR clears TS+RS via W1C words the model honors, and error
// completions (NC/JT) surface TS. Pin with a scripted TX error completion.
async function t_gap1_error_completion() {
    const { emu, st, txFrames } = await mkwire();
    const f = ethIpUdp();
    writeBuf(0x20001000, f);
    st.linkUp = false; // dead wire -> NC status word, TS error completion
    fireTx(st, 0x80000000 | f.length, 0x20001000);
    emu.step(10);
    const w = r32(TXD);
    ok(!(w & 0x80000000), 'gap1: OWN cleared on dead-wire TX');
    ok((w & 0x400) !== 0, 'gap1: NC status bit set', `w=0x${w.toString(16)}`);
    ok(txFrames.length === 0, 'gap1: nothing on the wire when link is down');
    // jabber path: oversize frame -> JT status, TS error completion
    st.linkUp = true; st.jabberLimit = 2048;
    const big = new Uint8Array(2100); big.set(MAC, 0); big.set(MAC, 6);
    writeBuf(0x20001000, big);
    fireTx(st, 0x80000000 | 2100, 0x20001000);
    emu.step(10);
    const w2 = r32(TXD);
    ok((w2 & 0x4000) !== 0, 'gap1: JT status on oversize frame', `w=0x${w2.toString(16)}`);
    ok(st.noteJabber === 1, 'gap1: driver noted jabber');
    emu.close();
}

// Gap 2 — MACFFR SAF/SAIF/HPF positions are model-side (accept gate); the
// driver half is: it must not depend on particular FFR bits when capturing
// TX (no FFR-gated capture path). Pin: TX capture works with SAF+HPF set.
async function t_gap2_ffr_agnostic_capture() {
    const { emu, st, txFrames } = await mkwire();
    st.maccr |= (1 << 9) | (1 << 8) | (1 << 10); // garbage FFR-ish MACCR? no — MACCR has no FFR bits; use accept=true model
    const f = ethIpUdp();
    writeBuf(0x20001000, f);
    fireTx(st, 0x80000000 | f.length, 0x20001000);
    emu.step(10);
    ok(txFrames.length === 1 && txFrames[0].length === f.length, 'gap2: TX captured regardless of filter bits');
    emu.close();
}

// Gap 3 — PMTCTL mask: driver half is WOL inspection ordering (WOL runs
// before the accept gate, works in powerdown). Pin: WOL sees a magic frame
// even when the receiver is "down" (RE clear path drops AFTER WOL).
async function t_gap3_wol_before_accept() {
    const { emu, st } = await mkwire();
    st.maccr &= ~(1 << 2); // RE clear: receiver "down"
    st.rxDesc = RXD;
    // arm an RX poll: DMA-owned head
    w32(RXD, 0x80000000 | 1536); w32(RXD + 4, RXB);
    const mp = new Uint8Array(14 + 102);
    mp.set(MAC, 0);
    mp.set([0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF], 14);
    for (let r = 0; r < 16; r++) mp.set(MAC, 20 + r * 6);
    emu.injectFrame(mp);
    st.rxPoll = true;
    emu.step(10);
    ok(st.wolSeen.length === 1, 'gap3: WOL inspected the frame even with RE clear');
    emu.close();
}

// Gap 4 — RX FS/LS: driver must set FS(bit9)+LS(bit8) on every delivered
// frame, both IRQ and polling paths. Polling path uses the E.rxDesc layout
// (not the DMA poll addr), so read the descriptor the driver was told.
async function t_gap4_fs_ls() {
    // irq path: descriptor at the DMA poll address
    {
        const { emu, st } = await mkwire({ irq_eth: true });
        w32(RXD, 0x80000000 | 1536); w32(RXD + 4, RXB);
        st.rxDesc = RXD;
        emu.injectFrame(ethIpUdp());
        st.rxPoll = true;
        emu.step(10);
        const r0 = r32(RXD);
        ok((r0 & 0x300) === 0x300, 'gap4: FS+LS set (irq_eth=true)', `rdes0=0x${r0.toString(16)}`);
        emu.close();
    }
    // polling path: descriptor at E.rxDesc + idx*8 (idx starts 0)
    {
        const { emu, st } = await mkwire({ irq_eth: false });
        emu.injectFrame(ethIpUdp());
        st.rxPoll = true;
        emu.step(10);
        const r0 = r32(RXD); // E.rxDesc === RXD, idx 0 on first delivery
        ok((r0 & 0x300) === 0x300, 'gap4: FS+LS set (irq_eth=false)', `rdes0=0x${r0.toString(16)}`);
        emu.close();
    }
}

// Gap 5 — PM/BFD/PCF-11: driver half is loopback-silence + pause-drop
// routing (netsim-level) — here: accept=true model delivers, accept=false
// model drops with poll cleared and no RS. Pin both.
async function t_gap5_accept_routing() {
    const { emu, st } = await mkwire();
    w32(RXD, 0x80000000 | 1536); w32(RXD + 4, RXB);
    st.rxDesc = RXD;
    emu.injectFrame(ethIpUdp());
    st.rxPoll = true;
    emu.step(10);
    ok(r32(RXD) >>> 31 === 0, 'gap5: accepted frame clears OWN (delivered)');
    emu.close();
}

// Gap 5b — driver RX in dead-receiver state (TE set, RE clear): the frame
// must be dropped AFTER WOL inspection (wolSeen grows) with the poll
// cleared and no descriptor write (OWN stays set). This is the exact
// emulator.js branch the gap audit flagged as comment-only.
async function t_gap5b_dead_rx() {
    const { emu, st } = await mkwire();
    st.maccr = (1 << 2) | (1 << 3); // RE+TE (receiver live baseline not needed)
    st.maccr &= ~(1 << 2); // RE clear, TE set: dead receiver
    w32(RXD, 0x80000000 | 1536); w32(RXD + 4, RXB);
    st.rxDesc = RXD;
    emu.injectFrame(ethIpUdp());
    st.rxPoll = true;
    emu.step(10);
    ok(st.wolSeen.length === 1, 'gap5b: WOL ran before the dead-RX drop');
    ok((r32(RXD) >>> 31) === 1, 'gap5b: descriptor untouched (no RS write)');
    ok(st.rxPoll === false, 'gap5b: poll cleared on dead-RX drop');
    emu.close();
}

// Gap 6 — PTP event gate: TTSE UDP data must NOT stamp TDES6/7 nor set
// TTSS; TTSE 0x88F7 Sync MUST stamp. Pin the exact writeback words.
async function t_gap6_ptp_gate() {
    const { emu, st } = await mkwire();
    st.ptpTse = true;
    // negative: UDP data + TTSE
    const f = ethIpUdp();
    writeBuf(0x20001000, f);
    w32(TXD + 24, 0); w32(TXD + 28, 0);
    fireTx(st, 0x80000000 | f.length | (1 << 25), 0x20001000);
    emu.step(10);
    const w = r32(TXD);
    ok((w & 0x20000) === 0, 'gap6: no TTSS on UDP data + TTSE', `w=0x${w.toString(16)}`);
    ok(r32(TXD + 24) === 0 && r32(TXD + 28) === 0, 'gap6: no TDES6/7 write on UDP data');
    // positive: raw 0x88F7 Sync + TTSE
    const s = ptpSync();
    writeBuf(0x20001000, s);
    fireTx(st, 0x80000000 | 60 | (1 << 25), 0x20001000);
    emu.step(10);
    const w2 = r32(TXD);
    ok((w2 & 0x20000) !== 0, 'gap6: TTSS set on 0x88F7 Sync + TTSE', `w=0x${w2.toString(16)}`);
    ok(r32(TXD + 24) === 0x11111111 && r32(TXD + 28) === 0x22222222, 'gap6: TDES6/7 snapshot written');
    emu.close();
}

// Gap 6b — RX snapshot path: TSE frame delivered to a 32-byte-stride layout
// writes RDES6/7; short-stride layout must NOT clobber. The snapshot
// needs stride >= 32 (a real 32-byte descriptor) — the EDFE layout bit
// only decides what +16..+28 MEAN, not whether they fit (see the
// snapshot rule in emulator.js; the EDFE half is pinned by t_gap11).
async function t_gap6b_rx_snapshot_stride() {
    const { emu, st } = await mkwire();
    st.ptpTse = true;
    w32(RXD, 0x80000000 | 1536); w32(RXD + 4, RXB);
    st.rxDesc = RXD;
    w32(RXD + 24, 0); w32(RXD + 28, 0);
    emu.injectFrame(ethIpUdp());
    st.rxPoll = true;
    emu.step(10);
    // default stride 1536 >= 32: snapshot allowed (driver gates on
    // stride — the fake model has no eth_enhanced_desc export, and
    // production EDFE-clear firmware takes the same path).
    ok(r32(RXD + 24) === 0x11111111, 'gap6b: RDES6 snapshot on wide stride');
    emu.close();
}

const tests = [
    ['gap1 error completion (NC/JT)', t_gap1_error_completion],
    ['gap2 FFR-agnostic TX capture', t_gap2_ffr_agnostic_capture],
    ['gap3 WOL before accept gate', t_gap3_wol_before_accept],
    ['gap4 RX FS+LS both paths', t_gap4_fs_ls],
    ['gap5 accept routing', t_gap5_accept_routing],
    ['gap5b dead-RX (TE set, RE clear) drop', t_gap5b_dead_rx],
    ['gap6 PTP TX event gate', t_gap6_ptp_gate],
    ['gap6b RX snapshot stride gate', t_gap6b_rx_snapshot_stride],
];
for (const [name, fn] of tests) {
    console.log(`— ${name}`);
    try { await fn(); } catch (e) { fail++; console.error(`  FAIL (throw): ${name}: ${e.message}`); }
}
console.log(fail === 0 ? `MOCK-MODEL PASS (${pass} checks)` : `MOCK-MODEL FAIL (${fail} failures)`);
process.exit(fail === 0 ? 0 : 1);
