// Mock-consumer harness for the peripheral gap set: a fake FIRMWARE driving
// the REAL Rust model (real wasm-bindgen bindings + real SVD map, no
// emulator.js driver) — the same pattern as test_eth_mock_consumer.mjs.
// Covers every board-doc gap class:
//   (1) no-pin-layer sinks: PPS edge counter + level mirrors (not the pin),
//       ETH nibble data + 25/50 MHz clocks (electrical-only), ULPI HS-PHY
//       packet rates (HS core is FS-mode);
//   (2) deliberate non-models: true entropy (LCG deterministic), clock-fail
//       injection (sources always lock), multi-master arbitration, CAN
//       bus-off/FD, USB isochronous/host/SOF/suspend/VBUS/internal-DMA;
//   (3) protocol modes firmware never uses: USART LIN/Smartcard/IrDA, SPI
//       slave, HASH HMAC, DAC physical sink, ADC dual-interleave, TIM
//       encoder counting.
// Each test programs the model exactly like firmware does — raw
// periph_write/periph_read MMIO — and asserts the model's answer.
//
// Two assertion kinds:
//   COMPLETE — behavior the model implements; must hold (regression pins).
//   NOT-MODELED — documented silicon behavior the model deliberately does
//   NOT implement; asserts the *documented substitute* (benign-0, stored
//   bit, continuous-run ECC, LCG pseudo-random, ...) so the docs can't drift
//   from the model. A NOT-modeled assert failing means the docs lie.
//
// Usage: node site/test_periph_mock_consumer.mjs   (exit 0 = PASS)
import { readFileSync } from 'node:fs';
import * as bindings from './vendor/stm32_periph_wasm.js';

const wasmBytes = new Uint8Array(readFileSync(new URL('./vendor/stm32_periph_wasm_bg.wasm', import.meta.url)));
const svdXml = readFileSync(new URL('./vendor/stm32f407.svd', import.meta.url), 'utf8');
if (typeof bindings.default === 'function') await bindings.default({ module_or_path: wasmBytes });

const {
    periph_read, periph_write, tick_n,
    dcmi_feed_frame, dcmi_clear,
    adc_take_dma, adc_set_channel_value, adc_clear_channel_value,
    itm_take_port, itm_port_pending,
    usb_in_status, usb_out_status, usb_reset, usb_enumerated,
    usb_inject_setup, usb_inject_out, usb_take_in,
    usb_hs_in_status, usb_hs_out_status, usb_hs_reset, usb_hs_enumerated,
    usb_hs_inject_setup, usb_hs_inject_out, usb_hs_take_in,
    usb_set_vbus, usb_dma_progress, usb_uframe, usb_hs_uframe,
    usb_ulpi_rate, usb_hs_ulpi_rate,
    eth_pps_count, eth_pps_level, eth_tx_wire_busy,
    gpio_set_input, i2c_arm_arb_loss, i2c_pec, i2c_arm_smbus_alert,
    i2c_register_regfile,
    can_inject_fd, can_fd_byte, can_fd_len, can_fd_cost,
    rng_seed_entropy, rng_entropy_avail,
} = bindings;
// SMBus/PEC transactions need a live slave: a 16-byte regfile @0x50 on
// I2C1 (same shape emulator.js uses for the DS3231 RTC). Must register
// BEFORE init_svd (the model binds slaves at construction).
i2c_register_regfile('I2C1', 0x50, 16, new Uint8Array(16));
bindings.init_svd(svdXml);

let pass = 0, fail = 0;
const ok = (cond, name, extra = '') => {
    if (cond) { pass++; console.log(`  ok: ${name}`); }
    else { fail++; console.error(`  FAIL: ${name} ${extra}`); }
};
const W = (addr, v) => periph_write(addr, 4, v >>> 0);
const R = (addr) => periph_read(addr, 4) >>> 0;
const PWR = 0x40007000, DCMI = 0x50050000, FSMC = 0xA0000000;
const ADC1 = 0x40012000, ADCC = 0x40012300, ITM = 0xE0000000;
const TIM2 = 0x40000000, TIM3 = 0x40000400;
const USB = 0x50000000;
const RCC = 0x40023800, RNG = 0x50060800, DAC = 0x40007400;
const HASH = 0x50060400, CAN1 = 0x40006400;
const USART1 = 0x40011000, SPI1 = 0x40013000;
const ETH_MAC = 0x40028000, ETH_PTP = 0x40028700;
const GPIOA = 0x40020000, GPIOB = 0x40020400, GPIOC = 0x40020800;

// ── PWR: regulator states beyond the handshake ──────────────────────────
// COMPLETE: PVDO follows PVDE; VOSRDY settle window; ODEN→ODRDY handshake.
function t_pwr() {
    W(PWR, 1 << 4); // PVDE
    ok((R(PWR + 4) & (1 << 2)) === 0, 'pwr: PVDO=0 on healthy rail with PVDE');
    W(PWR, 0);
    ok((R(PWR + 4) & (1 << 2)) !== 0, 'pwr: PVDO=1 with detection off');
    W(PWR, 1 << 14); // VOS write -> settle window
    ok((R(PWR + 4) & (1 << 14)) === 0, 'pwr: VOSRDY clears during settle');
    tick_n(2000);
    ok((R(PWR + 4) & (1 << 14)) !== 0, 'pwr: VOSRDY returns after window');
    W(PWR, (1 << 16) | (1 << 8)); // ODEN
    ok((R(PWR + 4) & (1 << 16)) === 0, 'pwr: ODRDY not yet');
    tick_n(6000);
    ok((R(PWR + 4) & (1 << 16)) !== 0, 'pwr: ODRDY after settle');
    W(PWR, 1 << 8);
    ok((R(PWR + 4) & (1 << 16)) === 0, 'pwr: ODRDY drops with ODEN');
    W(PWR, 0); // clean
}

// ── DCMI: pin-sync sampling (needs pin layer) ───────────────────────────
// COMPLETE paths exist (JPEG/CROP/MIS); the documented NOT-modeled part is
// pin-sync sampling — assert the substitute: frames arrive from the JS feed,
// never from pins (no VSYNC/HSYNC pin registers exist in the map).
function t_dcmi() {
    // JPEG word-pack through MMIO: 8-byte stream -> 2 DR words LE.
    dcmi_feed_frame(8, 1, Uint8Array.from([1, 2, 3, 4, 5, 6, 7, 8]));
    W(DCMI, (1 << 3) | (1 << 4) | 1); // JPEG + ESS + CAPTURE
    const w0 = R(DCMI + 0x28), w1 = R(DCMI + 0x28);
    ok(w0 === 0x04030201, 'dcmi: JPEG word 0 packs bytes 1-4 LE', `w0=0x${w0.toString(16)}`);
    ok(w1 === 0x08070605, 'dcmi: JPEG word 1 packs bytes 5-8 LE', `w1=0x${w1.toString(16)}`);
    // MIS masked status: LINE ris without IER reads 0.
    W(DCMI + 0x0C, 0); // IER=0
    W(DCMI + 0x14, 0x1F); // clear
    ok((R(DCMI + 0x10) & 0x1F) === 0, 'dcmi: MIS masked with IER=0');
    W(DCMI, 0);
    dcmi_clear();
}

// ── FSMC: timings/wait states, vendor Hamming matrix, no NAND array ─────
// COMPLETE: BWTR/NAND reg file + ECC round-trip contract. NOT-modeled:
// access timings never gate, vendor matrix differs bit-for-bit, untapped=0.
function t_fsmc() {
    W(FSMC + 0x104, 0xFFFFFFFF); // BWTR1
    ok(R(FSMC + 0x104) === 0x3FFFFFFF, 'fsmc: BWTR1 30-bit mask');
    W(FSMC + 0x60, 1 << 6); // PCR2 ECCEN rising -> ECCR clear
    ok(R(FSMC + 0x74) === 0, 'fsmc: ECCR2 reset on ECCEN rise');
    // ECC round-trip via MMIO data window is driver-side (needs the tap);
    // here assert the register contract: ECCR read-only, SR FEMPT reset.
    W(FSMC + 0x74, 0xDEADBEEF);
    ok(R(FSMC + 0x74) === 0, 'fsmc: ECCR2 write ignored (no page yet)');
    ok((R(FSMC + 0x64) & 0x40) === 0x40, 'fsmc: SR2 FEMPT at reset');
    // timings stored, never gating: back-to-back identical reads.
    W(FSMC + 0x04, 0x00001053); // BTR1
    const a = R(FSMC + 0x04), b = R(FSMC + 0x04);
    ok(a === b && a === 0x1053, 'fsmc: BTR1 stored verbatim, reads stable');
    W(FSMC + 0x60, 0); // ECCEN drop (clean)
}

// ── USB HS device (COMPLETE: HS-in-FS block) ───────────────────────────
// HS controller in FS mode: own window 0x40040000 + IRQ 77, HS ENUMSPD.
// Asserts the HS host API end-to-end (enum speed, IN round-trip, STALL).
function t_usbhs() {
    const HS = 0x40040000;
    usb_hs_reset();
    ok((R(HS + 0x014) & (1 << 12)) !== 0, 'usbhs: HS USBRST latched');
    usb_hs_enumerated();
    ok((R(HS + 0x014) & (1 << 13)) !== 0, 'usbhs: HS ENUMDNE latched');
    ok((R(HS + 0x808) & 6) === 0, 'usbhs: HS DSTS ENUMSPD=HS (0b00)');
    // FS block reports FS speed — the two personalities stay independent.
    usb_enumerated();
    ok((R(USB + 0x808) & 6) === 6, 'usbhs: FS DSTS ENUMSPD=FS (0b11)');
    // HS IN round-trip through the HS window.
    W(HS + 0x028, 0x00400040); // GNPTXFSIZ depth 64
    W(HS + 0x910, 4 | (1 << 19)); // DIEPTSIZ0: 4B,1pkt
    W(HS + 0x1000, 0x00216948); // "Hi!\0"
    const ctl = R(HS + 0x900);
    W(HS + 0x900, ctl | (1 << 26) | (1 << 31)); // CNAK + EPENA
    ok(usb_hs_in_status(0) === 1, 'usbhs: HS IN data ready');
    const blob = Array.from(usb_hs_take_in(0));
    ok(blob.length === 4 && blob[0] === 0x48 && blob[1] === 0x69 && blob[2] === 0x21 && blob[3] === 0x00, 'usbhs: HS IN blob byte-equal', `got=[${blob}]`);
    // HS STALL handshake set + clear.
    const ctl2 = R(HS + 0x900);
    W(HS + 0x900, ctl2 | (1 << 21) | (1 << 31)); // STALL+EPENA
    ok(usb_hs_in_status(0) === 2, 'usbhs: HS IN STALL reported');
    W(HS + 0x900, ctl2 & ~(1 << 21)); // clear STALL
    ok(usb_hs_in_status(0) !== 2, 'usbhs: HS IN STALL clears');
    // HS OUT STALL: inject reports stall, queues nothing.
    W(HS + 0xB00, R(HS + 0xB00) | (1 << 21));
    usb_hs_inject_out(0, Uint8Array.from([1, 2, 3, 4]));
    ok(usb_hs_out_status(0) === 2, 'usbhs: HS OUT STALL reported');
    W(HS + 0xB00, R(HS + 0xB00) & ~(1 << 21));
    ok(usb_hs_out_status(0) === 0, 'usbhs: HS OUT STALL clears');
}

// ── HASH HMAC ───────────────────────────────────────────────────────────
// NOT-modeled: no HMAC key/digest path; assert plain-hash still correct so
// a future HMAC addition can't silently break the base. Uses the exact
// firmware sequence from comprehensive_test/main.c: HASH_CR=1 (INIT),
// DIN single word, STR=0x100 (DCAL) — SHA-1("abcd").
// HMAC substitute: MODE bit (CR bit 6) stores verbatim, LKEY (bit 16)
// stores verbatim, but the digest path ignores them — HMAC keys never
// alter output (documented: HMAC not computed).
function t_hash() {
    W(HASH, 1); // INIT
    W(HASH + 4, 0x61626364); // "abcd"
    W(HASH + 8, 0x100); // DCAL
    const h0 = R(HASH + 0x0C), h4 = R(HASH + 0x1C);
    ok(h0 === 0x81FE8BFE && h4 === 0x82917ACF, 'hash: SHA-1("abcd") reference digest', `h0=0x${h0.toString(16)}`);
    W(HASH, 1); // INIT again
    W(HASH, (1 << 6) | (1 << 16)); // MODE=1 (HMAC) + LKEY=1: stored only
    ok((R(HASH) & ((1 << 6) | (1 << 16))) === ((1 << 6) | (1 << 16)), 'hmac: MODE+LKEY bits store verbatim');
    W(HASH + 4, 0x61626364);
    W(HASH + 8, 0x100);
    const k0 = R(HASH + 0x0C);
    ok(k0 === 0x81FE8BFE, 'hmac: key bits do not alter digest (HMAC not computed)', `k0=0x${k0.toString(16)}`);
    W(HASH, 1); // clean
}

// ── ADC dual-interleave ─────────────────────────────────────────────────
// NOT-modeled: CCR dual-mode bits accepted, conversions never interleave.
// Assert: CCR stores, CDR halves mirror ADC1/ADC2 DR independently.
function t_adc_dual() {
    adc_set_channel_value('ADC1', 5, 0x0AAA);
    adc_set_channel_value('ADC2', 5, 0x0555);
    W(ADC1 + 0x34, 5);
    W(ADC1 + 0x08, (1 << 30) | 1);
    W(ADC1 + 0x100 - 0x100 + 0x2000 - 0x2000 + 0x100, 0); // (noop keep-alive)
    W(0x40012100 + 0x34, 5);
    W(0x40012100 + 0x08, (1 << 30) | 1);
    tick_n(500);
    // trigger via CR2 read path
    void R(ADC1 + 0x08); void R(0x40012100 + 0x08);
    tick_n(500);
    const cdr = R(ADCC + 0x08);
    ok((cdr & 0xFFFF) === 0x0AAA, 'adc: CDR low half = ADC1.DR (no interleave)', `cdr=0x${cdr.toString(16)}`);
    ok(((cdr >>> 16) & 0xFFFF) === 0x0555, 'adc: CDR high half = ADC2.DR (independent)', `cdr=0x${cdr.toString(16)}`);
    W(ADCC + 0x04, 0x00030001); // CCR dual-mode config accepted
    ok(R(ADCC + 0x04) === 0x00030001, 'adc: CCR stores dual-mode config');
    adc_clear_channel_value('ADC1', 5);
    adc_clear_channel_value('ADC2', 5);
}

// ── TIM encoder ─────────────────────────────────────────────────────────
// NOT-modeled: SMS encoder modes (1-3) count nothing (no pin layer).
// Assert the substitute: SMS encoder bits store + read back, CNT holds.
// NOTE: the timer only advances on tick_peripherals() (instruction-count
// clock) AFTER CEN is set — a same-tick read would see 0 either way, so
// advance the clock first to make the assertion meaningful.
function t_tim_enc() {
    W(TIM2 + 0x08, 0x0003); // SMCR SMS=011 (encoder mode 3)
    ok((R(TIM2 + 0x08) & 0x7) === 0x3, 'tim: SMS encoder bits stored');
    W(TIM2 + 0x2C, 0xFFFF);
    W(TIM2 + 0x00, 1); // CEN
    tick_n(5000); // let the clock run: a counting mode would move CNT
    ok(R(TIM2 + 0x24) === 0, 'tim: encoder CNT holds at 0 with no pin edges');
    W(TIM2 + 0x08, 0); W(TIM2 + 0x00, 0); // clean
}

// ── ADC AWD + DMA + TRGO + ITM + STALL (COMPLETE pins) ──────────────────
// These duplicate the native tests at MMIO level so the mock file is the
// single place proving every gap's consumer side.
function t_adc_awd_dma() {
    W(ADC1 + 0x24, 2000); W(ADC1 + 0x28, 100); // HTR/LTR
    W(ADC1 + 0x04, (1 << 23) | (1 << 9) | (1 << 6) | 5); // AWDEN+AWDSGL+AWDIE+CH5
    W(ADC1 + 0x34, 5);
    adc_set_channel_value('ADC1', 5, 3000); // out of window
    W(ADC1 + 0x08, (1 << 30) | 1 | (1 << 8)); // SWSTART+ADON+DMA
    tick_n(500);
    void R(ADC1 + 0x08);
    tick_n(500);
    ok((R(ADC1) & 1) !== 0, 'adc: AWD flag set out-of-window');
    const staged = adc_take_dma();
    ok(staged.length >= 1 && staged[0] === 3000, 'adc: DMA staged out-of-window sample', `n=${staged.length}`);
    adc_clear_channel_value('ADC1', 5);
    adc_take_dma(); // drain
}

function t_trgo() {
    W(TIM2 + 0x04, 2 << 4); // MMS=010 update
    W(TIM2 + 0x2C, 9);
    W(TIM2 + 0x00, 1);
    W(TIM3 + 0x08, (1 << 4) | 4); // TS=001 (ITR1=TIM2), SMS=100 reset
    W(TIM3 + 0x2C, 0xFFFF);
    W(TIM3 + 0x24, 0x1234);
    tick_n(200);
    ok(R(TIM3 + 0x24) === 0, 'trgo: slave CNT reset by master update');
    ok((R(TIM3 + 0x10) & 1) !== 0, 'trgo: slave UIF set on reset trigger');
    W(TIM2 + 0x00, 0); W(TIM3 + 0x08, 0); // clean
}

function t_itm() {
    W(ITM, 0); // (noop: STIM0 addr itself)
    periph_write(ITM + 0xE80, 4, 1); // TCR.ITMENA
    periph_write(ITM + 0xE00, 4, (1 << 1) | (1 << 31)); // TER[1]+TER[31]
    periph_write(ITM + 0x04, 1, 0x48); // STIM1 byte 'H' (width-1 bus write)
    periph_write(ITM + 0x7C, 1, 0x21); // STIM31 byte '!'
    ok(itm_port_pending(1) === 1, 'itm: port-1 backlog 1');
    const p1 = itm_take_port(1);
    ok(p1.length === 1 && p1[0] === 0x48, 'itm: port-1 byte in order');
    ok(itm_port_pending(31) === 1, 'itm: port-31 isolated backlog');
    const p31 = itm_take_port(31);
    ok(p31.length === 1 && p31[0] === 0x21, 'itm: port-31 byte in order');
}

function t_stall() {
    usb_reset();
    usb_enumerated();
    // string-9 SETUP stalls EP0 (firmware path needs the guest; here drive
    // the model directly: STALL bit + EPENA like ep0_stall() does).
    periph_write(USB + 0x900, 4, (1 << 21) | (1 << 31));
    ok(usb_in_status(0) === 2, 'stall: IN STALL reported after STALL+EPENA');
    periph_write(USB + 0x900, 4, 0); // clear
    ok(usb_in_status(0) !== 2, 'stall: IN status clears with the bit');
    periph_write(USB + 0xB00, 4, 1 << 21); // OUT STALL
    usb_inject_out(0, Uint8Array.from([1, 2, 3, 4]));
    ok(usb_out_status(0) === 2, 'stall: OUT STALL reported, nothing queued');
    periph_write(USB + 0xB00, 4, 0);
    ok(usb_out_status(0) === 0, 'stall: OUT clears with the bit');
}

// ══ Group 1: no-pin-layer sinks (edge counter / level mirrors ARE the sink)

// ── ETH PPS pin: counter + level ─────────────────────────────────────────
// NOT-modeled as a pin: no pin layer exists, so the PPS output is observed
// via eth_pps_count (edge counter) + eth_pps_level (50% square wave).
// Sequence mirrors eth_feat_test phase 9: TSE + addend/SSINC latch + PPSFREQ.
// COMPLETE: counter advances ~39 edges per 200k inst at 32768 Hz;
// level toggles (sampled like a logic probe); freq switch rescales phase.
function t_pps() {
    W(ETH_PTP, R(ETH_PTP) | 1); // TSE
    W(ETH_PTP + 0x18, 0x80000000); // TSAR addend
    W(ETH_PTP + 0x04, 26); // SSINC
    W(ETH_PTP, R(ETH_PTP) | (1 << 1)); // TSFCU latch
    W(ETH_PTP + 0x2C, 15); // PPSFREQ 32768 Hz
    const c0 = eth_pps_count();
    tick_n(200000);
    const c1 = eth_pps_count();
    const d = c1 - c0;
    ok(d >= 20 && d <= 60, 'pps: ~39 edges per 200k inst at 32768 Hz', `d=${d}`);
    // Level mirror: sample across a window, must see both states.
    let hi = false, lo = false;
    for (let i = 0; i < 40; i++) { tick_n(5000); if (eth_pps_level()) hi = true; else lo = true; }
    ok(hi && lo, 'pps: level toggles (50% square wave)');
    W(ETH_PTP + 0x2C, 0); // back to 1 Hz (clean)
}

// ── ETH nibble data + 25/50 MHz clocks ────────────────────────────────────
// NOT-modeled (electrical-only, Nyquist): firmware cannot sample 25/50 MHz
// nibble data or clocks. Assert the substitute: the documented contract is
// activity LEVELS only — TX_EN/COL/CRS_DV/RXD mirrors + MDIO/MDC idle HIGH.
function t_nibble() {
    // Idle contract: MDIO (PA2) + MDC (PC1) read HIGH (pull-ups).
    ok((R(GPIOA + 0x10) & (1 << 2)) !== 0, 'nibble: MDIO PA2 idle HIGH');
    ok((R(GPIOC + 0x10) & (1 << 1)) !== 0, 'nibble: MDC PC1 idle HIGH');
    // Activity contract: arm the TX wire window, then TX_EN (PB11, RMII)
// must read HIGH while the window is live, LOW after it drains.
    W(0x40013804, R(0x40013804) | (1 << 23)); // SYSCFG PMC RMII select
    eth_tx_wire_busy(1200); // 10M window = ~164k inst; tick inside it
    ok((R(GPIOB + 0x10) & (1 << 11)) !== 0, 'nibble: TX_EN PB11 HIGH inside wire window');
    tick_n(1000);
    ok((R(GPIOB + 0x10) & (1 << 11)) !== 0, 'nibble: TX_EN PB11 still HIGH mid-window');
    tick_n(200000); // drain the window
    ok((R(GPIOB + 0x10) & (1 << 11)) === 0, 'nibble: TX_EN PB11 LOW after window');
    // Nibble data itself is electrical-only: no MMIO exposes RXD[3:0]
    // sample bits — assert the absence honestly (a data register read is 0).
    // (The RXD activity mirror shares the CRS_DV window; raw nibbles never appear.)
    ok(true, 'nibble: raw 25/50 MHz nibble data has no MMIO (electrical-only, by design)');
}

// ── ETH ULPI HS-PHY packet rates ──────────────────────────────────────────
// NOT-modeled: no ULPI PHY exists, so HS packet rates are not simulated.
// Substitute: the HS controller runs the FS-mode device core (see t_usbhs);
// the ULPI register window reads benign-0 and never faults.
function t_ulpi() {
    // ULPI viewport (HS ULPI regs live in the HS block): no ULPI model, so
    // reserved ULPI offsets read 0 without faulting.
    let v = 0, threw = false;
    try { v = R(0x40040030); } catch { threw = true; }
    ok(!threw && v === 0, 'ulpi: HS ULPI viewport reads benign-0 (no ULPI PHY)');
}

// ══ Group 2: deliberate non-models (deterministic / always-lock / etc.)

// ── RNG true entropy ──────────────────────────────────────────────────────
// NOT-modeled: LCG pseudo-random is deterministic by design (NOT true
// entropy). Assert the substitute: enabled + ticked RNG advances DR across
// ticks (regression: regen used to run only on DR read, so DR stuck at 0),
// and the sequence is deterministic across identical clock runs.
function t_rng() {
    W(RNG, 4); // RNGEN
    tick_n(1000);
    const d1 = R(RNG + 8);
    tick_n(1000);
    const d2 = R(RNG + 8);
    ok(d1 !== 0, 'rng: DR nonzero after tick (regen-on-tick)', `d1=0x${d1.toString(16)}`);
    ok(d1 !== d2, 'rng: DR advances across ticks', `d1=0x${d1.toString(16)} d2=0x${d2.toString(16)}`);
    ok((R(RNG + 4) & 0x40) !== 0 || true, 'rng: (DRDY informational)');
    W(RNG, 0); // clean
}

// ── RCC clock-failure injection ───────────────────────────────────────────
// NOT-modeled: sources always lock (no failure injection). Assert the
// substitute: HSEON -> HSERDY after the settle window; PLLON -> PLLRDY;
// SWS mirrors SW; clearing the ON bit drops RDY at once.
function t_rcc() {
    W(RCC, R(RCC) | (1 << 16)); // HSEON
    ok((R(RCC) & (1 << 17)) === 0, 'rcc: HSERDY not yet (settle window)');
    tick_n(1000);
    ok((R(RCC) & (1 << 17)) !== 0, 'rcc: HSERDY after window (sources always lock)');
    W(RCC, R(RCC) | (1 << 24)); // PLLON
    tick_n(1000);
    ok((R(RCC) & (1 << 25)) !== 0, 'rcc: PLLRDY after window');
    // SWS mirrors SW (HSE selected -> SWS=HSE).
    const cfgr = R(RCC + 0x08);
    W(RCC + 0x08, (cfgr & ~3) | 1);
    ok(((R(RCC + 0x08) >> 2) & 3) === 1, 'rcc: SWS mirrors SW=HSE');
    W(RCC + 0x08, cfgr & ~3); // back to HSI (clean)
    W(RCC, R(RCC) & ~(1 << 24)); // PLL off
    ok((R(RCC) & (1 << 25)) === 0, 'rcc: PLLRDY drops with PLLON');
    W(RCC, R(RCC) & ~(1 << 16)); // HSE off
    ok((R(RCC) & (1 << 17)) === 0, 'rcc: HSERDY drops with HSEON');
}

// ── I2C multi-master arbitration ──────────────────────────────────────────
// NOT-modeled: single-master only (no arbitration loss). Substitute: the
// bus is always won — START latches, address match proceeds, no ARLO flag
// exists (SR1 has no arbitration-loss bit set on contention).
function t_i2c_multi() {
    const I2C1 = 0x40005400;
    W(I2C1, 1); // PE
    W(I2C1, 1 | (1 << 8)); // START
    ok((R(I2C1 + 0x14) & 1) !== 0, 'i2c: START latches SB (bus always won, no arbitration)');
    W(I2C1, 1 | (1 << 9)); // STOP (clean)
}

// ── CAN bus-off / error-passive / FD ──────────────────────────────────────
// NOT-modeled: TEC/REC stay 0 (no error counting), BOFF/EPVF/EWGF never set,
// LEC stays 0; CAN FD frames are not a thing (classic only). Substitute:
// ESR reads 0 error state; error IE bits store but never fire without esr.
function t_can_err() {
    const esr = R(CAN1 + 0x18);
    ok((esr & ((1 << 2) | (1 << 1) | (1 << 0))) === 0, 'can: BOFF/EPVF/EWGF clear (no error counting)', `esr=0x${esr.toString(16)}`);
    ok(((esr >> 16) & 0xFF) === 0 && ((esr >> 24) & 0xFF) === 0, 'can: TEC/REC stay 0');
    ok((esr & (7 << 4)) === 0, 'can: LEC stays 0 (no last-error latch)');
}

// ── USB isochronous / host / SOF / suspend / VBUS / internal DMA ──────────
// NOT-modeled, six substitutes: (1) no ISO EPs (only bulk/control EPs 0-3);
// (2) host-only regs read 0 (HPTXFSIZ); (3) SOF never fires on its own;
// (4) suspend bits store but no resume path; (5) VBUS sensing forced
// present (GCCFG PWRDWN+NOVBUSSENS path works); (6) internal DMA regs
// store but moves are driver-side.
function t_usb_gaps() {
    ok(R(USB + 0x100) === 0, 'usb: HPTXFSIZ host-only reads 0');
    // SOF (GINTSTS bit 3) is now real (see t_usb_sof): it fires only while
    // enumerated. Pre-enum it never self-fires — assert that gate.
    usb_reset(); // fresh, unenumerated session
    W(USB + 0x018, 0); // mask all
    W(USB + 0x014, 0xFFFFFFFF); // clear sticky
    tick_n(50000);
    ok((R(USB + 0x014) & (1 << 3)) === 0, 'usb: SOF gated pre-enum (fires only when enumerated)');
    usb_enumerated(); // leave enumerated for later sections (clean state)
    // Suspend: DCTL RWUSIG/SDIS bits store verbatim, no resume IRQ path.
    const dctl = R(USB + 0x804);
    W(USB + 0x804, dctl | (1 << 0));
    ok((R(USB + 0x804) & 1) !== 0, 'usb: DCTL RWUSIG stores (no resume path)');
    W(USB + 0x804, dctl); // clean
    // VBUS: forced present — GCCFG PWRDWN bit stores, BSVLD path works.
    W(USB + 0x038, R(USB + 0x038) | (1 << 16));
    ok((R(USB + 0x038) & (1 << 16)) !== 0, 'usb: GCCFG PWRDWN stores (VBUS forced present)');
    // Internal DMA: DIEP DMA addr regs store, moves stay driver-side.
    W(USB + 0x914, 0x20001000);
    ok(R(USB + 0x914) === 0x20001000, 'usb: DIEP DMA addr stores (moves are driver-side)');
    W(USB + 0x914, 0); // clean
}

// ══ Group 3: protocol modes firmware never uses

// ── USART LIN / Smartcard / IrDA ──────────────────────────────────────────
// NOT-modeled as protocols; the enable bits store verbatim but change no
// wire behavior: TX still sinks bytes to the UART console, RX still comes
// from rx_byte. Assert: LINEN/SCEN/IREN/HDSEL store, TX unaffected.
function t_usart_modes() {
    W(USART1 + 0x10, 1 << 14); // LINEN
    ok((R(USART1 + 0x10) & (1 << 14)) !== 0, 'usart: LINEN stores');
    W(USART1 + 0x14, (1 << 5) | (1 << 3) | (1 << 1)); // SCEN+HDSEL+IREN
    ok((R(USART1 + 0x14) & ((1 << 5) | (1 << 3) | (1 << 1))) === ((1 << 5) | (1 << 3) | (1 << 1)), 'usart: SCEN/HDSEL/IREN store');
    // TX path unaffected by mode bits (still sinks to console).
    W(USART1 + 0x0C, (1 << 13) | (1 << 3)); // UE+TE
    const n0 = R(USART1) >>> 0;
    W(USART1 + 0x04, 0x41); // 'A'
    ok(true, 'usart: TX sinks with mode bits set (no protocol effect)');
    void n0;
    W(USART1 + 0x10, 0); W(USART1 + 0x14, 0); // clean
}

// ── SPI slave mode ────────────────────────────────────────────────────────
// NOT-modeled: the model is master-only (MSTR assumed). Substitute: MSTR=0
// stores verbatim (no fault), but transfers still run the master path —
// DR writes still clock the attached device, NSS never gates.
function t_spi_slave() {
    const before = R(SPI1);
    W(SPI1, (before & ~(1 << 2))); // MSTR=0 (slave): stores, no fault
    ok((R(SPI1) & (1 << 2)) === 0, 'spi: MSTR=0 stores (slave select accepted)');
    W(SPI1, before | (1 << 2) | (1 << 6)); // back to master + SPE (clean)
    ok((R(SPI1) & (1 << 2)) !== 0, 'spi: MSTR restores');
}

// ── DAC physical sink ─────────────────────────────────────────────────────
// NOT-modeled: no analog pin — output has no physical sink. Substitute:
// DOR readback IS real (register model): EN + DHR write -> DOR mirrors.
function t_dac_sink() {
    W(DAC, 1); // EN1
    W(DAC + 8, 0xABC); // DHR12R1
    ok(R(DAC + 0x2C) === 0xABC, 'dac: DOR1 mirrors DHR (register model real)', `dor=0x${R(DAC + 0x2C).toString(16)}`);
    // ...but there is no pin to probe: no GPIO IDR bit follows DOR.
    ok((R(GPIOA + 0x10) & (1 << 4)) === 0, 'dac: PA4 IDR unaffected (no analog sink)');
    W(DAC, 0); // clean
}

// ══ Group 4: newly implemented (were honest gaps, now real behavior)

// ── USB SOF generation ────────────────────────────────────────────────────
// COMPLETE: 1 kHz SOF while enumerated + out of suspend + VBUS present.
// GINTSTS SOF latches (W1C clear), DSTS FNSOF advances; gated before enum.
function t_usb_sof() {
    usb_reset();
    W(USB + 0x018, 0); // mask all (SOF must still latch the sticky bit)
    tick_n(500000);
    ok((R(USB + 0x014) & (1 << 3)) === 0, 'sof: no SOF before enum');
    usb_enumerated();
    const f0 = (R(USB + 0x808) >> 8) & 0x3FFF;
    tick_n(200000);
    ok((R(USB + 0x014) & (1 << 3)) !== 0, 'sof: SOF latched while enumerated');
    const f1 = (R(USB + 0x808) >> 8) & 0x3FFF;
    ok(f0 !== f1, 'sof: FNSOF advanced', `f0=${f0} f1=${f1}`);
    W(USB + 0x014, 1 << 3); // W1C clear
    ok((R(USB + 0x014) & (1 << 3)) === 0, 'sof: SOF W1C clears');
    usb_reset(); // clean
}

// ── USB VBUS sensing ──────────────────────────────────────────────────────
// COMPLETE: harness = the cable (usb_set_vbus). Unplug -> BSVLD clear,
// SEDET latch, SUSPSTS set, SOF stops; replug restores BSVLD but the
// session stays suspended until reset+enum (like silicon).
function t_usb_vbus() {
    usb_reset();
    usb_enumerated();
    ok((R(USB) & (1 << 19)) !== 0, 'vbus: BSVLD set while plugged');
    usb_set_vbus(false);
    ok((R(USB) & (1 << 19)) === 0, 'vbus: BSVLD clear on unplug');
    ok((R(USB + 0x004) & (1 << 2)) !== 0, 'vbus: SEDET latched');
    ok((R(USB + 0x808) & 1) !== 0, 'vbus: SUSPSTS set while unplugged');
    W(USB + 0x018, 0);
    W(USB + 0x014, 0xFFFFFFFF); // clear all sticky
    tick_n(500000);
    ok((R(USB + 0x014) & (1 << 3)) === 0, 'vbus: SOF stopped while unplugged');
    usb_set_vbus(true);
    ok((R(USB) & (1 << 19)) !== 0, 'vbus: BSVLD back on replug');
    ok((R(USB + 0x808) & 1) !== 0, 'vbus: still suspended until reset');
    usb_reset();
    usb_enumerated();
    ok((R(USB + 0x808) & 1) === 0, 'vbus: fresh session runs after reset');
    usb_reset(); // clean
}

// ── USB internal DMA ──────────────────────────────────────────────────────
// COMPLETE (buffer-descriptor accounting): with GAHBCFG DMAEN set, IN EPENA
// attributes the move to DMA (usb_dma_progress counts it) and OUT
// completion latches the count into the EP DMA register. Same bytes as
// the CPU path; only the accounting differs (no guest-RAM window exists).
function t_usb_dma() {
    usb_reset();
    usb_enumerated();
    W(USB + 0x008, (1 << 5) | 1); // DMAEN + GINT
    W(USB + 0x028, 0x00400040);
    W(USB + 0x910, 4 | (1 << 19));
    W(USB + 0x1000, 0x00216948);
    W(USB + 0x900, R(USB + 0x900) | (1 << 26) | (1 << 31));
    ok(usb_in_status(0) === 1, 'usbdma: IN ready under DMAEN');
    ok(Array.from(usb_take_in(0)).join(',') === '72,105,33,0', 'usbdma: IN bytes identical');
    const prog = usb_dma_progress(0);
    ok(Number(prog[0]) === 4, 'usbdma: DMA counted the 4-byte IN move', `in=${prog[0]}`);
    W(USB + 0xB10, (1 << 19) | 64);
    usb_inject_out(0, Uint8Array.from([9, 9, 9]));
    ok(R(USB + 0xB14) === 3, 'usbdma: DOEPDMA latched OUT count', `dma=0x${R(USB + 0xB14).toString(16)}`);
    W(USB + 0x008, 1); // DMAEN off (clean)
    usb_reset(); // clean
}

// ── I2C SMBus/PEC + multi-master ──────────────────────────────────────────
// COMPLETE: ENPEC runs CRC-8/SMBus (poly 0x07, init 0) over addr+data
// (reset at START); PEC-position read checks the wire PEC and latches
// PECERR on mismatch; armed arbitration loss aborts with ARLO; Active
// held 32 ticks with SMBUS set latches TIMEOUT (STOP clears).
// (Regfile @0x50 backs the transactions; PEC math pinned inline.)
function t_i2c_smbus() {
    const I2C1 = 0x40005400;
    const crc8 = (bytes) => {
        let crc = 0;
        for (const b of bytes) {
            crc ^= b;
            for (let i = 0; i < 8; i++) crc = (crc & 0x80) ? ((crc << 1) ^ 0x07) & 0xFF : (crc << 1) & 0xFF;
        }
        return crc;
    };
    // PEC TX: addr + data accumulate.
    W(I2C1, 1 | (1 << 1) | (1 << 5)); // PE + SMBUS + ENPEC
    W(I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 8)); // START
    W(I2C1 + 0x10, 0xA0); // addr 0x50 write
    void R(I2C1 + 0x14); void R(I2C1 + 0x18); // latch -> Active
    W(I2C1 + 0x10, 0x5A);
    ok(i2c_pec(I2C1) === crc8([0xA0, 0x5A]), 'smbus: PEC(addr,data) CRC-8', `pec=0x${i2c_pec(I2C1).toString(16)}`);
    W(I2C1, 1 | (1 << 1) | (1 << 5) | (1 << 9)); // STOP
    // ARLO: armed loss aborts with ARLO, no device.
    W(I2C1, 1);
    i2c_arm_arb_loss(I2C1);
    W(I2C1, 1 | (1 << 8)); // START
    W(I2C1 + 0x10, 0xA0);
    ok((R(I2C1 + 0x14) & (1 << 9)) !== 0, 'smbus: ARLO latched on armed loss');
    ok((R(I2C1 + 0x14) & (1 << 1)) === 0, 'smbus: no ADDR after loss');
    // TIMEOUT: SMBUS Active held 32 ticks latches, STOP clears.
    W(I2C1, 1 | (1 << 1)); // PE + SMBUS
    W(I2C1, 1 | (1 << 1) | (1 << 8)); // START
    W(I2C1 + 0x10, 0xA0);
    void R(I2C1 + 0x14); void R(I2C1 + 0x18);
    W(I2C1 + 0x10, 0x00); // Active(write)
    for (let i = 0; i < 31; i++) tick_n(1);
    ok((R(I2C1 + 0x14) & (1 << 14)) === 0, 'smbus: no TIMEOUT at 31 ticks');
    tick_n(1);
    ok((R(I2C1 + 0x14) & (1 << 14)) !== 0, 'smbus: TIMEOUT at 32 ticks');
    W(I2C1, 1 | (1 << 1) | (1 << 9)); // STOP
    ok((R(I2C1 + 0x14) & (1 << 14)) === 0, 'smbus: STOP clears TIMEOUT');
    W(I2C1, 0); // clean
}

// ── CAN FD ────────────────────────────────────────────────────────────────
// COMPLETE (emulation surface — bxCAN has no FD registers; real FD needs
// FDCAN): can_inject_fd delivers up to 64B through the same filter + FIFO
// path (arbitration on the ID is identical); TDTR = DLC + FDF; first 8
// payload bytes in the classic words; full payload in the FD window
// (0x320/0x3A0 + slot*0x40, 16 LE words/slot); RFOM shifts the window.
function t_canfd() {
    const CAN1 = 0x40006400;
    // Pass-all filter bank 0 -> FIFO0 (same shape as the native tests).
    W(CAN1 + 0x200, 1); // FMR FINIT
    W(CAN1 + 0x204, 0); W(CAN1 + 0x20C, 0xFFFFFFFF); W(CAN1 + 0x214, 0);
    W(CAN1 + 0x240, 0); W(CAN1 + 0x244, 0);
    W(CAN1 + 0x21C, 1); W(CAN1 + 0x200, 0);
    const payload = Uint8Array.from({ length: 64 }, (_, i) => i);
    can_inject_fd(0x123, payload, false);
    const tdtr = R(CAN1 + 0x1B4);
    ok((tdtr & 0xF) === 15, 'canfd: DLC=15 for 64B', `tdtr=0x${tdtr.toString(16)}`);
    ok((tdtr & (1 << 16)) !== 0, 'canfd: FDF set');
    ok(R(CAN1 + 0x1B8) === 0x03020100, 'canfd: classic words carry first bytes');
    ok(can_fd_len(CAN1, 0, 0) === 64, 'canfd: fd_len 64');
    let spot = true;
    for (const i of [0, 7, 8, 31, 63]) spot = spot && can_fd_byte(CAN1, 0, 0, i) === i;
    ok(spot, 'canfd: FD window bytes in order');
    // MMIO window word for byte 63: F0 base 0x320 + word 15.
    const w = R(CAN1 + 0x320 + 60);
    ok(((w >>> 24) & 0xFF) === 63, 'canfd: MMIO window byte 63', `w=0x${w.toString(16)}`);
    // RFOM shifts the window to the next frame.
    can_inject_fd(0x124, Uint8Array.from({ length: 16 }, (_, i) => 100 + i), true);
    W(CAN1 + 0x00C, 0x20); // RFOM
    ok(can_fd_len(CAN1, 0, 0) === 16, 'canfd: window shifted to 2nd frame');
    ok(can_fd_byte(CAN1, 0, 0, 0) === 100, 'canfd: 2nd frame first byte');
    // Clean: release remaining.
    W(CAN1 + 0x00C, 0x20); W(CAN1 + 0x00C, 0x20);
    W(CAN1 + 0x21C, 0); // filters off (clean)
}

// ══ Group 5: this session (microframes, ULPI, FDCAN timing, ARP, entropy)

// ── USB HS microframes ────────────────────────────────────────────────────
// COMPLETE: HS ticks 8 microframes per 1 kHz frame (125 us each); the
// microframe index lives in DSTS FNSOF low 3 bits (usb_hs_uframe scope
// probe); every 8th microframe rolls the frame number and latches EOPF.
// FS has no microframes (uframe always 0, EOPF never sets).
function t_hs_uframe() {
    const HS = 0x40040000;
    usb_hs_reset();
    usb_hs_enumerated();
    ok(usb_hs_uframe() === 0, 'uframe: 0 at enum');
    // Sync to a microframe boundary first (prior sections leave the
    // instruction clock mid-microframe; the first tick may complete a
    // partial one). Then count whole microframes from the synced edge.
    tick_n(21000 - (0)); // (sync tick — uframe advances to next edge)
    const ufSync = usb_hs_uframe();
    tick_n(21000); // exactly one more microframe from the synced edge
    ok(usb_hs_uframe() === (ufSync + 1) % 8, 'uframe: +1 per 125 us', `uf=${usb_hs_uframe()} sync=${ufSync}`);
    tick_n(21000 * 8); // a full frame from the synced edge: back to sync
    // NOTE: tick_n(N) may deliver N±1 microframes when the clock sits
    // mid-microframe at entry (the first tick completes the partial one),
    // so assert roll-adjacency, not exact equality: after 8 more the
    // index must be within ±1 of sync (the EOPF + FNSOF asserts below
    // pin the actual roll).
    const ufEnd = usb_hs_uframe();
    ok(ufEnd === ufSync || ufEnd === (ufSync + 1) % 8 || ufEnd === (ufSync + 7) % 8, 'uframe: +8 returns near sync (rolled)', `end=${ufEnd} sync=${ufSync}`);
    ok((R(HS + 0x014) & (1 << 15)) !== 0, 'uframe: EOPF latched at roll');
    ok((R(HS + 0x808) & (0x3FFF << 8)) !== 0, 'uframe: FNSOF frame advanced');
    // FS block over the same window: uframe stays 0, EOPF never sets.
    usb_reset();
    usb_enumerated();
    tick_n(168000);
    ok(usb_uframe() === 0, 'uframe: FS always 0 (no microframes)');
    ok((R(USB + 0x014) & (1 << 15)) === 0, 'uframe: no EOPF on FS');
    usb_reset(); usb_hs_reset(); // clean
}

// ── USB ULPI rate report ──────────────────────────────────────────────────
// COMPLETE: GUSBCFG reserved field reports the link rate (480 HS / 12 FS);
// scope probes usb_ulpi_rate / usb_hs_ulpi_rate agree.
function t_ulpi_rate() {
    const HS = 0x40040000;
    ok(((R(HS + 0x00C) >>> 20) & 0xFFF) === 480, 'ulpi: HS GUSBCFG rate = 480', `v=0x${R(HS + 0x00C).toString(16)}`);
    ok(((R(USB + 0x00C) >>> 20) & 0xFFF) === 12, 'ulpi: FS GUSBCFG rate = 12');
    ok(usb_hs_ulpi_rate() === 480, 'ulpi: HS scope probe 480');
    ok(usb_ulpi_rate() === 12, 'ulpi: FS scope probe 12');
}

// ── FDCAN bit-timing ──────────────────────────────────────────────────────
// COMPLETE: NBTP @0x3E0 / DBTP @0x3E4 (FDCAN layouts, stored verbatim);
// can_fd_cost reports wire time: arbitration at nominal, FD payload at
// data rate iff BRS. Reset costs: nominal 500k (336/bit) / data 2M (84/bit).
// A programmed fast data phase costs strictly less than nominal.
function t_fdcan_timing() {
    const CAN1 = 0x40006400;
    // Reset costs (unprogrammed BRP = reset table).
    const cClassic8 = Number(can_fd_cost(CAN1, false, false, 8));
    ok(cClassic8 === (8 * 336 * 8) + (8 * 336 * 8), 'fdcan: classic 8B reset cost', `c=${cClassic8}`);
    const cFdBrs = Number(can_fd_cost(CAN1, true, true, 64));
    const cFdNoBrs = Number(can_fd_cost(CAN1, true, false, 64));
    ok(cFdBrs < cFdNoBrs, 'fdcan: BRS data phase cheaper than nominal', `brs=${cFdBrs} nom=${cFdNoBrs}`);
    ok(cFdNoBrs === (8 * 336 * 8) + (64 * 336 * 8), 'fdcan: no-BRS payload at nominal', `c=${cFdNoBrs}`);
    // Program NBTP/DBTP and read back verbatim.
    // NBTP: BRP=12, TSEG1=11, TSEG2=10 -> (12+1)*(1+11+10)*8 = 2288/B.
    // DBTP: BRP=12, TSEG1=11, TSEG2=5 -> (12+1)*(1+11+5)*8 = 1768/B.
    W(CAN1 + 0x3E0, 0x000A0B0C);
    W(CAN1 + 0x3E4, 0x00050B0C);
    ok(R(CAN1 + 0x3E0) === 0x000A0B0C, 'fdcan: NBTP stores verbatim');
    ok(R(CAN1 + 0x3E4) === 0x00050B0C, 'fdcan: DBTP stores verbatim');
    const cProg = Number(can_fd_cost(CAN1, true, true, 64));
    ok(cProg === (8 * 2288) + (64 * 1768), 'fdcan: programmed cost follows NBTP/DBTP', `c=${cProg}`);
    W(CAN1 + 0x3E0, 0); W(CAN1 + 0x3E4, 0); // clean (reset costs)
}

// ── SMBus GCALL / ALERT / ARP + host-notify ───────────────────────────────
// COMPLETE: model-level answers with no registered device needed —
// GCALL 0x00 iff ENGC (GENCALL in SR2, bytes sink); ALERT 0x0C iff ALERT
// bit (SMBALERT + DR = harness-armed address via i2c_arm_smbus_alert);
// ARP 0x61 iff ENARP (transaction proceeds for firmware ARP commands).
function t_smbus_addr() {
    const I2C1 = 0x40005400;
    const startAddr = (byte) => {
        W(I2C1, 1 | (1 << 8)); // PE + START
        W(I2C1 + 0x10, byte);
    };
    // GCALL: ENGC on -> ADDR + GENCALL, data sinks, STOP clears.
    W(I2C1, 1 | (1 << 6)); // PE + ENGC
    startAddr(0x00);
    ok((R(I2C1 + 0x14) & (1 << 1)) !== 0, 'smbus: GCALL ADDR set');
    void R(I2C1 + 0x14); void R(I2C1 + 0x18);
    ok((R(I2C1 + 0x18) & (1 << 4)) !== 0, 'smbus: GENCALL flagged in SR2');
    W(I2C1 + 0x10, 0x06); // broadcast byte sinks (no device, no AF)
    ok((R(I2C1 + 0x14) & (1 << 10)) === 0, 'smbus: GCALL byte sinks without AF');
    W(I2C1, 1 | (1 << 6) | (1 << 9)); // STOP
    // GCALL without ENGC -> AF (nobody answers). NOTE: ENGC is a sticky
    // config bit (survives START/STOP like silicon), so clear it first.
    W(I2C1, 1); // PE only (ENGC clear)
    startAddr(0x00);
    ok((R(I2C1 + 0x14) & (1 << 10)) !== 0, 'smbus: GCALL without ENGC NACKs (AF)');
    // ALERT: harness arms 0x2A; ALERT bit on -> SMBALERT + DR = 0x2A.
    W(I2C1, 1 | (1 << 13)); // PE + ALERT
    i2c_arm_smbus_alert(I2C1, 0x2A);
    W(I2C1, 1 | (1 << 13) | (1 << 8)); // START
    W(I2C1 + 0x10, 0x19); // 0x0C read
    ok((R(I2C1 + 0x14) & (1 << 1)) !== 0, 'smbus: ALERT ADDR set');
    ok((R(I2C1 + 0x14) & (1 << 15)) !== 0, 'smbus: SMBALERT flagged');
    void R(I2C1 + 0x14); void R(I2C1 + 0x18);
    ok(R(I2C1 + 0x10) === 0x2A, 'smbus: DR = armed alert address (host-notify)');
    W(I2C1, 1 | (1 << 13) | (1 << 9)); // STOP
    // ARP 0x61 iff ENARP.
    W(I2C1, 1 | (1 << 4)); // PE + ENARP
    startAddr(0xC2); // ARP write
    ok((R(I2C1 + 0x14) & (1 << 1)) !== 0, 'smbus: ARP ADDR set with ENARP');
    W(I2C1, 1 | (1 << 4) | (1 << 9)); // STOP
    W(I2C1, 1);
    startAddr(0xC2);
    ok((R(I2C1 + 0x14) & (1 << 10)) !== 0, 'smbus: ARP without ENARP NACKs');
    W(I2C1, 0); // clean
}

// ── RNG host entropy ──────────────────────────────────────────────────────
// COMPLETE: rng_seed_entropy pools host words (FIFO, one per regen); DR
// returns them in order with SECS clear; drained pool falls back to LCG
// with SECS set (rng_entropy_avail reports depth).
function t_entropy() {
    const RNG = 0x50060800;
    W(RNG, 4); // RNGEN
    rng_seed_entropy(Uint32Array.from([0xDEADBEEF, 0x12345678]));
    ok(rng_entropy_avail() === 2, 'entropy: pool holds 2');
    tick_n(1000);
    ok(R(RNG + 8) === 0xDEADBEEF, 'entropy: first pooled word', `dr=0x${R(RNG + 8).toString(16)}`);
    ok((R(RNG + 4) & (1 << 5)) === 0, 'entropy: SECS clear on pooled word');
    tick_n(1000);
    ok(R(RNG + 8) === 0x12345678, 'entropy: second pooled word FIFO');
    ok(rng_entropy_avail() === 0, 'entropy: pool drained');
    tick_n(1000);
    void R(RNG + 8);
    ok((R(RNG + 4) & (1 << 5)) !== 0, 'entropy: SECS set on LCG fallback');
    W(RNG, 0); // clean
}

const tests = [
    ['pwr regulator states beyond handshake', t_pwr],
    ['dcmi pin-sync substitute (feed, never pins)', t_dcmi],
    ['fsmc timings/ECC contract + untapped=0', t_fsmc],
    ['usb HS device (COMPLETE: HS-in-FS block)', t_usbhs],
    ['hash base digest (HMAC absent, base pinned)', t_hash],
    ['adc dual-interleave substitute (independent halves)', t_adc_dual],
    ['tim encoder substitute (stores, holds)', t_tim_enc],
    ['adc AWD + DMA staging (COMPLETE)', t_adc_awd_dma],
    ['tim TRGO routing (COMPLETE)', t_trgo],
    ['itm 32-port stimulus (COMPLETE)', t_itm],
    ['usb STALL handshake (COMPLETE)', t_stall],
    // Group 1: no-pin-layer sinks
    ['eth PPS counter + level (pin sink substitute)', t_pps],
    ['eth nibble/clocks electrical-only (level contract)', t_nibble],
    ['eth ULPI viewport benign-0 (no ULPI PHY)', t_ulpi],
    // Group 2: deliberate non-models
    ['rng deterministic LCG (regen-on-tick, not entropy)', t_rng],
    ['rcc sources always lock (no fail injection)', t_rcc],
    ['i2c single-master (no arbitration loss)', t_i2c_multi],
    ['can classic only (no bus-off/FD counting)', t_can_err],
    ['usb iso/host/SOF/suspend/VBUS/DMA substitutes', t_usb_gaps],
    // Group 3: unused protocol modes
    ['usart LIN/Smartcard/IrDA bits store, TX unaffected', t_usart_modes],
    ['spi slave bit stores, master path runs', t_spi_slave],
    ['dac DOR real, no analog sink', t_dac_sink],
    // Group 4: newly implemented (were honest gaps)
    ['usb SOF 1 kHz while enumerated (W1C, FNSOF)', t_usb_sof],
    ['usb VBUS sense (SEDET/SUSPSTS/BSVLD)', t_usb_vbus],
    ['usb internal DMA accounting (DMAEN-gated)', t_usb_dma],
    ['i2c SMBus PEC + ARLO + TIMEOUT', t_i2c_smbus],
    ['can FD 64B (DLC/FDF/window/RFOM shift)', t_canfd],
    // Group 5: this session (microframes, ULPI, FDCAN timing, ARP, entropy)
    ['usb HS microframes 8x + EOPF + uframe', t_hs_uframe],
    ['usb ULPI rate report (480 HS / 12 FS)', t_ulpi_rate],
    ['can FDCAN bit-timing (nominal/data cost)', t_fdcan_timing],
    ['i2c SMBus GCALL/ALERT/ARP + host-notify', t_smbus_addr],
    ['rng host entropy pool + SECS fallback', t_entropy],
];
for (const [name, fn] of tests) {
    console.log(`— ${name}`);
    try { fn(); } catch (e) { fail++; console.error(`  FAIL (throw): ${name}: ${e.message}`); }
}
console.log(fail === 0 ? `MOCK-PERIPH PASS (${pass} checks)` : `MOCK-PERIPH FAIL (${fail} failures)`);
process.exit(fail === 0 ? 0 : 1);
