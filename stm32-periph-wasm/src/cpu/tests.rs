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


/// Resolve a symbol address from doom.elf's symtab at test time, so tests
/// never hardcode firmware addresses (any rebuild that changes code size
/// shifts them; a stale constant jumps mid-function and fails spuriously,
/// looking exactly like a CPU regression — this bit us on strcasecmp).
fn doom_sym(name: &str) -> u32 {
    let elf: &[u8] = include_bytes!("../../../doom/doom.elf");
    let u16le = |o: usize| u16::from_le_bytes([elf[o], elf[o + 1]]) as usize;
    let u32le = |o: usize| u32::from_le_bytes([elf[o], elf[o + 1], elf[o + 2], elf[o + 3]]);
    let shoff = u32le(0x20) as usize;
    let shentsize = u16le(0x2E);
    let shnum = u16le(0x30);
    let mut symoff = 0usize;
    let mut symsize = 0usize;
    let mut strtab = 0usize;
    for i in 0..shnum {
        let h = shoff + i * shentsize;
        let ty = u32le(h + 4);
        if ty == 2 {
            // SHT_SYMTAB; sh_link = associated strtab section index
            symoff = u32le(h + 16) as usize;
            symsize = u32le(h + 20) as usize;
            let link = u32le(h + 24) as usize;
            let sh = shoff + link * shentsize;
            strtab = u32le(sh + 16) as usize;
            break;
        }
    }
    assert!(symoff != 0 && strtab != 0, "no symtab in doom.elf");
    let want = name.as_bytes();
    let mut o = symoff;
    while o + 16 <= symoff + symsize {
        let noff = u32le(o) as usize;
        let val = u32le(o + 4);
        let mut ok = elf[strtab + noff..].starts_with(want);
        ok = ok && elf[strtab + noff + want.len()] == 0;
        if ok {
            return val;
        }
        o += 16;
    }
    panic!("symbol {name:?} not found in doom.elf");
}

fn boot_doom() -> (Cpu, FlatMemory) {
    let doom = include_bytes!("../../../doom/doom.bin");
    let wad = include_bytes!("../../../site/doom1.wad");
    let sp = u32::from_le_bytes([doom[0], doom[1], doom[2], doom[3]]);
    let pc = u32::from_le_bytes([doom[4], doom[5], doom[6], doom[7]]);
    let sys = WasmSystem::new_svd(include_str!("../../../monox/stm32f407.svd"));
    crate::init_svd_for_test(sys);
    let mut cpu = Cpu::new(sp, pc | 1);
    let mut mem = FlatMemory::new(0x100000, 0x20000);
    mem.load(doom, 0x08000000);
    mem.map_extra(0xC0000000, 16 * 1024 * 1024);
    mem.map_extra(0xB8000000, 8 * 1024 * 1024);
    mem.load(wad, 0xB8000000);
    crate::system::get_uart_output().lock().unwrap().clear();
    (cpu, mem)
}

#[test]
fn doom_title_renders() {
    // DOOM boots through R_InitTextures to a rendered TITLEPIC on the wasm
    // CPU. Guards the decoder fixes this took (T3 register-offset writeback,
    // SDIV/SMLAL select, ADDW/SUBW, USAT/SSAT, TBB index + unmasked base):
    // each produced silent wrongness (garbage textures, skipped divides,
    // wrong demo, undrawn title) rather than faults.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot_doom();
    let sys = crate::sys();
    let mut drew = false;
    for _ in 0..100 {
        cpu.run(sys, &mut mem, 1_000_000);
        assert!(cpu.fault.is_none(), "doom faulted: {:?}", cpu.fault);
        assert!(mem.bad.get().is_none(), "bad mem access");
        let fb = mem.read32(0x20002510);
        if fb != 0 {
            let nz: u32 = (0..64000u32).map(|i| (mem.read8(fb + i) != 0) as u32).sum();
            if nz > 10000 {
                drew = true;
                break;
            }
        }
    }
    assert!(drew, "title never rendered");
    // and it is really the title (not garbage): pagename == TITLEPIC
    let pn = mem.read32(0xC00143A0);
    let nm: Vec<u8> = (0..8u32).map(|i| mem.read8(pn + i)).collect();
    assert_eq!(&nm, b"TITLEPIC");
}

fn run_snippet(code: &[u16], regs: &[(usize, u32)]) -> (Cpu, FlatMemory) {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    for (i, w) in code.iter().enumerate() {
        mem.write16(0x20002000 + (i as u32) * 2, *w);
    }
    for &(r, v) in regs {
        cpu.regs.r[r] = v;
    }
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, code.len() as u32 / 2 + 2);
    (cpu, mem)
}

#[test]
fn tbb_index_by_value() {
    // tbb [pc,r3] indexes by r3's VALUE with an unmasked pc+4 base.
    // Table at (pc+4): [0x04 -> case0][0x10 -> case1]; r3=1 -> case1.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    mem.write16(0x20002000, 0xE8DF);
    mem.write16(0x20002002, 0xF003);
    mem.write8(0x20002004, 0x04);
    mem.write8(0x20002005, 0x10);
    cpu.regs.r[3] = 1;
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[15] & !1, 0x20002024);
}

#[test]
fn sdiv_plain_and_it() {
    // sdiv r1,r1,r3 (FB91 F1F3): plain, IT-taken, IT-skipped (sentinel kept).
    let (mut cpu, _) = run_snippet(&[0xFB91, 0xF1F3], &[(1, 1680), (3, 10)]);
    assert_eq!(cpu.regs.r[1], 168);
    // cmp r1,#11 (NE, r1=1680) ; ite gt (BFCC) ; sdivne (taken: 168)
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    for (i, w) in [0x290Bu16, 0xBFCC, 0xFB91, 0xF1F3].iter().enumerate() {
        mem.write16(0x20002000 + i as u32 * 2, *w);
    }
    cpu.regs.r[1] = 1680;
    cpu.regs.r[3] = 10;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 4); // cmp,it,sdiv(taken) + 1 nop to consume ite slot2
    assert_eq!(cpu.regs.r[1], 168);
    // EQ: cmp r1,#11 (r1=11) ; ite gt ; sdivne must NOT run
    cpu.regs.r[1] = 11;
    cpu.regs.r[3] = 10;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 3);
    // sdivne skipped -> r1 stays 11
    assert_eq!(cpu.regs.r[1], 11);
}

#[test]
fn usat_ssat_q() {
    let (mut cpu, _) = run_snippet(&[0xF380, 0x0005], &[(0, 100)]);
    assert_eq!(cpu.regs.r[0], 31);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    let (mut cpu, _) = run_snippet(&[0xF380, 0x0005], &[(0, 20)]);
    assert_eq!(cpu.regs.r[0], 20);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // SSAT sat field encodes N-1 (ssat#8 = o2 0x0007)
    let (mut cpu, _) = run_snippet(&[0xF300, 0x0007], &[(0, 1000)]);
    assert_eq!(cpu.regs.r[0], 127);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    let (mut cpu, _) = run_snippet(&[0xF300, 0x0007], &[(0, 0xFFFFFC18)]);
    assert_eq!(cpu.regs.r[0], 0xFFFFFF80);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
}

#[test]
fn smlald_dual_add_long() {
    // smlald r0,r1,r2,r3 = fbc2 01c3: acc += lo*lo + hi*hi (signed).
    let (mut cpu, _) = run_snippet(
        &[0xFBC2, 0x01C3],
        &[(0, 0), (1, 0), (2, 0x00020001), (3, 0x00040003)],
    );
    assert_eq!(cpu.regs.r[0], 11); // 1*3 + 2*4
    assert_eq!(cpu.regs.r[1], 0);
    // Mixed signs: 1*1 + (-1)*1 = 0.
    let (mut cpu, _) = run_snippet(
        &[0xFBC2, 0x01C3],
        &[(0, 0), (1, 0), (2, 0xFFFF0001), (3, 0x00010001)],
    );
    assert_eq!(cpu.regs.r[0], 0);
    assert_eq!(cpu.regs.r[1], 0);
}

#[test]
fn smlsld_dual_sub_long() {
    // smlsld r0,r1,r2,r3 = fbd2 01c3: acc += lo*lo - hi*hi.
    // 1*3 - 2*4 = -5.
    let (mut cpu, _) = run_snippet(
        &[0xFBD2, 0x01C3],
        &[(0, 0), (1, 0), (2, 0x00020001), (3, 0x00040003)],
    );
    assert_eq!(cpu.regs.r[0], 0xFFFFFFFB);
    assert_eq!(cpu.regs.r[1], 0xFFFFFFFF);
}

#[test]
fn umaal_dual_accumulate() {
    // umaal r4,r5,r6,r7 = fbe6 4567: acc += Rn*Rm + RdLo + RdHi.
    // (2^32-1)^2 + 1 + 2 = 0xFFFFFFFE00000004.
    let (mut cpu, _) = run_snippet(
        &[0xFBE6, 0x4567],
        &[(4, 1), (5, 2), (6, 0xFFFFFFFF), (7, 0xFFFFFFFF)],
    );
    assert_eq!(cpu.regs.r[4], 4);
    assert_eq!(cpu.regs.r[5], 0xFFFFFFFE);
    // Small: 2*3 + 10 + 0 = 16 (exercises both Rd adds).
    let (mut cpu, _) = run_snippet(
        &[0xFBE6, 0x4567],
        &[(4, 10), (5, 0), (6, 2), (7, 3)],
    );
    assert_eq!(cpu.regs.r[4], 16);
    assert_eq!(cpu.regs.r[5], 0);
}

#[test]
fn smmul_rounding() {
    // smmul r0,r1,r2 = fb51 f002: top32(prod). 0x40000000^2 top = 0x10000000.
    let (mut cpu, _) = run_snippet(&[0xFB51, 0xF002], &[(1, 0x40000000), (2, 0x40000000)]);
    assert_eq!(cpu.regs.r[0], 0x10000000);
    // smmulr (fb51 f012) rounds: (-2^31)*(-1) = 2^31 -> top 0...
    // prod = 0x80000000, +0x80000000 = 0x100000000 -> top 1.
    let (mut cpu, _) = run_snippet(&[0xFB51, 0xF012], &[(1, 0x80000000), (2, 0xFFFFFFFF)]);
    assert_eq!(cpu.regs.r[0], 1);
    let (mut cpu, _) = run_snippet(&[0xFB51, 0xF002], &[(1, 0x80000000), (2, 0xFFFFFFFF)]);
    assert_eq!(cpu.regs.r[0], 0);
    // smmla r0,r1,r2,r3 = fb51 3002: Ra + top.
    let (mut cpu, _) = run_snippet(
        &[0xFB51, 0x3002],
        &[(1, 0x40000000), (2, 0x40000000), (3, 5)],
    );
    assert_eq!(cpu.regs.r[0], 0x10000005);
    // smmlar (fb51 3012) vs smmla on the rounding edge (prod = 0x80000000).
    let (mut cpu, _) = run_snippet(
        &[0xFB51, 0x3012],
        &[(1, 0x80000000), (2, 0xFFFFFFFF), (3, 7)],
    );
    assert_eq!(cpu.regs.r[0], 8);
    let (mut cpu, _) = run_snippet(
        &[0xFB51, 0x3002],
        &[(1, 0x80000000), (2, 0xFFFFFFFF), (3, 7)],
    );
    assert_eq!(cpu.regs.r[0], 7);
    // smmls r0,r1,r2,r3 = fb61 3002: Ra - top.
    let (mut cpu, _) = run_snippet(
        &[0xFB61, 0x3002],
        &[(1, 0x40000000), (2, 0x40000000), (3, 5)],
    );
    assert_eq!(cpu.regs.r[0], 0xF0000005);
    // smmlsr (fb61 3012): 7 - 1 = 6 with rounding, 7 - 0 = 7 without.
    let (mut cpu, _) = run_snippet(
        &[0xFB61, 0x3012],
        &[(1, 0x80000000), (2, 0xFFFFFFFF), (3, 7)],
    );
    assert_eq!(cpu.regs.r[0], 6);
    let (mut cpu, _) = run_snippet(
        &[0xFB61, 0x3002],
        &[(1, 0x80000000), (2, 0xFFFFFFFF), (3, 7)],
    );
    assert_eq!(cpu.regs.r[0], 7);
}

#[test]
fn usad8_accumulate() {
    // usad8 r0,r1,r2 = fb71 f002: |1-4|+|2-3|+|3-2|+|4-1| = 8.
    let (mut cpu, _) = run_snippet(&[0xFB71, 0xF002], &[(1, 0x01020304), (2, 0x04030201)]);
    assert_eq!(cpu.regs.r[0], 8);
    // usada8 r0,r1,r2,r3 = fb71 3002: + Ra.
    let (mut cpu, _) = run_snippet(
        &[0xFB71, 0x3002],
        &[(1, 0x01020304), (2, 0x04030201), (3, 100)],
    );
    assert_eq!(cpu.regs.r[0], 108);
}

#[test]
fn parallel_qadd_qsub() {
    // qadd8 r0,r1,r2 = fa81 f012: 0x7F+1 saturates per lane + Q.
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF012], &[(1, 0x7F7F7F7F), (2, 0x01010101)]);
    assert_eq!(cpu.regs.r[0], 0x7F7F7F7F);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // No saturation: exact + Q clear.
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF012], &[(1, 0x01010101), (2, 0x02020202)]);
    assert_eq!(cpu.regs.r[0], 0x03030303);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // qsub8 r0,r1,r2 = fac1 f012: -128-1 saturates to -128 + Q.
    let (mut cpu, _) = run_snippet(&[0xFAC1, 0xF012], &[(1, 0x80808080), (2, 0x01010101)]);
    assert_eq!(cpu.regs.r[0], 0x80808080);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // qadd16 r0,r1,r2 = fa91 f012: 0x7FFF+1 saturates + Q.
    let (mut cpu, _) = run_snippet(&[0xFA91, 0xF012], &[(1, 0x7FFF7FFF), (2, 0x00010001)]);
    assert_eq!(cpu.regs.r[0], 0x7FFF7FFF);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // qsub16 r0,r1,r2 = fad1 f012: 0x8000-1 saturates to 0x8000 + Q.
    let (mut cpu, _) = run_snippet(&[0xFAD1, 0xF012], &[(1, 0x80008000), (2, 0x00010001)]);
    assert_eq!(cpu.regs.r[0], 0x80008000);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // uqadd16 r0,r1,r2 = fa91 f052: 0xFFFF+1 saturates + Q (lo exact).
    let (mut cpu, _) = run_snippet(&[0xFA91, 0xF052], &[(1, 0xFFFF0001), (2, 0x00010000)]);
    assert_eq!(cpu.regs.r[0], 0xFFFF0001);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // uqsub16 r0,r1,r2 = fad1 f052: underflow saturates to 0 + Q.
    let (mut cpu, _) = run_snippet(&[0xFAD1, 0xF052], &[(1, 0x00010000), (2, 0x00020001)]);
    assert_eq!(cpu.regs.r[0], 0x00000000);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // uqadd8/uqsub8 (fa81/fac1 f052).
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF052], &[(1, 0xFF00FF00), (2, 0x01000100)]);
    assert_eq!(cpu.regs.r[0], 0xFF00FF00);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    let (mut cpu, _) = run_snippet(&[0xFAC1, 0xF052], &[(1, 0x01000100), (2, 0x02000200)]);
    assert_eq!(cpu.regs.r[0], 0x00000000);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
}

#[test]
fn parallel_halving() {
    // shadd8 r0,r1,r2 = fa81 f022: (2+4)>>1 = 3, no Q.
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF022], &[(1, 0x02020202), (2, 0x04040404)]);
    assert_eq!(cpu.regs.r[0], 0x03030303);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // Arithmetic shift keeps sign: (-2 + -4)>>1 = -3.
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF022], &[(1, 0xFEFEFEFE), (2, 0xFCFCFCFC)]);
    assert_eq!(cpu.regs.r[0], 0xFDFDFDFD);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // shsub8 r0,r1,r2 = fac1 f022: (4-2)>>1 = 1.
    let (mut cpu, _) = run_snippet(&[0xFAC1, 0xF022], &[(1, 0x04040404), (2, 0x02020202)]);
    assert_eq!(cpu.regs.r[0], 0x01010101);
    // uhadd8 r0,r1,r2 = fa81 f062: (255+255)>>1 = 255 (logical).
    let (mut cpu, _) = run_snippet(&[0xFA81, 0xF062], &[(1, 0xFFFFFFFF), (2, 0xFFFFFFFF)]);
    assert_eq!(cpu.regs.r[0], 0xFFFFFFFF);
    // uhsub8 r0,r1,r2 = fac1 f062: (4-6)>>1 logical = 0x7FFFFFFF[lane] = 0xFF.
    let (mut cpu, _) = run_snippet(&[0xFAC1, 0xF062], &[(1, 0x04040404), (2, 0x06060606)]);
    assert_eq!(cpu.regs.r[0], 0xFFFFFFFF);
    // shadd16 r0,r1,r2 = fa91 f022: (2+6)>>1=4, (4+8)>>1=6.
    let (mut cpu, _) = run_snippet(&[0xFA91, 0xF022], &[(1, 0x00020004), (2, 0x00060008)]);
    assert_eq!(cpu.regs.r[0], 0x00040006);
    // shsub16 r0,r1,r2 = fad1 f022.
    let (mut cpu, _) = run_snippet(&[0xFAD1, 0xF022], &[(1, 0x00040006), (2, 0x00020004)]);
    assert_eq!(cpu.regs.r[0], 0x00010001);
    // uhadd16 r0,r1,r2 = fa91 f062: (0xFFFE+4)>>1 = 0x8001, (2+6)>>1 = 4.
    let (mut cpu, _) = run_snippet(&[0xFA91, 0xF062], &[(1, 0xFFFE0002), (2, 0x00040006)]);
    assert_eq!(cpu.regs.r[0], 0x80010004);
    // uhsub16 r0,r1,r2 = fad1 f062: (4-2)>>1 = 1, (6-8)>>1 logical = 0xFFFF.
    let (mut cpu, _) = run_snippet(&[0xFAD1, 0xF062], &[(1, 0x00040006), (2, 0x00020008)]);
    assert_eq!(cpu.regs.r[0], 0x0001FFFF);
}

#[test]
fn parallel_asx_sax() {
    // qasx r0,r1,r2 = faa1 f012: top = hi+lo, bottom = lo-hi.
    // Rn=0x00020001, Rm=0x00040003 -> top 2+3=5, bot 1-4=-3.
    let (mut cpu, _) = run_snippet(&[0xFAA1, 0xF012], &[(1, 0x00020001), (2, 0x00040003)]);
    assert_eq!(cpu.regs.r[0], 0x0005FFFD);
    assert_eq!(cpu.regs.xpsr & 0x08000000, 0);
    // Saturating: top 0x7FFF+1 -> 0x7FFF + Q; bot 1-0x7FFF fits.
    let (mut cpu, _) = run_snippet(&[0xFAA1, 0xF012], &[(1, 0x7FFF0001), (2, 0x7FFF0001)]);
    assert_eq!(cpu.regs.r[0], 0x7FFF8002);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // qsax r0,r1,r2 = fae1 f012: top = hi-lo, bot = lo+hi.
    let (mut cpu, _) = run_snippet(&[0xFAE1, 0xF012], &[(1, 0x00020001), (2, 0x00040003)]);
    assert_eq!(cpu.regs.r[0], 0xFFFF0005);
    // uqasx r0,r1,r2 = faa1 f052: top 0xFFFF+0xFFFF saturates + Q.
    let (mut cpu, _) = run_snippet(&[0xFAA1, 0xF052], &[(1, 0xFFFF0001), (2, 0x0001FFFF)]);
    assert_eq!(cpu.regs.r[0], 0xFFFF0000);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // uqsax r0,r1,r2 = fae1 f052: bot 2+0xFFFF saturates + Q.
    let (mut cpu, _) = run_snippet(&[0xFAE1, 0xF052], &[(1, 0x00010002), (2, 0xFFFF0001)]);
    assert_eq!(cpu.regs.r[0], 0x0000FFFF);
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0);
    // shasx r0,r1,r2 = faa1 f022: top (2+8)>>1=5, bot (4-6)>>1=-1.
    let (mut cpu, _) = run_snippet(&[0xFAA1, 0xF022], &[(1, 0x00020004), (2, 0x00060008)]);
    assert_eq!(cpu.regs.r[0], 0x0005FFFF);
    // shsax r0,r1,r2 = fae1 f022: top (2-8)>>1=-3, bot (4+6)>>1=5.
    // (Rm halves exchange: top uses Rm.lo=8, bottom uses Rm.hi=6.)
    let (mut cpu, _) = run_snippet(&[0xFAE1, 0xF022], &[(1, 0x00020004), (2, 0x00060008)]);
    assert_eq!(cpu.regs.r[0], 0xFFFD0005);
    // uhasx r0,r1,r2 = faa1 f062: top (2+8)>>1=5, bot (4-6)>>1 logical=0xFFFF.
    let (mut cpu, _) = run_snippet(&[0xFAA1, 0xF062], &[(1, 0x00020004), (2, 0x00060008)]);
    assert_eq!(cpu.regs.r[0], 0x0005FFFF);
    // uhsax r0,r1,r2 = fae1 f062: top (2-6)>>1 logical=0xFFFE,
    // bot (4+8)>>1=6.
    let (mut cpu, _) = run_snippet(&[0xFAE1, 0xF062], &[(1, 0x00020004), (2, 0x00080006)]);
    assert_eq!(cpu.regs.r[0], 0xFFFE0006);
}

#[test]
fn sxtab16_uxtab16() {    // sxtab16 r0,r1,r2 = fa21 f082: lo = 2+SXTH(4), hi = 1+SXTH(3).
    let (mut cpu, _) = run_snippet(&[0xFA21, 0xF082], &[(1, 0x00010002), (2, 0x00030004)]);
    assert_eq!(cpu.regs.r[0], 0x00040006);
    // ror #8 first: Rm=0x04000300 ror 8 = 0x00040003.
    let (mut cpu, _) = run_snippet(&[0xFA21, 0xF092], &[(1, 0x00010002), (2, 0x04000300)]);
    assert_eq!(cpu.regs.r[0], 0x00050005);
    // uxtab16 r0,r1,r2 = fa31 f082: zero-extend.
    let (mut cpu, _) = run_snippet(&[0xFA31, 0xF082], &[(1, 0x00010002), (2, 0x00FF00FE)]);
    assert_eq!(cpu.regs.r[0], 0x01000100);
}

#[test]
fn shift_reg_flag_setting() {
    // lsls.w r0,r1,r2 = fa11 f002.
    let (mut cpu, _) = run_snippet(&[0xFA11, 0xF002], &[(1, 1), (2, 3)]);
    assert_eq!(cpu.regs.r[0], 8);
    assert_eq!(cpu.regs.xpsr & 0xE0000000, 0);
    // Carry out of bit 31.
    let (mut cpu, _) = run_snippet(&[0xFA11, 0xF002], &[(1, 0x80000000), (2, 1)]);
    assert_eq!(cpu.regs.r[0], 0);
    assert_ne!(cpu.regs.xpsr & 0x60000000, 0); // Z=1, C=1
    // lsrs.w r0,r1,r2 = fa31 f002: 1>>1 = 0, C=1 (bit 0 out), Z=1.
    let (mut cpu, _) = run_snippet(&[0xFA31, 0xF002], &[(1, 1), (2, 1)]);
    assert_eq!(cpu.regs.r[0], 0);
    assert_ne!(cpu.regs.xpsr & 0x60000000, 0);
    // asrs.w r0,r1,r2 = fa51 f002: 0x80000000>>4 arithmetic.
    let (mut cpu, _) = run_snippet(&[0xFA51, 0xF002], &[(1, 0x80000000), (2, 4)]);
    assert_eq!(cpu.regs.r[0], 0xF8000000);
    assert_ne!(cpu.regs.xpsr & 0x80000000, 0); // N=1
    // rors.w r0,r1,r2 = fa71 f002: ror(1, 1) = 0x80000000, C=1.
    let (mut cpu, _) = run_snippet(&[0xFA71, 0xF002], &[(1, 1), (2, 1)]);
    assert_eq!(cpu.regs.r[0], 0x80000000);
    assert_ne!(cpu.regs.xpsr & 0xA0000000, 0); // N=1, C=1
}

#[test]
fn ldrex_strex_sizes() {
    // Byte/halfword/word exclusives; single-threaded: STREX always 0.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    mem.write8(0x20003000, 0xAB);
    mem.write16(0x20003010, 0xCDEF);
    mem.write32(0x20003020, 0x12345678);
    // ldrexb r0,[r1] = e8d1 0f4f ; ldrexh r2,[r3] = e8d3 2f5f
    // (Rt = o2[15:12]).
    mem.write16(0x20002000, 0xE8D1);
    mem.write16(0x20002002, 0x0F4F);
    mem.write16(0x20002004, 0xE8D3);
    mem.write16(0x20002006, 0x2F5F);
    cpu.regs.r[1] = 0x20003000;
    cpu.regs.r[3] = 0x20003010;
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 2);
    assert_eq!(cpu.regs.r[0], 0xAB);
    assert_eq!(cpu.regs.r[2], 0xCDEF);
    // strexb r4,r5,[r6] = e8c6 5f44 ; strexh r7,r8,[r9] = e8c9 8f57
    // (o2 = Rt:F:size:Rd).
    mem.write16(0x20002000, 0xE8C6);
    mem.write16(0x20002002, 0x5F44);
    mem.write16(0x20002004, 0xE8C9);
    mem.write16(0x20002006, 0x8F57);
    cpu.regs.r[5] = 0x12;
    cpu.regs.r[6] = 0x20003000;
    cpu.regs.r[8] = 0x3456;
    cpu.regs.r[9] = 0x20003010;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 2);
    assert_eq!(mem.read8(0x20003000), 0x12);
    assert_eq!(cpu.regs.r[4], 0);
    assert_eq!(mem.read16(0x20003010), 0x3456);
    assert_eq!(cpu.regs.r[7], 0);
    // Word forms: ldrex r0,[r1] = e851 0f00 ;
    // strex r2,r3,[r4,#8] = e844 3208 (word o1 nibble is 0x084x, NOT 0x08Cx;
    // o2 = Rt:Rd:imm8, imm scaled x4).
    // (Reload the word source: the strexb above wrote 0x12 to 0x20003000.)
    mem.write32(0x20003000, 0x12345678);
    mem.write16(0x20002000, 0xE851);
    mem.write16(0x20002002, 0x0F00);
    mem.write16(0x20002004, 0xE844);
    mem.write16(0x20002006, 0x3208);
    cpu.regs.r[1] = 0x20003000;
    cpu.regs.r[3] = 0xDEADBEEF;
    cpu.regs.r[4] = 0x20003000;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 2);
    assert_eq!(cpu.regs.r[0], 0x12345678);
    assert_eq!(mem.read32(0x20003020), 0xDEADBEEF);
    assert_eq!(cpu.regs.r[2], 0);
    // Word LDREX with an offset whose imm8 hits the B/H size nibbles
    // (imm8 0x40 -> [7:4]==4): still a word load, addr scaled x4.
    // ldrex r5,[r6,#0x100] = e856 5f40 (nibble stays 0x0850).
    mem.write32(0x20003100, 0xA5A5A5A5);
    mem.write16(0x20002000, 0xE856);
    mem.write16(0x20002002, 0x5F40);
    cpu.regs.r[6] = 0x20003000;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[5], 0xA5A5A5A5);
}

#[test]
fn addw_subw_plain_imm() {
    let (mut cpu, _) = run_snippet(&[0xF20A, 0x46BC], &[(10, 100)]);
    assert_eq!(cpu.regs.r[6], 100 + 1212);
    let (mut cpu, _) = run_snippet(&[0xF2AA, 0x46BC], &[(10, 100)]);
    assert_eq!(cpu.regs.r[6], (100i32 - 1212) as u32);
    let (mut cpu, _) = run_snippet(&[0xF6A1, 0x71FF], &[(1, 5000)]);
    assert_eq!(cpu.regs.r[1], 5000 - 4095);
}

#[test]
fn t3_reg_no_writeback() {
    // strh.w r2,[r9,r3,lsl#1] (F829 2013) must not write back Rn/Rm.
    let (mut cpu, mem) = run_snippet(&[0xF829, 0x2013], &[(9, 0x20003000), (3, 5), (2, 0xABCD)]);
    assert_eq!(mem.read16(0x2000300A), 0xABCD);
    assert_eq!(cpu.regs.r[9], 0x20003000);
    assert_eq!(cpu.regs.r[3], 5);
}




#[test]
fn it_pred_mov_preserves() {
    // D_PageTicker: cmp sets N=1; itt lt; movlt (taken) must preserve N
    // so strlt (LT) also takes. Unpredicated movs still sets N/Z.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    // cmp r3,#0 (r3=-20) ; itt lt (BFBC) ; movlt r3,#1 ; strlt r3,[r2]
    for (i, w) in [0x2B03u16, 0xBFBC, 0x2301, 0x6013].iter().enumerate() {
        mem.write16(0x20002000 + i as u32 * 2, *w);
    }
    cpu.regs.r[3] = 0xFFFFFFEC;
    cpu.regs.r[2] = 0x20003000;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 4);
    assert_eq!(cpu.regs.r[3], 1);
    assert_eq!(mem.read32(0x20003000), 1);
    assert_eq!((cpu.regs.xpsr >> 31) & 1, 1, "N preserved through predicated movs");
}

#[test]
fn bare_movs_sets_nz_preserves_c() {
    // cmp r2,#1 (r2=-6: C=1) ; movs r0,#0 -> N=0,Z=1,C stays 1
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    for (i, w) in [0x2A01u16, 0x2000].iter().enumerate() {
        mem.write16(0x20002000 + i as u32 * 2, *w);
    }
    cpu.regs.r[2] = 0xFFFFFFFA;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 2);
    let x = cpu.regs.xpsr;
    assert_eq!(cpu.regs.r[0], 0);
    assert_eq!((x >> 31) & 1, 0, "N");
    assert_eq!((x >> 30) & 1, 1, "Z");
    assert_eq!((x >> 29) & 1, 1, "C preserved");
}

#[test]
fn it_pred_add_preserves() {
    // cmp r3,#0 (r3=-5, N=1) ; itt mi (BF? mask C cond MI=4: 0xBFC4) ;
    // addmi r6,r6,r3 (taken, r6=0+5, N stays 1)
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    for (i, w) in [0x2B00u16, 0xBF44, 0x18F6].iter().enumerate() {
        mem.write16(0x20002000 + i as u32 * 2, *w);
    }
    cpu.regs.r[3] = 0xFFFFFFFB;
    cpu.regs.r[6] = 0;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 3);
    assert_eq!(cpu.regs.r[6], 0xFFFFFFFB);
    assert_eq!((cpu.regs.xpsr >> 31) & 1, 1, "N preserved through predicated add");
}

#[test]
fn bare_subreg_sets_flags() {
    // subs r3,r3,r0 (1A1B) unpredicated with equal inputs -> Z=1.
    // Run EXACTLY 1 step: run_snippet's trailing NOPs (movs r0,r0) would
    // clobber Z and mask the assertion.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write16(0x20002000, 0x1A1B);
    cpu.regs.r[3] = 0x64;
    cpu.regs.r[0] = 0x64;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[3], 0);
    assert_eq!((cpu.regs.xpsr >> 30) & 1, 1, "Z");
}

#[test]
fn ldrsh_reg_sx() {
    // ldrsh.w r2,[r0,r3,lsl#2] (F930 2023): signed halfword, no writeback.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write16(0x20002000, 0xF930);
    mem.write16(0x20002002, 0x2023);
    mem.write16(0x20003000, 0xFF80); // -128
    cpu.regs.r[0] = 0x20003000;
    cpu.regs.r[3] = 0;
    cpu.regs.r[9] = 0x20003000;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[2], 0xFFFFFF80);
    assert_eq!(cpu.regs.r[0], 0x20003000, "no writeback to Rn");
}

#[test]
fn cmp13_n_flag() {
    // cmp r3,#3 with r3=1 -> N=1,Z=0,C=0,V=0 (S_Start's LE depends on N).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write16(0x20002000, 0x2B03);
    cpu.regs.r[3] = 1;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    let x = cpu.regs.xpsr;
    assert_eq!((x >> 31) & 1, 1, "N");
    assert_eq!((x >> 30) & 1, 0, "Z");
    assert_eq!((x >> 29) & 1, 0, "C");
    assert_eq!((x >> 28) & 1, 0, "V");
}

#[test]
fn strcasecmp_pairs() {
    // newlib strcasecmp via doom firmware: equal-ci, differ-byte-0,
    // differ-byte-4 (doom1 vs doom2 must differ: D_FindIWAD depends on it).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot_doom();
    let sys = crate::sys();
    for (i, b) in b"doom1.wad\0".iter().enumerate() { mem.write8(0x20003000 + i as u32, *b); }
    for (i, b) in b"heretic1.wad\0".iter().enumerate() { mem.write8(0x20003100 + i as u32, *b); }
    for (i, b) in b"DOOM1.WAD\0".iter().enumerate() { mem.write8(0x20003200 + i as u32, *b); }
    for (i, b) in b"doom2.wad\0".iter().enumerate() { mem.write8(0x20003300 + i as u32, *b); }
    mem.write16(0x20001000, 0xE7FE);
    for ((a, b), want_ne) in [((0x20003000, 0x20003100), true), ((0x20003000, 0x20003200), false), ((0x20003000, 0x20003300), true)] {
        cpu.regs.r[0] = a;
        cpu.regs.r[1] = b;
        cpu.regs.r[14] = 0x20001001;
        cpu.regs.r[15] = doom_sym("strcasecmp") | 1;
        for _ in 0..100000 {
            cpu.run(sys, &mut mem, 1);
            if (cpu.regs.r[15] & !1) == 0x20001000 { break; }
            if cpu.fault.is_some() { break; }
        }
        let r = cpu.regs.r[0] as i32;
        assert_eq!(r != 0, want_ne, "strcasecmp pair");
    }
}


#[test]
fn can_inject_native() {
    let _g = lock_boot();
    let bin = include_bytes!("../../../can_host_rx/can_host_rx.bin");
    let sp = u32::from_le_bytes([bin[0], bin[1], bin[2], bin[3]]);
    let pc = u32::from_le_bytes([bin[4], bin[5], bin[6], bin[7]]);
    let sys = WasmSystem::new_svd(include_str!("../../../monox/stm32f407.svd"));
    crate::init_svd_for_test(sys);
    let mut cpu = Cpu::new(sp, pc | 1);
    let mut mem = FlatMemory::new(0x100000, 0x20000);
    mem.load(bin, 0x08000000);
    crate::system::get_uart_output().lock().unwrap().clear();
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    for _ in 0..30 { cpu.run(sys, &mut mem, 100_000); }
    crate::peripherals::can::can_inject(sys, 0x123, 8, b"HELLO!!!", false, false);
    for _ in 0..30 { cpu.run(sys, &mut mem, 100_000); crate::tick_n(100_000); }
    eprintln!("rf0r={:08x} rf1r={:08x}", mem.read32(0x40006400 + 0x1B4 - 0x40), mem.read32(0x40006400 + 0x1B4 - 0x40 + 4));
    for f in 0..2 {
        for sl in 0..3 {
            let b = 0x40006400 + 0x1B0 + (f * 3 + sl) as u32 * 0x10;
            eprintln!("fifo{f} slot{sl}: tir={:08x} tdtr={:08x} tdlr={:08x} tdhr={:08x}",
                mem.read32(b), mem.read32(b + 8), mem.read32(b + 12), mem.read32(b + 16));
        }
    }
    let rir = mem.read32(0x40006400 + 0x1B0);
    let rdlr = mem.read32(0x40006400 + 0x1B8);
    let rdhr = mem.read32(0x40006400 + 0x1BC);
    eprintln!("rir={:08x} id={:03x} rdlr={:08x} rdhr={:08x}", rir, (rir >> 21) & 0x7FF, rdlr, rdhr);
    let u = crate::system::get_uart_output().lock().unwrap().clone();
    eprintln!("uart tail: {:?}", &u[u.len().saturating_sub(80)..]);
}

#[test]
fn lsr_reg_zero_noop() {
    // LSRS-reg with Rs==0 is a no-op (result + carry preserved); the
    // immediate-#0-means-32 rule must NOT apply. DOOM's `(v >> (i*8))`
    // nibble/byte extracts with i==0 returned 0 (patch id 0x120).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    // lsrs r2, r3, r1 (FA T2 LSR-reg: check exact encoding via GAS? use
    // 16-bit T1 LSRS-reg: 000100_xxxx? T1 LSR-reg = 010000_0010_Rm_Rd
    mem.write16(0x20002000, 0x408B); // lsrs r3, r1? (0100000010100011: Rm=1,Rd=3)
    cpu.regs.r[3] = 0x12345678;
    cpu.regs.r[1] = 0;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.r[3], 0x12345678);
}

#[test]
fn fpu_mvfr_and_cpacr_reset() {
    // M4F ID values (M4F TRM) + CPACR/FPSCR/S-file reset state. Grounds the
    // FPU bring-up: guests probe MVFR0-2 at 0xE000EF40-48 and enable CP10/11
    // via CPACR before the first VFP insn.
    let _g = lock_boot();
    let (cpu, mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    assert_eq!(mem.read32(0xE000EF40), 0x1011_0021, "MVFR0");
    assert_eq!(mem.read32(0xE000EF44), 0x1100_0011, "MVFR1");
    assert_eq!(mem.read32(0xE000EF48), 0x0000_0040, "MVFR2");
    assert_eq!(mem.read32(0xE000ED88), 0, "CPACR reset disables FPU");
    assert_eq!(mem.read32(0xE000EF34) & 0xC000_0000, 0xC000_0000, "FPCCR ASPEN|LSPEN");
    assert_eq!(cpu.regs.fpscr, 0, "FPSCR reset");
    assert!(cpu.regs.s.iter().all(|&w| w == 0), "S-file reset");
}

#[test]
fn fpu_nocp_faults_and_latches_ufsr() {
    // vmov.f32 s0, #1.0 (EEB7 0A00) with CPACR==0: loud fault (no delivery
    // in tests) + UFSR NOCP latched (CFSR bit 19). Non-FPU coproc (0xC)
    // faults regardless of CPACR. One locked boot: the latch lives in the
    // process-global model, so no fresh boot may intervene before the read.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    mem.write32(0xE000ED88, 0); // do not rely on reset: parallel tests share SYS
    mem.write16(0x20002000, 0xEEB7);
    mem.write16(0x20002002, 0x0A00);
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_some(), "FPU insn without CPACR must fault");
    assert_ne!(mem.read32(0xE000ED28) & 0x0008_0000, 0, "UFSR NOCP latched");
    // coproc 0xC (not FPU) faults even with CPACR fully enabled.
    mem.write32(0xE000ED88, 0x00F0_0000);
    cpu.fault = None;
    mem.write16(0x20002000, 0xEEC7);
    mem.write16(0x20002002, 0x0C00);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_some(), "non-FPU coproc must fault");
}

/// FPU snippet runner: fresh boot, CPACR full access (explicit: SYS is
/// process-global), S-file/FPSCR seeding, run, return owned state. Model
/// state must not be read after return (another test may re-boot); cpu and
/// RAM results are stable.
fn run_fpu_snippet(code: &[u16], regs: &[(usize, u32)], sregs: &[(usize, u32)], fpscr: u32, n: u32) -> (Cpu, FlatMemory) {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    mem.write32(0xE000ED88, 0x00F0_0000); // CP10+CP11 full access
    for (i, w) in code.iter().enumerate() {
        mem.write16(0x20002000 + (i as u32) * 2, *w);
    }
    for &(r, v) in regs {
        cpu.regs.r[r] = v;
    }
    for &(r, v) in sregs {
        cpu.regs.s[r] = v;
    }
    cpu.regs.fpscr = fpscr;
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, n);
    assert!(cpu.fault.is_none(), "fpu fault: pc={:08x} op1={:04x} op2={:04x}",
        cpu.fault.map(|f| f.pc).unwrap_or(0), cpu.fault.map(|f| f.op1).unwrap_or(0),
        cpu.fault.map(|f| f.op2).unwrap_or(0));
    (cpu, mem)
}

#[test]
fn fpu_vmov_imm() {
    // GAS: vmov.f32 s0,#1.0=EEB7 0A00; vmov.f32 s5,#-0.5=EEFE 2A00;
    // vmov.f32 s0,#6.75=EEB1 0A0B (VFPExpandImm pins).
    let (cpu, _) = run_fpu_snippet(&[0xEEB7, 0x0A00], &[], &[], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x3F80_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEEFE, 0x2A00], &[], &[], 0, 1);
    assert_eq!(cpu.regs.s[5], 0xBF00_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0A0B], &[], &[], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x40D8_0000);
    assert_ne!(cpu.regs.control & 4, 0, "FPCA set by FPU use");
}

#[test]
fn fpu_vmov_reg_and_core() {
    // vmov.f32 s0,s1=EEB0 0A60; vmov s4,r5=EE02 5A10; vmov r4,s5=EE12 4A90.
    let (cpu, _) = run_fpu_snippet(&[0xEEB0, 0x0A60], &[], &[(1, 0x4049_0FDB)], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x4049_0FDB);
    let (cpu, _) = run_fpu_snippet(&[0xEE02, 0x5A10], &[(5, 0xDEAD_BEEF)], &[], 0, 1);
    assert_eq!(cpu.regs.s[4], 0xDEAD_BEEF);
    let (cpu, _) = run_fpu_snippet(&[0xEE12, 0x4A90], &[], &[(5, 0x1234_5678)], 0, 1);
    assert_eq!(cpu.regs.r[4], 0x1234_5678);
    // High regs: vmov s20,r4=EE0A 4A10; vmov r4,s20=EE1A 4A10.
    let (cpu, _) = run_fpu_snippet(&[0xEE0A, 0x4A10], &[(4, 0xA5A5_A5A5)], &[], 0, 1);
    assert_eq!(cpu.regs.s[20], 0xA5A5_A5A5);
    let (cpu, _) = run_fpu_snippet(&[0xEE1A, 0x4A10], &[], &[(20, 0x5A5A_5A5A)], 0, 1);
    assert_eq!(cpu.regs.r[4], 0x5A5A_5A5A);
}

#[test]
fn fpu_vmov_double_pair() {
    // vmov r4,r5,d6=EC55 4B16 (r4=S12, r5=S13); reverse EC45 4B16.
    let (cpu, _) = run_fpu_snippet(&[0xEC55, 0x4B16], &[], &[(12, 0x1111_1111), (13, 0x2222_2222)], 0, 1);
    assert_eq!(cpu.regs.r[4], 0x1111_1111);
    assert_eq!(cpu.regs.r[5], 0x2222_2222);
    let (cpu, _) = run_fpu_snippet(&[0xEC45, 0x4B16], &[(4, 0x3333_3333), (5, 0x4444_4444)], &[], 0, 1);
    assert_eq!(cpu.regs.s[12], 0x3333_3333);
    assert_eq!(cpu.regs.s[13], 0x4444_4444);
}

#[test]
fn fpu_vmrs_vmsr() {
    // vmrs APSR_nzcv,fpscr=EEF1 FA10 imports NZCV only; vmrs r0,fpscr=EEF1
    // 0A10 moves the whole word; vmsr fpscr,r0=EEE1 0A10 is masked.
    let (cpu, _) = run_fpu_snippet(&[0xEEF1, 0xFA10], &[], &[], 0xE000_0000, 1);
    assert_eq!(cpu.regs.xpsr & 0xF000_0000, 0xE000_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEEF1, 0x0A10], &[], &[], 0x1234_5678, 1);
    assert_eq!(cpu.regs.r[0], 0x1234_5678);
    let (cpu, _) = run_fpu_snippet(&[0xEEE1, 0x0A10], &[(0, 0xFFFF_FFFF)], &[], 0, 1);
    assert_eq!(cpu.regs.fpscr, 0xFFC0_01FF, "VMSR writes NZCVQC+AHP/DN/FZ/RMode+enables/flags only");
}

#[test]
fn fpu_vldr_vstr() {
    // GAS: vstr s4,[r5,#8]=ED85 2A02; vldr s4,[r5,#8]=ED95 2A02;
    // vldr s5,[r0]=EDD0 2A00. Store+reload in ONE boot (RAM is zeroed fresh).
    let (cpu, mem) = run_fpu_snippet(
        &[0xED85, 0x2A02, 0xED95, 0x2A02, 0xEDD0, 0x2A00],
        &[(5, 0x2000_3000), (0, 0x2000_3008)], &[(4, 0x4049_0FDB)], 0, 3);
    assert_eq!(mem.read32(0x2000_3008), 0x4049_0FDB, "vstr wrote RAM");
    assert_eq!(cpu.regs.s[4], 0x4049_0FDB, "reload via vldr s4");
    assert_eq!(cpu.regs.s[5], 0x4049_0FDB, "high-reg vldr s5");
    // Double: vldr d1,[r0]=ED90 1B00 / vstr d1,[r0]=ED80 1B00 round-trip.
    let (cpu, mem) = run_fpu_snippet(&[0xED80, 0x1B00, 0xED90, 0x1B00], &[(0, 0x2000_3100)], &[(2, 0xAAAAAAAA), (3, 0xBBBB_BBBB)], 0, 2);
    assert_eq!(mem.read32(0x2000_3100), 0xAAAAAAAA);
    assert_eq!(mem.read32(0x2000_3104), 0xBBBB_BBBB);
    assert_eq!((cpu.regs.s[2], cpu.regs.s[3]), (0xAAAAAAAA, 0xBBBB_BBBB));
}

#[test]
fn fpu_vldm_vstm_push_pop() {
    // vstmia r4,{s4-s7}=EC84 2A04 / vldmia r4,{s4-s7}=EC94 2A04;
    // vpush {s0-s3}=ED2D 0A04 / vpop {s0-s3}=ECBD 0A04; D-list EC84
    // 2B04/EC94 2B04; writeback vstmia r4!,{s16-s19}=ECA4 8A04.
    let seeds = [(4, 0x1111_1111), (5, 0x2222_2222), (6, 0x3333_3333), (7, 0x4444_4444)];
    let (cpu, mem) = run_fpu_snippet(&[0xEC84, 0x2A04], &[(4, 0x2000_3200)], &seeds, 0, 1);
    assert_eq!((mem.read32(0x2000_3200), mem.read32(0x2000_320C)), (0x1111_1111, 0x4444_4444));
    assert_eq!(cpu.regs.r[4], 0x2000_3200, "no writeback without !");
    let (cpu, _) = run_fpu_snippet(
        &[0xEC84, 0x2A04, 0xEC94, 0x2A04], &[(4, 0x2000_3300)], &seeds, 0, 2);
    assert_eq!((cpu.regs.s[4], cpu.regs.s[5], cpu.regs.s[6], cpu.regs.s[7]),
        (0x1111_1111, 0x2222_2222, 0x3333_3333, 0x4444_4444), "store+reload round-trip");
    // push/pop round-trip on the real stack.
    let (cpu, _) = run_fpu_snippet(
        &[0xED2D, 0x0A04, 0xECBD, 0x0A04], &[(13, 0x2000_4000)],
        &[(0, 0xAAAAAAAA), (1, 0xBBBB_BBBB), (2, 0xCCCC_CCCC), (3, 0xDDDD_DDDD)], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.s[1], cpu.regs.s[2], cpu.regs.s[3]),
        (0xAAAAAAAA, 0xBBBB_BBBB, 0xCCCC_CCCC, 0xDDDD_DDDD));
    assert_eq!(cpu.regs.r[13], 0x2000_4000, "push+pop restores SP");
    // D-list + writeback.
    let (cpu, _) = run_fpu_snippet(
        &[0xEC84, 0x2B04, 0xEC94, 0x2B04], &[(4, 0x2000_3400)],
        &[(4, 0xAAAAAAAA), (5, 0xBBBB_BBBB)], 0, 2);
    assert_eq!((cpu.regs.s[4], cpu.regs.s[5]), (0xAAAAAAAA, 0xBBBB_BBBB), "d2-d3 round-trip");
    let (cpu, _) = run_fpu_snippet(&[0xECA4, 0x8A04], &[(4, 0x2000_3500)], &[(16, 1)], 0, 1);
    assert_eq!(cpu.regs.r[4], 0x2000_3510, "vstmia! writes back +16");
}

#[test]
fn fpu_rejects() {
    // sz=1 data-processing (no f64 on FPv4-SP), DB without writeback, bad
    // P/U combo, VLDM Rn=PC, D-list overflow, VMOV Rt=PC.
    for code in [
        [0xEE30u16, 0x0B81u16], // vadd sz=1
        [0xEE80, 0x0AD1u16],    // opc1 8 + op 1: no such op
        [0xEE30, 0x0A91u16],    // vadd shape with op2[4]=1 (reserved)
        [0xEEB6, 0x0A60u16],    // B-group opc2=6: no such op
        [0xED00, 0x2A04],       // DB store without ! (no such encoding)
        [0xEC50, 0x0A04],       // P=1,U=1: no such mode (also VMOV-2reg shape? op2 0A04: (0x04&0xD0)=0x00 != 0x10, falls to VLDM -> bad P/U)
        [0xEC9F, 0x0A04],       // vldmia pc,{s0-s3}
        [0xEC94, 0xEB08],       // vldmia r4,{d14-d17}: d17 > 15
        [0xEE10, 0xFA10],       // vmov pc,s0
    ] {
        let _g = lock_boot();
        let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
        mem.write32(0xE000ED88, 0x00F0_0000);
        for (i, w) in code.iter().enumerate() {
            mem.write16(0x20002000 + (i as u32) * 2, *w);
        }
        cpu.regs.r[15] = 0x20002001;
        let sys = crate::sys();
        cpu.run(sys, &mut mem, 1);
        assert!(cpu.fault.is_some(), "must fault: {:04x} {:04x}", code[0], code[1]);
    }
}


#[test]
fn fpu_arith_basic() {
    // GAS: vadd s4,s5,s6=EE32 2A83; vsub=EE32 2AC3; vmul s4,s5,s6=EE22
    // 2A83; vdiv=EE82 2A83; vmla s0,s1,s2=EE00 0A81; vmls=EE00 0AC1;
    // vnmla=EE10 0AC1.
    let f = |x: f32| x.to_bits();
    let (cpu, _) = run_fpu_snippet(&[0xEE32, 0x2A83], &[], &[(5, f(2.5)), (6, f(1.5))], 0, 1);
    assert_eq!(cpu.regs.s[4], f(4.0));
    assert_eq!(cpu.regs.fpscr & 0x1F, 0, "exact add sets no flags");
    let (cpu, _) = run_fpu_snippet(&[0xEE32, 0x2AC3], &[], &[(5, f(2.5)), (6, f(5.0))], 0, 1);
    assert_eq!(cpu.regs.s[4], f(-2.5));
    let (cpu, _) = run_fpu_snippet(&[0xEE22, 0x2A83], &[], &[(5, f(2.0)), (6, f(1.5))], 0, 1);
    assert_eq!(cpu.regs.s[4], f(3.0));
    let (cpu, _) = run_fpu_snippet(&[0xEE82, 0x2A83], &[], &[(5, f(7.0)), (6, f(2.0))], 0, 1);
    assert_eq!(cpu.regs.s[4], f(3.5));
    let (cpu, _) = run_fpu_snippet(&[0xEE00, 0x0A81], &[], &[(0, f(1.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(7.0), "vmla unfused");
    let (cpu, _) = run_fpu_snippet(&[0xEE00, 0x0AC1], &[], &[(0, f(10.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(4.0), "vmls");
    let (cpu, _) = run_fpu_snippet(&[0xEE10, 0x0AC1], &[], &[(0, f(1.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-7.0), "vnmla");
}

#[test]
fn fpu_arith_specials() {
    let f = |x: f32| x.to_bits();
    // Overflow: max+max -> +inf + OFC|IXC.
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, f(f32::MAX)), (2, f(f32::MAX))], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x7F80_0000);
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x04 | 0x10, "OFC|IXC");
    // Divide by zero: 1/0 -> +inf + DZC; 0/0 -> NaN + IOC; inf-inf -> NaN + IOC.
    let (cpu, _) = run_fpu_snippet(&[0xEE80, 0x0A81], &[], &[(1, f(1.0)), (2, 0)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7F80_0000, 0x02));
    let (cpu, _) = run_fpu_snippet(&[0xEE80, 0x0A81], &[], &[(1, 0), (2, 0)], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x01, "0/0 IOC");
    assert_eq!(cpu.regs.s[0], 0x7FC0_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, 0x7F80_0000), (2, 0xFF80_0000)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0000, 0x01), "inf-inf IOC");
    // 0*inf -> NaN + IOC.
    let (cpu, _) = run_fpu_snippet(&[0xEE20, 0x0A81], &[], &[(1, 0), (2, 0x7F80_0000)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0000, 0x01));
    // QNaN propagates quietly (same bits, no flag); SNaN -> IOC + quieted.
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, 0x7FC0_1234), (2, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_1234, 0));
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, 0x7F80_0001), (2, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0001, 0x01), "SNaN quieted + IOC");
    // DN=1: any NaN result is the default NaN.
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, 0x7FC0_1234), (2, f(1.0))], 1 << 25, 1);
    assert_eq!(cpu.regs.s[0], 0x7FC0_0000);
    // FZ=1 flushes subnormal inputs: 2^-127 * 2 -> +0, no flags.
    let (cpu, _) = run_fpu_snippet(&[0xEE20, 0x0A81], &[], &[(1, 0x0040_0000), (2, f(2.0))], 1 << 24, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0, 0));
    // Same without FZ: 2^-126 * 0.5 -> 2^-127 subnormal -> UFC (+IXC).
    let (cpu, _) = run_fpu_snippet(&[0xEE20, 0x0A81], &[], &[(1, 0x0080_0000), (2, f(0.5))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x0040_0000, 0x08 | 0x10));
    // Exact min-normal needs no flags: 2^-127 * 2 -> 2^-126, exact + normal.
    let (cpu, _) = run_fpu_snippet(&[0xEE20, 0x0A81], &[], &[(1, 0x0040_0000), (2, f(2.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x0080_0000, 0));
}

#[test]
fn fpu_sqrt_abs_neg() {
    let f = |x: f32| x.to_bits();
    // GAS: vsqrt s0,s1=EEB1 0AE0; vabs=EEB0 0AE0; vneg s0,s1=EEB1 0A60.
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0AE0], &[], &[(1, f(4.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(2.0));
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0AE0], &[], &[(1, f(2.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x3FB5_04F3, "sqrt(2)");
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0AE0], &[], &[(1, f(-1.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0000, 0x01), "sqrt(-1) IOC");
    let (cpu, _) = run_fpu_snippet(&[0xEEB0, 0x0AE0], &[], &[(1, f(-3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(3.0));
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0A60], &[], &[(1, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-1.0));
    // ABS/NEG never raise, even on SNaN (bit ops).
    let (cpu, _) = run_fpu_snippet(&[0xEEB0, 0x0AE0], &[], &[(1, 0xFF80_0001)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7F80_0001, 0));
}

#[test]
fn fpu_vcmp() {
    let f = |x: f32| x.to_bits();
    // GAS: vcmp s0,s1=EEB4 0A60; vcmp s0,#0=EEB5 0A40; vmrs=EEF1 FA10.
    // LT -> 0x8, EQ -> 0x6, GT -> 0x2, unordered -> 0x3 in FPSCR+APSR.
    for (a, b, want) in [(1.0f32, 2.0, 0x8u32), (1.0, 1.0, 0x6), (2.0, 1.0, 0x2)] {
        let (cpu, _) = run_fpu_snippet(
            &[0xEEB4, 0x0A60, 0xEEF1, 0xFA10], &[], &[(0, f(a)), (1, f(b))], 0, 2);
        assert_eq!(cpu.regs.fpscr & 0xF000_0000, want << 28, "fpscr {a} vs {b}");
        assert_eq!(cpu.regs.xpsr & 0xF000_0000, want << 28, "apsr {a} vs {b}");
    }
    // -0 == +0.
    let (cpu, _) = run_fpu_snippet(&[0xEEB5, 0x0A40], &[], &[(0, f(-0.0))], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0xF000_0000, 0x6000_0000);
    // QNaN: unordered, no flag. SNaN: unordered + IOC.
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x0A60], &[], &[(0, 0x7FC0_0000), (1, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.fpscr & 0xF000_0000, cpu.regs.fpscr & 0x1F), (0x3000_0000, 0));
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x0A60], &[], &[(0, 0x7F80_0001), (1, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.fpscr & 0xF000_0000, cpu.regs.fpscr & 0x1F), (0x3000_0000, 0x01));
}

#[test]
fn fpu_vcvt_int() {
    let f = |x: f32| x.to_bits();
    // GAS: vcvt.s32.f32=EEBD 0AC0; vcvt.u32.f32=EEBC 0AC0;
    // vcvt.f32.s32=EEB8 0AC0; vcvt.f32.u32=EEB8 0A40.
    // RNE ties-to-even: 1.5->2, 2.5->2 (not 3!), -1.5->-2, 0.5->0.
    for (x, want) in [(1.5f32, 2u32), (2.5, 2), (-1.5, 0xFFFF_FFFEu32), (0.5, 0), (2.6, 3)] {
        let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, f(x))], 0, 1);
        assert_eq!(cpu.regs.s[0], want, "s32({x})");
    }
    let (cpu, _) = run_fpu_snippet(&[0xEEBC, 0x0AC0], &[], &[(0, f(1.5))], 0, 1);
    assert_eq!(cpu.regs.s[0], 2);
    // Invalid: NaN->0, +overflow saturates, -overflow saturates, all + IOC.
    let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, 0x7FC0_0000)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0, 0x01));
    let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, f(1e20))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FFF_FFFF, 0x01));
    let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, f(-1e20))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x8000_0000, 0x01));
    let (cpu, _) = run_fpu_snippet(&[0xEEBC, 0x0AC0], &[], &[(0, f(4294967295.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0xFFFF_FFFF, 0x01), "u32 range");
    // Exact: i32::MIN is valid and exact (no IXC); 1.5 is inexact (IXC).
    let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, f(-2147483648.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x8000_0000, 0));
    let (cpu, _) = run_fpu_snippet(&[0xEEBD, 0x0AC0], &[], &[(0, f(1.5))], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x10, "inexact IXC");
    // int->float: exact (no IXC) vs inexact (IXC).
    let (cpu, _) = run_fpu_snippet(&[0xEEB8, 0x0AC0], &[], &[(0, 42)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (f(42.0), 0));
    let (cpu, _) = run_fpu_snippet(&[0xEEB8, 0x0AC0], &[], &[(0, 0x1234_5678)], 0, 1);
    assert_eq!(cpu.regs.s[0], (0x1234_5678i32 as f32).to_bits());
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x10);
    // RMode via VMSR: toward-zero turns 1.9 into 1 (RNE would give 2).
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEEBD, 0x0AC0], &[(0, 3 << 22)], &[(0, f(1.9))], 0, 2);
    assert_eq!(cpu.regs.s[0], 1);
}

#[test]
fn fpu_vcvt_fixed() {
    let f = |x: f32| x.to_bits();
    // GAS: vcvt.f32.s32 #16=EEBA 0AC8; vcvt.s32.f32 #16=EEBE 0AC8;
    // vcvt.f32.u32 #16=EEBB 0AC8; vcvt.f32.s32 #1=EEBA 0AEF.
    let (cpu, _) = run_fpu_snippet(&[0xEEBA, 0x0AC8], &[], &[(0, 0x0001_0000)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "Q16.16 1.0");
    let (cpu, _) = run_fpu_snippet(&[0xEEBE, 0x0AC8], &[], &[(0, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x0001_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEEBA, 0x0AEF], &[], &[(0, 3)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.5), "#1 frac");
    let (cpu, _) = run_fpu_snippet(&[0xEEBB, 0x0AC8], &[], &[(0, 0x0001_0000)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "unsigned Q16.16");
    // Negative fixed stays negative; overflow saturates + IOC.
    let (cpu, _) = run_fpu_snippet(&[0xEEBA, 0x0AC8], &[], &[(0, 0xFFFF_0000u32)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-1.0));
    let (cpu, _) = run_fpu_snippet(&[0xEEBE, 0x0AC8], &[], &[(0, f(100000.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FFF_FFFF, 0x01), "s32 #16 saturate");
}

#[test]
fn fpu_vcvt_f16() {
    let f = |x: f32| x.to_bits();
    // GAS: vcvtb.f32.f16=EEB2 0A60; vcvtt.f32.f16=EEB2 0AE0;
    // vcvtb.f16.f32=EEB3 0A60; vcvtt.f16.f32=EEB3 0AE0.
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0A60], &[], &[(1, 0x3C00)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "bottom half");
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0AE0], &[], &[(1, 0x3C00_0000)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "top half");
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0A60], &[], &[(1, 0x0001)], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x3380_0000, "f16 subnormal 2^-24");
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0A60], &[], &[(1, 0x7C00)], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x7F80_0000, "f16 inf");
    // f32->f16: 1.0->0x3C00 (low half, high preserved), 100000->inf+OFC|IXC.
    let (cpu, _) = run_fpu_snippet(&[0xEEB3, 0x0A60], &[], &[(0, 0xABCD_0000), (1, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0xABCD_3C00, 0));
    let (cpu, _) = run_fpu_snippet(&[0xEEB3, 0x0AE0], &[], &[(1, f(6.75))], 0, 1);
    assert_eq!(cpu.regs.s[0] >> 16, 0x46C0, "top-half 6.75");
    let (cpu, _) = run_fpu_snippet(&[0xEEB3, 0x0A60], &[], &[(1, f(100000.0))], 0, 1);
    assert_eq!((cpu.regs.s[0] & 0xFFFF, cpu.regs.fpscr & 0x1F), (0x7C00, 0x04 | 0x10));
    // Tiny: 1e-5 -> f16 subnormal 0xA8 + UFC|IXC.
    let (cpu, _) = run_fpu_snippet(&[0xEEB3, 0x0A60], &[], &[(1, f(1e-5))], 0, 1);
    assert_eq!((cpu.regs.s[0] & 0xFFFF, cpu.regs.fpscr & 0x1F), (0x00A8, 0x08 | 0x10));
    // f16 SNaN -> IOC (both directions); f32 SNaN narrows quieted.
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0A60], &[], &[(1, 0x7C01)], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x01);
    assert_eq!(cpu.regs.s[0] & 0x7FFF_FFFF, 0x7FC0_2000, "SNaN widened+quieted");
    let (cpu, _) = run_fpu_snippet(&[0xEEB3, 0x0A60], &[], &[(1, 0x7F80_0001)], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0x1F, 0x01);
    assert_eq!((cpu.regs.s[0] >> 10) & 0x1F, 0x1F, "narrowed NaN exp all-ones");
    assert_ne!(cpu.regs.s[0] & 0x3FF, 0, "narrowed NaN keeps payload");
}


#[test]
fn fpu_fma_fused() {
    let f = |x: f32| x.to_bits();
    // GAS (fpu11.s): vfma s0,s1,s2=EEA0 0A81; vfms=EEA0 0AC1;
    // vfnma s0,s1,s2=EE90 0AC1; vfnms=EE90 0A81.
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, f(1.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(7.0));
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0AC1], &[], &[(0, f(10.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(4.0), "vfms");
    let (cpu, _) = run_fpu_snippet(&[0xEE90, 0x0AC1], &[], &[(0, f(1.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-7.0), "vfnma");
    let (cpu, _) = run_fpu_snippet(&[0xEE90, 0x0A81], &[], &[(0, f(10.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-4.0), "vfnms");
    // THE fused-vs-unfused discriminator: a=b=1+2^-23, acc=-(1+2^-22).
    // Unfused rounds a*b to 1+2^-22 first, then acc+p = 0. Fused keeps the
    // exact 2^-46 square term: result 2^-46 (exp 81-127, 0x28800000).
    let e = 2f32.powi(-23);
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, f(-(1.0 + 2.0 * e))), (1, f(1.0 + e)), (2, f(1.0 + e))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x2880_0000, 0), "fused keeps eps^2");
    // Same inputs through UNfused vmla give exactly 0 (control case).
    let (cpu, _) = run_fpu_snippet(&[0xEE00, 0x0A81], &[], &[(0, f(-(1.0 + 2.0 * e))), (1, f(1.0 + e)), (2, f(1.0 + e))], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x0000_0000, "unfused rounds first");
    // Specials mirror unfused: 0*inf -> IOC; inf-inf -> IOC; QNaN quiet.
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, f(1.0)), (1, 0), (2, 0x7F80_0000)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0000, 0x01));
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, 0xFF80_0000), (1, 0x7F80_0000), (2, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_0000, 0x01), "inf-inf IOC");
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, 0x7FC0_1234), (1, f(1.0)), (2, f(2.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7FC0_1234, 0));
    // Overflow: max*2 + max -> +inf + OFC|IXC.
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, f(f32::MAX)), (1, f(f32::MAX)), (2, f(2.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7F80_0000, 0x04 | 0x10));
    // Exact cancellation: acc == a*b -> +0, no flags.
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0AC1], &[], &[(0, f(6.0)), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0, 0), "vfms exact zero");
    // Zero addend: single-rounding product (vfms negates it).
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0A81], &[], &[(0, 0), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(6.0));
    let (cpu, _) = run_fpu_snippet(&[0xEEA0, 0x0AC1], &[], &[(0, 0), (1, f(2.0)), (2, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-6.0), "vfms zero-acc negates");
}

#[test]
fn fpu_vcmpe() {
    let f = |x: f32| x.to_bits();
    // GAS (fpu12.s): vcmpe s0,s1=EEB4 0AE0; vcmpe s4,s5=EEB4 2AE2;
    // vcmpe s0,#0=EEB5 0AC0. E-form raises IOC on ANY NaN (quiet too).
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x0AE0], &[], &[(0, f(1.0)), (1, f(2.0))], 0, 1);
    assert_eq!((cpu.regs.fpscr & 0xF000_0000, cpu.regs.fpscr & 0x1F), (0x8000_0000, 0), "ordered LT, no flag");
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x0AE0], &[], &[(0, 0x7FC0_0000), (1, f(1.0))], 0, 1);
    assert_eq!((cpu.regs.fpscr & 0xF000_0000, cpu.regs.fpscr & 0x1F), (0x3000_0000, 0x01), "QNaN unordered + IOC");
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x2AE2], &[], &[(4, f(2.0)), (5, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0xF000_0000, 0x2000_0000, "high-reg GT");
    let (cpu, _) = run_fpu_snippet(&[0xEEB5, 0x0AC0], &[], &[(0, f(0.0))], 0, 1);
    assert_eq!((cpu.regs.fpscr & 0xF000_0000, cpu.regs.fpscr & 0x1F), (0x6000_0000, 0), "E-#0 equal");
}

#[test]
fn fpu_high_regs_and_even_sm() {
    // The D=1 (odd-high dest) and M=0 (even source) space the first FPU
    // cut missed: opc used to include D, and the B-group op-nibble baked
    // in M=1. GAS vectors: fpu13.s + fpu14.s.
    let f = |x: f32| x.to_bits();
    // vadd s17,s18,s19=EE79 8A29 (the firmware's EE76-class bug).
    let (cpu, _) = run_fpu_snippet(&[0xEE79, 0x8A29], &[], &[(18, f(1.5)), (19, f(2.5))], 0, 1);
    assert_eq!(cpu.regs.s[17], f(4.0));
    // vsub s31,s30,s29=EE7F FA6E (D,N,M all set).
    let (cpu, _) = run_fpu_snippet(&[0xEE7F, 0xFA6E], &[], &[(30, f(5.0)), (29, f(1.5))], 0, 1);
    assert_eq!(cpu.regs.s[31], f(3.5));
    // vfma s21,s22,s23=EEEB AA2B.
    let (cpu, _) = run_fpu_snippet(&[0xEEEB, 0xAA2B], &[], &[(21, f(1.0)), (22, f(2.0)), (23, f(3.0))], 0, 1);
    assert_eq!(cpu.regs.s[21], f(7.0));
    // vsqrt s17,s18=EEF1 8AC9 (op1 collides with VMRS shape; op2lo decides).
    let (cpu, _) = run_fpu_snippet(&[0xEEF1, 0x8AC9], &[], &[(18, f(9.0))], 0, 1);
    assert_eq!(cpu.regs.s[17], f(3.0));
    // vcmp s17,s18=EEF4 8A49; vmov s17,s18=EEF0 8A49.
    let (cpu, _) = run_fpu_snippet(&[0xEEF4, 0x8A49], &[], &[(17, f(1.0)), (18, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0xF000_0000, 0x6000_0000);
    let (cpu, _) = run_fpu_snippet(&[0xEEF0, 0x8A49], &[], &[(18, 0xDEAD_BEEF)], 0, 1);
    assert_eq!(cpu.regs.s[17], 0xDEAD_BEEF);
    // vcvt.s32.f32 s17,s18=EEFD 8AC9 (old gate faulted M=1 here).
    let (cpu, _) = run_fpu_snippet(&[0xEEFD, 0x8AC9], &[], &[(18, f(2.5))], 0, 1);
    assert_eq!(cpu.regs.s[17], 2);
    // M=0 forms the old nibble match missed.
    let (cpu, _) = run_fpu_snippet(&[0xEEB0, 0x0A40], &[], &[(0, 0x1234_5678)], 0, 1);
    assert_eq!(cpu.regs.s[0], 0x1234_5678, "vmov M=0");
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0A40], &[], &[(0, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(-1.0), "vneg M=0");
    let (cpu, _) = run_fpu_snippet(&[0xEEB0, 0x0AC0], &[], &[(0, f(-1.0))], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "vabs M=0");
    let (cpu, _) = run_fpu_snippet(&[0xEEB4, 0x0A40], &[], &[(0, f(1.0))], 0, 1);
    assert_eq!(cpu.regs.fpscr & 0xF000_0000, 0x6000_0000, "vcmp M=0");
    let (cpu, _) = run_fpu_snippet(&[0xEEB2, 0x0A40], &[], &[(0, 0x3C00)], 0, 1);
    assert_eq!(cpu.regs.s[0], f(1.0), "vcvtb M=0");
    // Unsigned int->float (sign is op2[7], not op2[6]) + M=1 source.
    let (cpu, _) = run_fpu_snippet(&[0xEEB8, 0x2A62], &[], &[(5, 0xFFFF_FFFF)], 0, 1);
    assert_eq!((cpu.regs.s[4], cpu.regs.fpscr & 0x1F), (0x4F80_0000, 0x10), "u32 max -> 2^32");
    let (cpu, _) = run_fpu_snippet(&[0xEEB8, 0x2AE2], &[], &[(5, 42)], 0, 1);
    assert_eq!((cpu.regs.s[4], cpu.regs.fpscr & 0x1F), (f(42.0), 0), "s32 M=1 exact");
}

#[test]
fn fpu_lazy_reserve_and_return() {
    // CONTROL.FPCA=1 + FPCCR.ASPEN (reset) => PendSV reserves the 26-word
    // extended frame: FPSCR stacked at +96, S0-S15 untouched (lazy),
    // LSPACT=1, FPCAR=sp+32, LR=0xFFFFFFE9. Return restores FPSCR + SP.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    cpu.regs.s[0] = 0xAAAAAAAA;
    cpu.regs.s[15] = 0xBBBB_BBBB;
    cpu.regs.fpscr = 0x1234_5678;
    cpu.regs.control |= 4; // FPCA: thread uses the FPU
    let sp0 = cpu.regs.r[13];
    cpu.take_exception(sys, &mut mem, -2); // PendSV
    assert_eq!(cpu.regs.r[14], 0xFFFF_FFE9, "LR carries FType=0");
    let sp = cpu.regs.r[13];
    assert_eq!(sp, sp0.wrapping_sub(104), "26-word frame");
    assert_eq!(mem.read32(sp.wrapping_add(96)), 0x1234_5678, "FPSCR stacked");
    assert_eq!(mem.read32(sp.wrapping_add(32)), 0, "S0 lazy (untouched)");
    assert_ne!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "LSPACT set");
    assert_eq!(sys.p.read(sys, 0xE000EF38, 4), sp.wrapping_add(32), "FPCAR at S0 slot");
    // Handler clobbers FPSCR; return must restore it (lazy: no S traffic).
    cpu.regs.fpscr = 0;
    cpu.regs.s[0] = 0;
    assert!(cpu.exception_return(sys, &mut mem, 0xFFFF_FFE9, 0x2000_2000));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.fpscr, 0x1234_5678, "FPSCR restored");
    assert_eq!(cpu.regs.s[0], 0, "S0 not restored (was never stacked)");
    assert_eq!(cpu.regs.r[13], sp0, "SP restored");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "LSPACT clear after pop");
}

#[test]
fn fpu_lazy_first_use_stacks() {
    // First handler FPU use stacks the LIVE S regs into FPCAR, clears
    // LSPACT; return then restores the thread's values (over handler mods).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    cpu.regs.s[0] = 0xAAAAAAAA;
    cpu.regs.s[15] = 0xBBBB_BBBB;
    cpu.regs.control |= 4;
    cpu.take_exception(sys, &mut mem, -2);
    let fpcar = sys.p.read(sys, 0xE000EF38, 4);
    // Run a real FPU insn in handler mode: vmov.f32 s0,#1.0.
    mem.write16(0x20002000, 0xEEB7);
    mem.write16(0x20002002, 0x0A00);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.s[0], 0x3F80_0000, "insn executed");
    assert_eq!(mem.read32(fpcar), 0xAAAAAAAA, "live S0 stacked first");
    assert_eq!(mem.read32(fpcar.wrapping_add(60)), 0xBBBB_BBBB, "live S15 stacked");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "LSPACT cleared by use");
    // Return with LSPACT=0: full unstack overwrites the handler's s0=1.0
    // with the thread's seeded value.
    assert!(cpu.exception_return(sys, &mut mem, 0xFFFF_FFE9, 0x2000_2000));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.s[0], 0xAAAAAAAA, "thread S0 restored");
    assert_eq!(cpu.regs.s[15], 0xBBBB_BBBB, "thread S15 restored");
}

#[test]
fn fpu_eager_stacks_at_entry() {
    // LSPEN=0: S0-S15 + FPSCR land in the frame at entry; LSPACT stays 0.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    mem.write32(0xE000EF34, 0x8000_0000); // ASPEN only (no LSPEN)
    cpu.regs.s[3] = 0xCCCC_CCCC;
    cpu.regs.fpscr = 0x1122_3344;
    cpu.regs.control |= 4;
    cpu.take_exception(sys, &mut mem, -2);
    assert_eq!(cpu.regs.r[14], 0xFFFF_FFE9, "FType=0 even eager");
    let sp = cpu.regs.r[13];
    assert_eq!(mem.read32(sp.wrapping_add(32 + 12)), 0xCCCC_CCCC, "S3 stacked at entry");
    assert_eq!(mem.read32(sp.wrapping_add(96)), 0x1122_3344, "FPSCR stacked at entry");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "no LSPACT when eager");
    cpu.regs.s[3] = 0;
    assert!(cpu.exception_return(sys, &mut mem, 0xFFFF_FFE9, 0x2000_2000));
    assert_eq!(cpu.regs.s[3], 0xCCCC_CCCC, "eager frame restores");
}

#[test]
fn fpu_no_fpca_unchanged() {
    // No FPU use: classic 8-word frame, classic LR, model FP regs untouched.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    let sp0 = cpu.regs.r[13];
    cpu.take_exception(sys, &mut mem, -2);
    assert_eq!(cpu.regs.r[14], 0xFFFF_FFF9, "classic EXC_RETURN");
    assert_eq!(cpu.regs.r[13], sp0.wrapping_sub(32), "8-word frame");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "no LSPACT");
    assert_eq!(sys.p.read(sys, 0xE000EF38, 4), 0, "no FPCAR");
    assert!(cpu.exception_return(sys, &mut mem, 0xFFFF_FFF9, 0x2000_2000));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.r[13], sp0);
}

#[test]
fn fpu_nested_frames() {
    // Outer PendSV reserve, inner SysTick reserve, inner use+return, outer
    // use+return — every S modification goes through a real FPU snippet
    // (the only way guest code CAN touch S-regs, so the hook always fires).
    // Thread 0x11111111 -> outer use stacks it, sets 1.0 -> inner use
    // stacks 1.0, sets -0.5 -> returns unwind exactly.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    // Thread uses the FPU: vmov s0,r0 (EE00 0A10) with r0=0x11111111.
    cpu.regs.r[0] = 0x1111_1111;
    mem.write16(0x20002000, 0xEE00);
    mem.write16(0x20002002, 0x0A10);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert_eq!(cpu.regs.s[0], 0x1111_1111);
    assert_ne!(cpu.regs.control & 4, 0, "FPCA set by use");
    cpu.take_exception(sys, &mut mem, -2); // outer PendSV
    let outer_fpcar = sys.p.read(sys, 0xE000EF38, 4);
    let outer_lr = cpu.regs.r[14];
    assert_eq!(outer_lr, 0xFFFF_FFE9);
    // Outer handler: vmov.f32 s0,#1.0 stacks 0x11111111, sets 1.0.
    mem.write16(0x20002000, 0xEEB7);
    mem.write16(0x20002002, 0x0A00);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.s[0], 0x3F80_0000);
    assert_eq!(mem.read32(outer_fpcar), 0x1111_1111, "thread value stacked");
    cpu.take_exception(sys, &mut mem, -1); // inner SysTick
    let inner_fpcar = sys.p.read(sys, 0xE000EF38, 4);
    assert_ne!(inner_fpcar, outer_fpcar, "inner frame is distinct");
    let inner_lr = cpu.regs.r[14];
    // Inner handler: vmov.f32 s0,#-0.5 (EEBE 0A00, GAS fpu15.s)
    // stacks live 1.0, sets -0.5.
    mem.write16(0x20002000, 0xEEBE);
    mem.write16(0x20002002, 0x0A00);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.s[0], 0xBF00_0000);
    assert_eq!(mem.read32(inner_fpcar), 0x3F80_0000, "outer-live stacked");
    assert!(cpu.exception_return(sys, &mut mem, inner_lr, 0x2000_2000));
    assert_eq!(cpu.regs.s[0], 0x3F80_0000, "outer-live restored");
    assert_eq!(sys.p.read(sys, 0xE000EF38, 4), outer_fpcar, "outer FPCAR restored");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "outer LSPACT consumed");
    assert!(cpu.exception_return(sys, &mut mem, outer_lr, 0x2000_2000));
    assert_eq!(cpu.regs.s[0], 0x1111_1111, "thread value restored");
    assert_eq!(sys.p.read(sys, 0xE000EF34, 4) & 1, 0, "LSPACT clear at end");
}

#[test]
fn fpu_rmode_arith() {
    // Directed rounding via VMSR RMode (vmsr=EEE1 0A10): 1.0 + 2^-24 is
    // exact-tie-ish — RNE gives 1.0 (even), +inf gives nextUp, -inf/zero
    // give 1.0, all with IXC (except the exact RNE case... 1.0 is inexact
    // too: the true sum is not representable, so IXC fires everywhere).
    let f = |x: f32| x.to_bits();
    let tiny = 0x3380_0000u32; // 2^-24
    // RNE baseline.
    let (cpu, _) = run_fpu_snippet(&[0xEE30, 0x0A81], &[], &[(1, f(1.0)), (2, tiny)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (f(1.0), 0x10));
    // Toward +inf: nextUp(1.0) = 1+2^-23.
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE30, 0x0A81], &[(0, 1 << 22)], &[(1, f(1.0)), (2, tiny)], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x3F80_0001, 0x10));
    // Toward -inf and toward zero: 1.0.
    for rmode in [2u32, 3u32] {
        let (cpu, _) = run_fpu_snippet(
            &[0xEEE1, 0x0A10, 0xEE30, 0x0A81], &[(0, rmode << 22)], &[(1, f(1.0)), (2, tiny)], 0, 2);
        assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (f(1.0), 0x10), "rmode {rmode}");
    }
    // Mul directed: (1+2^-22)^2... use (1+2^-23)*(1+2^-23): RNE rounds the
    // 2^-46 square term away -> 1+2^-22; -inf keeps it too (above); the
    // discriminating mode is toward-zero/+inf only when below... instead
    // pin exactness: 1.5*1.5 = 2.25 exact in all modes, no IXC.
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE20, 0x0A81], &[(0, 1 << 22)], &[(1, f(1.5)), (2, f(1.5))], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (f(2.25), 0));
    // Div/sqrt directed, expectations computed from host f64 (correctly
    // rounded) + next_up/down — execution-grounded, not hand-derived.
    let t = 1f64 / 3f64;
    let rne = t as f32;
    let up = if (rne as f64) < t { rne.next_up() } else { rne };
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, 1 << 22)], &[(1, f(1.0)), (2, f(3.0))], 0, 2);
    assert_eq!(cpu.regs.s[0], up.to_bits(), "div toward +inf");
    assert_ne!(cpu.regs.fpscr & 0x10, 0, "div inexact IXC");
    let t = 2f64.sqrt();
    let rne = t as f32;
    let up = if (rne as f64) < t { rne.next_up() } else { rne };
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEEB1, 0x0AE0], &[(0, 1 << 22)], &[(1, f(2.0))], 0, 2);
    assert_eq!(cpu.regs.s[0], up.to_bits(), "sqrt toward +inf");
}

#[test]
fn fpu_irq_firmware() {
    // Full firmware run: fpu_irq_test.bin with real SysTick delivery.
    // The loop pumps model ticks (what the JS driver's post-step tick does);
    // without it INSTRUCTION_COUNT never advances for the model and no IRQ
    // fires (a run that finishes with count 0 is vacuous — asserted below).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../fpu_irq_test/fpu_irq_test.bin"));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    let mut uart = String::new();
    for _ in 0..600 {
        cpu.run(sys, &mut mem, 100_000);
        sys.tick();
        uart.push_str(&crate::system::get_uart_output().lock().unwrap().clone());
        crate::system::get_uart_output().lock().unwrap().clear();
        if uart.contains("FPU IRQ done") {
            break;
        }
        if cpu.fault.is_some() {
            break;
        }
    }
    assert!(uart.contains("FPU IRQ done"), "firmware did not finish");
    for m in ["CPACR ok", "S0 11111111 ok", "S3 44444444 ok", "FPSCR 00000000 ok", "FPU IRQ all PASS"] {
        assert!(uart.contains(m), "missing marker {m}");
    }
    assert!(!uart.contains("FAIL"), "firmware reported FAIL");
    no_fault(&cpu, &mem);
    let n: u32 = uart.split("IRQ count ").nth(1).unwrap_or("0").split(|c: char| !c.is_numeric()).next().unwrap_or("0").parse().unwrap_or(0);
    eprintln!("IRQ count: {n}");
    assert!(n >= 10, "SysTick never fired - vacuous run");
}

#[test]
fn fpu_directed_div_sqrt_exact() {
    // Directed div/sqrt go through exact integer cores (long division +
    // digit-recurrence sqrt) — no f64 anywhere. Hand-derived vectors where
    // the directed answer provably differs from RNE.
    let f = |x: f32| x.to_bits();
    // 1/3: RNE = 0x3EAAAAAB (above truth); toward -inf steps down one ulp.
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, 2 << 22)], &[(1, f(1.0)), (2, f(3.0))], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x3EAA_AAAA, 0x10));
    // 2/3: RNE = 0x3F2AAAAB (above truth); -inf and toward-zero agree here.
    for (rm, want) in [(1u32, 0x3F2A_AAAB), (2, 0x3F2A_AAAA), (3, 0x3F2A_AAAA)] {
        let (cpu, _) = run_fpu_snippet(
            &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, rm << 22)], &[(1, f(2.0)), (2, f(3.0))], 0, 2);
        assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (want, 0x10), "2/3 rmode {rm}");
    }
    // Overflow quotient: max/0.5 overflows; -inf clamps to max finite.
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, 1 << 22)], &[(1, f(f32::MAX)), (2, f(0.5))], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7F80_0000, 0x04 | 0x10));
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, 2 << 22)], &[(1, f(f32::MAX)), (2, f(0.5))], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x7F7F_FFFF, 0x04 | 0x10));
    // Subnormal quotient ties: (2^-126 + 2^-149)/2 = 2^-127 + half-ulp:
    // RNE-even -> 0x00400000, +inf -> 0x00400001, -inf/zero -> 0x00400000.
    for (rm, want) in [(0u32, 0x0040_0000), (1, 0x0040_0001), (2, 0x0040_0000), (3, 0x0040_0000)] {
        let (cpu, _) = run_fpu_snippet(
            &[0xEEE1, 0x0A10, 0xEE80, 0x0A81], &[(0, rm << 22)], &[(1, 0x0080_0001), (2, f(2.0))], 0, 2);
        assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (want, 0x08 | 0x10), "tiny div rmode {rm}");
    }
    // sqrt(2): RNE = 0x3FB504F3 (below truth); +inf steps up one ulp.
    let (cpu, _) = run_fpu_snippet(
        &[0xEEE1, 0x0A10, 0xEEB1, 0x0AE0], &[(0, 1 << 22)], &[(1, f(2.0))], 0, 2);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x3FB5_04F4, 0x10));
    // Perfect squares are exact in every mode (no IXC).
    for rm in [0u32, 1, 2, 3] {
        let (cpu, _) = run_fpu_snippet(
            &[0xEEE1, 0x0A10, 0xEEB1, 0x0AE0], &[(0, rm << 22)], &[(1, f(6.25))], 0, 2);
        assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (f(2.5), 0), "sqrt exact rmode {rm}");
    }
    // sqrt(+-0) = +-0, exact, no flags (must not reach the integer core).
    let (cpu, _) = run_fpu_snippet(&[0xEEB1, 0x0AE0], &[], &[(1, 0x8000_0000)], 0, 1);
    assert_eq!((cpu.regs.s[0], cpu.regs.fpscr & 0x1F), (0x8000_0000, 0));
}

#[test]
fn fpu_vmrs_id_regs() {
    // GAS (fpu16.s): vmrs r0,mvfr0=EEF7 0A10; vmrs r4,mvfr1=EEF6 4A10.
    // Same M4F constants as the MMIO block (peripherals/fpu.rs).
    let (cpu, _) = run_fpu_snippet(&[0xEEF7, 0x0A10], &[], &[], 0, 1);
    assert_eq!(cpu.regs.r[0], 0x1011_0021, "MVFR0");
    let (cpu, _) = run_fpu_snippet(&[0xEEF6, 0x4A10], &[], &[], 0, 1);
    assert_eq!(cpu.regs.r[4], 0x1100_0011, "MVFR1");
    // MVFR with Rt=13/15 faults (no APSR form for ID regs).
    for code in [[0xEEF7, 0xFA10u16], [0xEEF6, 0xDA10u16]] {
        let _g = lock_boot();
        let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
        mem.write32(0xE000ED88, 0x00F0_0000);
        for (i, w) in code.iter().enumerate() {
            mem.write16(0x20002000 + (i as u32) * 2, *w);
        }
        cpu.regs.r[15] = 0x20002001;
        let sys = crate::sys();
        cpu.run(sys, &mut mem, 1);
        assert!(cpu.fault.is_some(), "must fault: {:04x} {:04x}", code[0], code[1]);
    }
}

#[test]
fn fpu_mvfr2_and_fpexc() {
    // MVFR2 (EEF5, probed via neon-fp-armv8 — GAS rejects the mnemonic on
    // fpv4-sp, but the encoding is architectural).
    let (cpu, _) = run_fpu_snippet(&[0xEEF5, 0x5A10], &[], &[], 0, 1);
    assert_eq!(cpu.regs.r[5], 0x0000_0040, "MVFR2");
    // FPEXC read: EX=0 (no lazy frame), EN=1 (CPACR full, reset-set shadow).
    let (cpu, _) = run_fpu_snippet(&[0xEEF8, 0x0A10], &[], &[], 0, 1);
    assert_eq!(cpu.regs.r[0], 0x4000_0000, "EX=0 EN=1");
    // With CPACR cleared the gate still faults first (FPEXC read needs the
    // FPU enabled, like every cp10/11 insn). Single lock for the whole
    // manual section (shadowed `let _g` does NOT drop the guard early —
    // re-locking deadlocks).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    mem.write16(0x20002000, 0xEEF8);
    mem.write16(0x20002002, 0x0A10);
    cpu.regs.r[15] = 0x20002001;
    let sys = crate::sys();
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_some(), "FPEXC without CPACR must fault");
    // Clearing EN via VMSR bricks FPU access (silicon behavior): the next
    // VFP insn faults, including a re-enabling VMSR. (Same lock scope —
    // the boot() below installs a fresh system, no re-lock needed.)
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    // vmsr fpexc, r0 (EEE8 0A10) with r0 bit30 clear.
    mem.write16(0x20002000, 0xEEE8);
    mem.write16(0x20002002, 0x0A10);
    cpu.regs.r[0] = 0;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_none(), "VMSR FPEXC itself runs (EN was set)");
    // Now every VFP insn faults, and the EX/EN read shows EN=0.
    mem.write16(0x20002000, 0xEEB7);
    mem.write16(0x20002002, 0x0A00);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    assert!(cpu.fault.is_some(), "FPU use with EN=0 must fault");
}

#[test]
fn fpu_fpexc_ex_tracks_lazy() {
    // EX (bit 31) is live LSPACT: set while a lazy frame is outstanding,
    // clear otherwise. NOTE the subtlety this pins: reading FPEXC via
    // VMRS is itself an FPU use, so it completes the pending stacking
    // FIRST (like silicon) and then observes EX=0. To see EX=1, read the
    // model FPCCR directly (no FPU insn involved).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write32(0xE000ED88, 0x00F0_0000);
    cpu.regs.s[0] = 0xAAAAAAAA;
    cpu.regs.control |= 4;
    cpu.take_exception(sys, &mut mem, -2);
    assert_ne!(mem.read32(0xE000EF34) & 1, 0, "EX outstanding (MMIO view)");
    // VMRS FPEXC in handler mode: stacks first, so EX reads clear — but
    // the seeded S0 must have landed in the frame, and EN reads set.
    mem.write16(0x20002000, 0xEEF8);
    mem.write16(0x20002002, 0x0A10);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.r[0], 0x4000_0000, "EX consumed by the read, EN set");
    let fpcar = mem.read32(0xE000EF38);
    assert_eq!(mem.read32(fpcar), 0xAAAAAAAA, "read stacked thread S0 first");
    assert_eq!(mem.read32(0xE000EF34) & 1, 0, "LSPACT consumed");
}

#[test]
fn dsp_nonzero_regs() {
    // Every FB/FA family with nonzero Rd/Ra (GAS regmatrix.s): the old
    // o2-mask gates faulted all of these (only Rd=0 probes existed).
    // smmla r4,r1,r2,r3=FB51 3402: (2^30)^2>>32 + 1.
    let (cpu, _) = run_snippet(&[0xFB51, 0x3402], &[(1, 0x4000_0000), (2, 0x4000_0000), (3, 1)]);
    assert_eq!(cpu.regs.r[4], 0x1000_0001);
    // smmls r5,r1,r2,r3=FB61 3502: 1 - 2^28 (wrapping).
    let (cpu, _) = run_snippet(&[0xFB61, 0x3502], &[(1, 0x4000_0000), (2, 0x4000_0000), (3, 1)]);
    assert_eq!(cpu.regs.r[5], 0xF000_0001);
    // smmul r6,r1,r2=FB51 F602 (no accumulate).
    let (cpu, _) = run_snippet(&[0xFB51, 0xF602], &[(1, 0x4000_0000), (2, 0x4000_0000)]);
    assert_eq!(cpu.regs.r[6], 0x1000_0000);
    // usada8 r7,r1,r2,r3=FB71 3702: 4x|1-2| + 16 = 20.
    let (cpu, _) = run_snippet(&[0xFB71, 0x3702], &[(1, 0x0101_0101), (2, 0x0202_0202), (3, 16)]);
    assert_eq!(cpu.regs.r[7], 20);
    // usad8 r8,r1,r2=FB71 F802 (no accumulate).
    let (cpu, _) = run_snippet(&[0xFB71, 0xF802], &[(1, 0x0101_0101), (2, 0x0202_0202)]);
    assert_eq!(cpu.regs.r[8], 4);
    // smlad r9,r1,r2,r3=FB21 3902: 1*3 + 2*4 + 16 = 27.
    let (cpu, _) = run_snippet(&[0xFB21, 0x3902], &[(1, 0x0001_0002), (2, 0x0003_0004), (3, 16)]);
    assert_eq!(cpu.regs.r[9], 27);
    // smulwb/smulwt r10,r1,r2 (B/T half select via o2[4]).
    let (cpu, _) = run_snippet(&[0xFB31, 0xFA02], &[(1, 0x0004_0000), (2, 0x0003_FFFF)]);
    assert_eq!(cpu.regs.r[10], 0xFFFF_FFFC, "262144 x -1 >> 16");
    let (cpu, _) = run_snippet(&[0xFB31, 0xFA12], &[(1, 0x0004_0000), (2, 0x0003_0000)]);
    assert_eq!(cpu.regs.r[10], 12, "262144 x 3 >> 16");
    // smlawb/smlawt r11,r1,r2,r3 (accumulate).
    let (cpu, _) = run_snippet(&[0xFB31, 0x3B02], &[(1, 0x0004_0000), (2, 0x0003_FFFF), (3, 0x100)]);
    assert_eq!(cpu.regs.r[11], 0xFC);
    let (cpu, _) = run_snippet(&[0xFB31, 0x3B12], &[(1, 0x0004_0000), (2, 0x0003_0000), (3, 0x100)]);
    assert_eq!(cpu.regs.r[11], 0x10C);
    // smlal r8,r9,r1,r2=FBC1 8902 (lo/hi accumulators, high regs).
    let (cpu, _) = run_snippet(&[0xFBC1, 0x8902], &[(8, 1), (9, 2), (1, 3), (2, 5)]);
    assert_eq!((cpu.regs.r[8], cpu.regs.r[9]), (0x10, 2));
    // FA parallel with nonzero Rd: qadd8 + shadd16.
    let (cpu, _) = run_snippet(&[0xFA85, 0xF416], &[(5, 0x7F7F_7F7F), (6, 0x0101_0101)]);
    assert_eq!(cpu.regs.r[4], 0x7F7F_7F7F, "saturate, not wrap");
    assert_ne!(cpu.regs.xpsr & 0x08000000, 0, "Q set");
    let (cpu, _) = run_snippet(&[0xFA98, 0xF729], &[(8, 0x0004_0004), (9, 0x0002_0002)]);
    assert_eq!(cpu.regs.r[7], 0x0003_0003, "halving add");
}

/// Synthetic interrupt-test image: vector table + main spin + two handlers.
/// IRQ0 -> A @0x110 (cntA++), IRQ1 -> B @0x120 (order=cntA snapshot, cntB++).
/// NMI/SVC/SysTick -> A, HardFault -> B; everything else -> A. Counters at
/// 0x20001000 (A) / 0x20001004 (B), order slot at 0x20001008. With
/// `spinning`, both handlers are branch-to-self loops (for nesting tests,
/// where a returning handler would close the preemption window).
fn irq_test_image(spinning: bool) -> Vec<u8> {
    let mut img = vec![0u8; 0x200];
    fn w32(img: &mut Vec<u8>, off: usize, v: u32) {
        img[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }
    fn w16(img: &mut Vec<u8>, off: usize, v: u16) {
        img[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    w32(&mut img, 0x00, 0x20002000); // SP
    w32(&mut img, 0x04, 0x08000101); // reset -> main
    for v in 2..16u32 {
        w32(&mut img, (v * 4) as usize, if v == 3 { 0x08000121 } else { 0x08000111 });
    }
    w32(&mut img, 0x40, 0x08000111); // IRQ0 -> A
    w32(&mut img, 0x44, 0x08000121); // IRQ1 -> B
    w16(&mut img, 0x100, 0xE7FE); // main: b .
    if spinning {
        w16(&mut img, 0x110, 0xE7FE);
        w16(&mut img, 0x120, 0xE7FE);
    } else {
        // A: ldr r0,[pc,#8](0x11C); ldr r1,[r0]; adds r1,#1; str r1,[r0]; bx lr
        for (o, v) in [(0x110, 0x4802u16), (0x112, 0x6801), (0x114, 0x3101), (0x116, 0x6001), (0x118, 0x4770), (0x11A, 0xBF00)] {
            w16(&mut img, o, v);
        }
        w32(&mut img, 0x11C, 0x20001000);
        // B: order=cntA; cntB++
        for (o, v) in [(0x120, 0x4804u16), (0x122, 0x6801), (0x124, 0x4A04), (0x126, 0x6011), (0x128, 0x4804), (0x12A, 0x6801), (0x12C, 0x3101), (0x12E, 0x6001), (0x130, 0x4770), (0x132, 0xBF00)] {
            w16(&mut img, o, v);
        }
        w32(&mut img, 0x134, 0x20001000);
        w32(&mut img, 0x138, 0x20001008);
        w32(&mut img, 0x13C, 0x20001004);
    }
    img
}

#[test]
fn irq_priority_order() {
    // B (prio 0x40) must run before A (prio 0xC0) when both pend: the old
    // vector-order selection ran A first (order slot would read 1).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x3); // ISER0: IRQ0+IRQ1
    mem.write32(0xE000E400, 0x000040C0); // IPR0: A=0xC0 (low), B=0x40 (high)
    sys.p.nvic.borrow_mut().set_intr_pending(0);
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.run(sys, &mut mem, 300);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0x20001008), 0, "B must run before A (order slot)");
    assert_eq!(mem.read32(0x20001000), 1, "A ran");
    assert_eq!(mem.read32(0x20001004), 1, "B ran");
    assert_eq!(cpu.ipsr, 0, "back in thread mode");
}

#[test]
fn basepri_masks_and_unmasks() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x2); // ISER0: IRQ1 only
    mem.write32(0xE000E400, 0x00004000); // IPR0: B=0x40
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.regs.basepri = 0x40; // masks prio >= 0x40, i.e. B
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0, "masked IRQ must not activate");
    assert_eq!(mem.read32(0x20001004), 0, "B handler must not run");
    assert!(sys.p.nvic.borrow().irq_pending(1), "masked IRQ stays pending");
    cpu.regs.basepri = 0; // unmask
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0x20001004), 1, "B runs after unmask");
    assert!(!sys.p.nvic.borrow().irq_pending(1), "pending cleared by take");
}

#[test]
fn faultmask_blocks_all_but_nmi() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x2);
    mem.write32(0xE000E400, 0x00004000);
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    sys.p.nvic.borrow_mut().set_intr_pending(-14); // NMI
    cpu.regs.faultmask = true;
    cpu.run(sys, &mut mem, 200);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0x20001000), 1, "NMI (vector->A) runs through FAULTMASK");
    assert_eq!(mem.read32(0x20001004), 0, "B stays masked");
    assert!(sys.p.nvic.borrow().irq_pending(1), "B still pending");
    cpu.regs.faultmask = false;
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0x20001004), 1, "B runs after clear");
}

#[test]
fn nesting_preempts_upward() {
    // Spinning A (prio 0xC0) is preempted by B (prio 0x00): ipsr moves 16->17.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(true));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x3);
    mem.write32(0xE000E400, 0x000000C0); // A=0xC0, B=0x00
    sys.p.nvic.borrow_mut().set_intr_pending(0);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 16, "in A handler");
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 17, "B preempted A");
}

#[test]
fn nesting_blocks_downward() {
    // Spinning B (prio 0x00) is NOT preempted by A (prio 0xC0).
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(true));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x3);
    mem.write32(0xE000E400, 0x000000C0);
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 17, "in B handler");
    sys.p.nvic.borrow_mut().set_intr_pending(0);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 17, "A must not preempt B");
    assert!(sys.p.nvic.borrow().irq_pending(0), "A stays pending");
}

#[test]
fn tail_chain_reuses_frame() {
    // Direct-drive: take A, pend B, return -> must chain into B (ipsr 17,
    // handler PC, one live entry) instead of unstacking to thread.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x3);
    mem.write32(0xE000E400, 0x000040C0);
    cpu.take_exception(sys, &mut mem, 0);
    assert_eq!(cpu.ipsr, 16);
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    let lr_a = cpu.regs.r[14];
    assert!(cpu.exception_return(sys, &mut mem, lr_a, cpu.regs.r[15] & !1));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 17, "chained into B, not returned to thread");
    assert_eq!(cpu.regs.r[15] & !1, 0x08000120, "at B handler");
    assert_eq!(cpu.exc_stack.len(), 1, "one live entry (pop+push)");
    // B's LR returns to the thread the reused frame came from (F9: the
    // stack held only A, so the frame owner is thread-on-MSP).
    assert_eq!(cpu.regs.r[14], super::EXC_RETURN_MSP, "chained LR targets thread MSP frame");
    let lr_b = cpu.regs.r[14];
    assert!(cpu.exception_return(sys, &mut mem, lr_b, cpu.regs.r[15] & !1));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0, "back in thread mode");
    assert!(cpu.exc_stack.is_empty(), "stack drained");
}

#[test]
fn sleeponexit_naps_on_thread_return() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000E100, 0x1);
    mem.write32(0xE000ED10, 0x2); // SCR.SLEEPONEXIT
    cpu.take_exception(sys, &mut mem, 0);
    let lr = cpu.regs.r[14];
    assert!(!cpu.sleeping, "not asleep mid-handler");
    assert!(cpu.exception_return(sys, &mut mem, lr, cpu.regs.r[15] & !1));
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0);
    assert!(cpu.sleeping, "SLEEPONEXIT naps the core on return to thread");
}

#[test]
fn dwt_cyccnt_counts_instructions() {
    // 100 spinning instructions must read back as exactly 100 counts; with
    // TRCENA clear the counter stays frozen.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write16(0x20002000, 0xE7FE); // b . spin in SRAM
    cpu.regs.r[15] = 0x20002001;
    mem.write32(0xE000EDFC, 1 << 24); // DEMCR.TRCENA
    mem.write32(0xE0001000, 1); // DWT_CTRL.CYCCNTENA
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0xE0001004), 100, "CYCCNT tracks instructions 1:1");
    mem.write32(0xE000EDFC, 0); // TRCENA clear freezes
    cpu.run(sys, &mut mem, 50);
    assert_eq!(mem.read32(0xE0001004), 100, "frozen without TRCENA");
}

#[test]
fn msr_basepri_max_semantics() {
    // MSR BASEPRI,R0 (F380 8812) sets the mask; BASEPRI_MAX (F380 8813)
    // only ever raises it; MRS (F3E0 8112) reads it back.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    mem.write16(0x20002000, 0xF380);
    mem.write16(0x20002002, 0x8812); // msr basepri, r0
    mem.write16(0x20002004, 0xF380);
    mem.write16(0x20002006, 0x8813); // msr basepri_max, r0
    mem.write16(0x20002008, 0xF3E0);
    mem.write16(0x2000200A, 0x8112); // mrs r1, basepri
    mem.write16(0x2000200C, 0xE7FE);
    cpu.regs.r[0] = 0x40;
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.basepri, 0x40, "MSR BASEPRI writes through");
    cpu.regs.r[0] = 0x20;
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.basepri, 0x40, "BASEPRI_MAX never lowers");
    cpu.regs.r[0] = 0x80;
    cpu.regs.r[15] = 0x20002004; // re-run the MAX insn
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.basepri, 0x80, "BASEPRI_MAX raises");
    cpu.regs.r[15] = 0x20002008; // MRS insn
    cpu.run(sys, &mut mem, 1);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.regs.r[1], 0x80, "MRS BASEPRI reads back");
}

#[test]
fn svc_taken_and_escalated() {
    // Unmasked SVC takes the SVC vector; FAULTMASK-locked SVC halts
    // (lockup: even HardFault is blocked), it does not ghost-take.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write16(0x20002000, 0xDF00); // svc #0
    mem.write16(0x20002002, 0xE7FE); // b .
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 20);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0, "SVC handler returned");
    assert_eq!(mem.read32(0x20001000), 1, "SVC vector (A) ran");

    let (mut cpu2, mut mem2) = boot(&irq_test_image(false));
    let sys2 = crate::sys();
    cpu2.deliver_irqs = true;
    cpu2.regs.faultmask = true;
    mem2.write16(0x20002000, 0xDF00);
    mem2.write16(0x20002002, 0xE7FE);
    cpu2.regs.r[15] = 0x20002001;
    cpu2.run(sys2, &mut mem2, 20);
    assert!(cpu2.fault.is_some(), "FAULTMASK-locked SVC must lock up loudly");
}

#[test]
fn usagefault_nocp_escalates_past_basepri() {
    // NOCP with USGFAULTENA + low UsageFault priority under BASEPRI must
    // escalate to a TAKEN HardFault (vector B), with the NOCP sticky set.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000ED24, 1 << 18); // SHCSR.USGFAULTENA
    mem.write8(0xE000ED1A, 0x80); // SHPR1 UsageFault byte = 0x80
    cpu.regs.basepri = 0x40; // masks 0x80, not HardFault (-1)
    mem.write16(0x20002000, 0xEEF1); // vsqrt s17,s18 (needs CPACR=FPU)
    mem.write16(0x20002002, 0x8AC9);
    mem.write16(0x20002004, 0xE7FE);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 30);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0, "HardFault handler returned");
    assert_eq!(mem.read32(0x20001004), 1, "HardFault vector (B) ran");
    assert_ne!(mem.read32(0xE000ED28) & 0x00080000, 0, "NOCP sticky latched");
}

#[test]
fn aircr_vectkey_gate() {
    // AIRCR writes need the VECTKEY in the HIGH halfword; the old gate
    // checked the low half, so PRIGROUP/SYSRESETREQ never applied.
    let _g = lock_boot();
    let (_, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    assert_eq!((mem.read32(0xE000ED0C) >> 8) & 7, 0, "reset PRIGROUP=0");
    mem.write32(0xE000ED0C, 0x05FA0300); // valid key, PRIGROUP=3
    assert_eq!((mem.read32(0xE000ED0C) >> 8) & 7, 3, "PRIGROUP write applies");
    mem.write32(0xE000ED0C, 0x00000300); // bad key: ignored
    assert_eq!((mem.read32(0xE000ED0C) >> 8) & 7, 3, "bad key ignored");
    mem.write32(0xE000ED0C, 0x05FA0000); // back to 0
    assert_eq!((mem.read32(0xE000ED0C) >> 8) & 7, 0);
    let _ = sys;
}

#[test]
fn prigroup_subpriority_ignores_preemption() {
    // PRIGROUP=0: A=0x41 and B=0x40 share group 0x20 — subpriority never
    // preempts, so B must NOT preempt running A (full-value compare would).
    // B=0x3F (group 0x1F) does preempt.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(true));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000ED0C, 0x05FA0000); // PRIGROUP=0 explicitly
    mem.write32(0xE000E100, 0x3);
    mem.write32(0xE000E400, 0x00004041); // A=0x41, B=0x40
    sys.p.nvic.borrow_mut().set_intr_pending(0);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 16, "in A handler");
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 16, "same-group subpriority must not preempt");
    assert!(sys.p.nvic.borrow().irq_pending(1), "B stays pending");
    mem.write8(0xE000E401, 0x3F); // B=0x3F: group 0x1F < 0x20
    cpu.run(sys, &mut mem, 50);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 17, "lower group preempts");
}

#[test]
fn basepri_masks_by_group() {
    // BASEPRI compares in group space: with PRIGROUP=0, BASEPRI=0x41
    // (group 0x20) masks B=0x40 (group 0x20) — full-value compare would
    // let it through (0x40 < 0x41). BASEPRI=0x60 (group 0x30) releases it.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write32(0xE000ED0C, 0x05FA0000);
    mem.write32(0xE000E100, 0x2);
    mem.write32(0xE000E400, 0x00004000);
    sys.p.nvic.borrow_mut().set_intr_pending(1);
    cpu.regs.basepri = 0x41;
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(cpu.ipsr, 0, "group-masked IRQ must not activate");
    assert_eq!(mem.read32(0x20001004), 0);
    assert!(sys.p.nvic.borrow().irq_pending(1), "stays pending");
    cpu.regs.basepri = 0x60;
    cpu.run(sys, &mut mem, 100);
    no_fault(&cpu, &mem);
    assert_eq!(mem.read32(0x20001004), 1, "released by higher group");
}

#[test]
fn sev_wfe_event_register() {
    // SEV then WFE: event registered -> clear-and-continue, no sleep.
    // Bare WFE: sleeps. Encodings: SEV=BF40, WFE=BF20, b .=E7FE.
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    mem.write16(0x20002000, 0xBF40);
    mem.write16(0x20002002, 0xBF20);
    mem.write16(0x20002004, 0xE7FE);
    cpu.regs.r[15] = 0x20002001;
    cpu.run(sys, &mut mem, 3);
    no_fault(&cpu, &mem);
    assert!(!cpu.sleeping, "SEV-armed WFE must not sleep");
    assert_eq!(cpu.regs.r[15] & !1, 0x20002004, "past the WFE");

    let (mut cpu2, mut mem2) = boot(include_bytes!("../../../blinky/blinky.bin"));
    let sys2 = crate::sys();
    cpu2.deliver_irqs = true;
    mem2.write16(0x20002000, 0xBF20);
    mem2.write16(0x20002002, 0xE7FE);
    cpu2.regs.r[15] = 0x20002001;
    cpu2.run(sys2, &mut mem2, 2);
    no_fault(&cpu2, &mem2);
    assert!(cpu2.sleeping, "bare WFE sleeps with no event");
}

#[test]
fn exception_entry_return_sets_event() {
    let _g = lock_boot();
    let (mut cpu, mut mem) = boot(&irq_test_image(false));
    let sys = crate::sys();
    cpu.deliver_irqs = true;
    assert!(!cpu.event_register, "reset clears the event register");
    cpu.take_exception(sys, &mut mem, 0);
    assert!(cpu.event_register, "entry sets the event");
    cpu.event_register = false;
    let lr = cpu.regs.r[14];
    assert!(cpu.exception_return(sys, &mut mem, lr, cpu.regs.r[15] & !1));
    assert!(cpu.event_register, "return sets the event");
}
