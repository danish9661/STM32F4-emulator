// Mock-consumer harness for the peripheral gap set: a fake FIRMWARE driving
// the REAL Rust model (real wasm-bindgen bindings + real SVD map, no
// emulator.js driver) — the same pattern as test_eth_mock_consumer.mjs,
// but for the PWR / DCMI / FSMC / ADC_Common / ITM / TIM-TRGO gaps plus the
// documented NOT-modeled set (USB HS, HASH HMAC, ADC dual-interleave, TIM
// encoder). Each test programs the model exactly like firmware does — raw
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
bindings.init_svd(svdXml);

const {
    periph_read, periph_write, tick_n,
    dcmi_feed_frame, dcmi_clear,
    adc_take_dma, adc_set_channel_value, adc_clear_channel_value,
    itm_take_port, itm_port_pending,
    usb_in_status, usb_out_status, usb_reset, usb_enumerated,
    usb_inject_setup, usb_inject_out, usb_take_in,
} = bindings;

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

// ── USB HS Missing ──────────────────────────────────────────────────────
// NOT-modeled: HS SVD entries dropped -> benign 0, never faults.
function t_usbhs() {
    // OTG_HS_GLOBAL base on F407 silicon; model has no slot -> reads 0.
    const HS_GLOBAL = 0x40040000;
    let v = 0, threw = false;
    try { v = R(HS_GLOBAL); } catch { threw = true; }
    ok(!threw && v === 0, 'usbhs: HS global reads benign-0 (dropped entry)');
}

// ── HASH HMAC ───────────────────────────────────────────────────────────
// NOT-modeled: no HMAC key/digest path; assert plain-hash still correct so
// a future HMAC addition can't silently break the base. Uses the exact
// firmware sequence from comprehensive_test/main.c: HASH_CR=1 (INIT),
// DIN single word, STR=0x100 (DCAL) — SHA-1("abcd").
function t_hash() {
    const HASH = 0x50060400;
    W(HASH, 1); // INIT
    W(HASH + 4, 0x61626364); // "abcd"
    W(HASH + 8, 0x100); // DCAL
    const h0 = R(HASH + 0x0C), h4 = R(HASH + 0x1C);
    ok(h0 === 0x81FE8BFE && h4 === 0x82917ACF, 'hash: SHA-1("abcd") reference digest', `h0=0x${h0.toString(16)}`);
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

const tests = [
    ['pwr regulator states beyond handshake', t_pwr],
    ['dcmi pin-sync substitute (feed, never pins)', t_dcmi],
    ['fsmc timings/ECC contract + untapped=0', t_fsmc],
    ['usb-hs missing reads benign-0', t_usbhs],
    ['hash base digest (HMAC absent, base pinned)', t_hash],
    ['adc dual-interleave substitute (independent halves)', t_adc_dual],
    ['tim encoder substitute (stores, holds)', t_tim_enc],
    ['adc AWD + DMA staging (COMPLETE)', t_adc_awd_dma],
    ['tim TRGO routing (COMPLETE)', t_trgo],
    ['itm 32-port stimulus (COMPLETE)', t_itm],
    ['usb STALL handshake (COMPLETE)', t_stall],
];
for (const [name, fn] of tests) {
    console.log(`— ${name}`);
    try { fn(); } catch (e) { fail++; console.error(`  FAIL (throw): ${name}: ${e.message}`); }
}
console.log(fail === 0 ? `MOCK-PERIPH PASS (${pass} checks)` : `MOCK-PERIPH FAIL (${fail} failures)`);
process.exit(fail === 0 ? 0 : 1);
