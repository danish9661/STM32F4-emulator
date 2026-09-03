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













#[test]
fn exception_svc_roundtrip() {
    let _g = lock_boot();
    // Minimal image in RAM (executable here): main does SVC #0 then loops;
    // SVC handler (vector 11) bumps a counter and returns via EXC_RETURN.
    // Layout: vector table at 0x20000000 is NOT used (CPU vectors come from
    // flash VTOR); instead point VTOR at RAM by writing the model SCB? The
    // model SCB defaults VTOR=0x08000000, so install vectors in flash image.
    let mut img = vec![0u8; 0x200];
    // SP=0x20002000, reset PC=0x08000100
    img[0..4].copy_from_slice(&0x20002000u32.to_le_bytes());
    img[4..8].copy_from_slice(&0x08000100u32.to_le_bytes());
    // SVC vector (11) -> handler at 0x08000110
    img[11 * 4..11 * 4 + 4].copy_from_slice(&0x08000111u32.to_le_bytes());
    // main at 0x100: svc #0 (0xDF00), then b.n loop (0xE7FE)
    img[0x100] = 0x00;
    img[0x101] = 0xDF;
    img[0x102] = 0xFE;
    img[0x103] = 0xE7;
    // handler at 0x110: ldr r0, [pc, #8] (counter addr); ldr r1,[r0]; adds r1,#1;
    // str r1,[r0]; bx lr. Counter at 0x130.
    // 0x110: 4802 (ldr r0,[pc,#8] -> 0x11C); 0x112: 6801 (ldr r1,[r0]); 0x114: 3101 (adds r1,#1)
    // 0x116: 6001 (str r1,[r0]); 0x118: 4770 (bx lr); 0x11A: bf00; 0x11C: 00 01 00 20
    let h: [u8; 16] = [0x02, 0x48, 0x01, 0x68, 0x01, 0x31, 0x01, 0x60, 0x70, 0x47, 0x00, 0xBF, 0x00, 0x01, 0x00, 0x20];
    img[0x110..0x120].copy_from_slice(&h);
    // counter at 0x20001000? use RAM 0x20001000 (in 128K SRAM).
    // patch handler literal to point there:
    img[0x11C..0x120].copy_from_slice(&0x20001000u32.to_le_bytes());
    let (mut cpu, mut mem) = boot(&img);
    // VTOR is 0x08000000 by default: vectors above are in flash image ✓.
    // SP/PC already at reset vector from boot():
    assert_eq!(cpu.regs.r[13], 0x20002000);
    assert_eq!(cpu.regs.r[15] & !1, 0x08000100);
    cpu.deliver_irqs = true;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 10);
    assert!(cpu.fault.is_none(), "fault: {:?}", cpu.fault);
    // SVC handler should have run exactly once (counter==1) and main resumed
    // into its branch-to-self loop at 0x102.
    assert_eq!(mem.read32(0x20001000), 1, "SVC handler did not run");
    assert_eq!(cpu.regs.r[15] & !1, 0x08000102, "did not resume after SVC");
    assert_eq!(cpu.ipsr, 0, "still in handler mode");
}


#[test]
fn freertos_tasks_run() {
    // Full FreeRTOS bring-up on the wasm CPU: SVC start, PendSV task
    // switches, TIM2 ISR semaphore give, TASK1/TASK2 ticks. Guards the
    // exception-entry/return + PSP-banking fixes (even stacked PC, CONTROL
    // update, bank sync, post-frame PSP advance).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../freertos_test/freertos_test.bin"));
    cpu.deliver_irqs = true;
    let sys = crate::sys();
    let mut uart_all = String::new();
    for _ in 0..100 {
        cpu.run(sys, &mut mem, 100_000);
        crate::tick_n(100_000);
        uart_all.push_str(&crate::system::get_uart_output().lock().unwrap().clone());
        crate::system::get_uart_output().lock().unwrap().clear();
        no_fault(&cpu, &mem);
    }
    for m in ["start scheduler", "Hhigh start", "TIM TEST PASS", "TASK1", "TASK2"] {
        assert!(uart_all.contains(m), "missing marker {m:?}: {uart_all:?}");
    }
}
