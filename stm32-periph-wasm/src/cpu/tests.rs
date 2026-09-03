//! Native bring-up tests for the WASM-native Thumb-2 CPU.
//!
//! These run the real firmware binaries through `WasmCpu` + the real
//! peripheral model (SVD map) without any JS/Unicorn involvement, so the
//! edit-compile-debug loop stays inside `cargo test`. They deliberately do
//! NOT call `tick_n` (no INSTRUCTION_COUNT movement) and only drain their
//! own UART output, so they are independent of the other (parallel) tests.
//! The two tests serialize on `BOOT_LOCK` because they share the process
//! `SYS` instance.

use super::{Cpu, mem::FlatMemory};
use super::mem::Memory;
use crate::system::WasmSystem;

static BOOT_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
fn lock_boot() -> std::sync::MutexGuard<'static, ()> {
    BOOT_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn boot(bin: &[u8]) -> (Cpu, FlatMemory) {
    assert!(bin.len() >= 8);
    let sp = u32::from_le_bytes([bin[0], bin[1], bin[2], bin[3]]);
    let pc = u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]]);
    assert!(sp != 0 && pc != 0, "bad vector table");
    // Install a fresh SVD system as the process instance (what init_svd
    // does on the JS path; called directly here to stay test-local).
    let sys = WasmSystem::new_svd(include_str!("../../../monox/stm32f407.svd"));
    crate::init_svd_for_test(sys);
    let mut cpu = Cpu::new(sp, pc | 1);
    let mut mem = FlatMemory::new(0x100000, 0x20000);
    mem.load(bin, 0x08000000);
    assert_eq!(mem.read32(0x08000000), sp, "flash load failed");
    // drain stale UART
    let _ = crate::system::get_uart_output().lock().unwrap().clone();
    crate::system::get_uart_output().lock().unwrap().clear();
    (cpu, mem)
}

fn no_fault(cpu: &Cpu, mem: &FlatMemory) {
    assert!(
        cpu.fault.is_none(),
        "cpu faulted: pc={:08x} op1={:04x} op2={:04x} len={}",
        cpu.fault.map(|f| f.pc).unwrap_or(0),
        cpu.fault.map(|f| f.op1).unwrap_or(0),
        cpu.fault.map(|f| f.op2).unwrap_or(0),
        cpu.fault.map(|f| f.len).unwrap_or(0),
    );
    assert_eq!(
        mem.bad.get(),
        None,
        "bad memory access at pc={:08x}",
        cpu.regs.r[15] & !1
    );
}

#[test]
fn blinky_boots_and_blinks() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    let mut uart = String::new();
    let mut on = false;
    let mut off = false;
    for _ in 0..30 {
        let done = cpu.run(sys, &mut mem, 1_000_000);
        uart.push_str(&crate::system::get_uart_output().lock().unwrap().clone());
        crate::system::get_uart_output().lock().unwrap().clear();
        let odr = mem.read32(0x40020014);
        if odr & 0x20 != 0 {
            on = true;
        } else {
            off = true;
        }
        no_fault(&cpu, &mem);
        if uart.contains("tick 2") && on && off {
            break;
        }
        assert!(done > 0, "cpu stopped making progress");
    }
    assert!(uart.contains("=== Blinky ==="), "no banner: {uart:?}");
    assert!(uart.contains("tick 0"), "no ticks: {uart:?}");
    assert!(on && off, "PA5 never toggled");
}

#[test]
fn eth_http_dhcp_offer_parse() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../eth_http/eth_http.bin"));
    let sys = crate::sys();
    // Offer/Ack captured from a live netsim run (XID is the firmware's fixed
    // 0x87654321, so they replay deterministically). Regenerate via
    // `node site/save_rx.mjs` if netsim's replies change.
    let offer = include_bytes!("../../../site/testdata_offer.bin");
    let ack = include_bytes!("../../../site/testdata_ack.bin");
    let mut n_tx = 0u32;
    let mut uart = String::new();
    for _ in 0..300 {
        cpu.run(sys, &mut mem, 200_000);
        no_fault(&cpu, &mem);
        if crate::system::eth_is_tx_poll() {
            let desc = crate::system::eth_get_tx_desc_addr();
            let tdes0 = mem.read32(desc);
            let tdes1 = mem.read32(desc + 4);
            if tdes0 & 0x80000000 != 0 {
                let len = (tdes0 & 0x3FFF) as usize;
                mem.write32(desc, (tdes0 & !0x80000000) | 0x20000000);
                crate::system::eth_clear_tx_poll();
                crate::system::eth_set_done(1);
                let f = mem.read32(0x20000620);
                mem.write32(0x20000620, f | 1);
                if len > 0 {
                    // DHCP (UDP dport 67): 1st TX = Discover -> Offer,
                    // 2nd TX = Request -> Ack.
                    let buf = tdes1;
                    let udp_dport =
                        (mem.read8(buf + 36) as u16) << 8 | mem.read8(buf + 37) as u16;
                    if udp_dport == 67 {
                        n_tx += 1;
                        let reply = if n_tx == 1 { &offer[..] } else { &ack[..] };
                        for (i, &b) in reply.iter().enumerate() {
                            mem.write8(0x20000660 + i as u32, b);
                        }
                        mem.write32(0x20000630, (reply.len() as u32) << 16);
                        mem.write32(0x20000628, 0);
                        mem.write32(0x2000062c, reply.len() as u32);
                        let f2 = mem.read32(0x20000620);
                        mem.write32(0x20000620, f2 | 2);
                        crate::system::eth_clear_rx_poll();
                        crate::system::eth_set_done(2);
                    }
                }
            } else {
                crate::system::eth_clear_tx_poll();
                crate::system::eth_set_done(1);
            }
        }
        uart.push_str(&crate::system::get_uart_output().lock().unwrap().clone());
        crate::system::get_uart_output().lock().unwrap().clear();
        if uart.contains("DHCP Ack") {
            return;
        }
        if uart.contains("DHCP failed") || uart.contains("TX timeout") {
            panic!("round failed: {uart:?}");
        }
    }
    panic!("no DHCP Ack, uart: {uart:?}");
}

#[test]
fn eth_http_reaches_dhcp_discover() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../eth_http/eth_http.bin"));
    let sys = crate::sys();
    let mut uart = String::new();
    for _ in 0..40 {
        let done = cpu.run(sys, &mut mem, 1_000_000);
        uart.push_str(&crate::system::get_uart_output().lock().unwrap().clone());
        crate::system::get_uart_output().lock().unwrap().clear();
        no_fault(&cpu, &mem);
        if uart.contains("DHCP Discover") {
            break;
        }
        assert!(done > 0, "cpu stopped making progress");
    }
    assert!(uart.contains("DHCP Discover"), "no discover: {uart:?}");
}












