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
