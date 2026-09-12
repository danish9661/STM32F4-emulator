// Build bare-metal demos for the F401/F411/F429 board families.
// Same sources, family link script (flash/RAM LENGTH) + -DSTACK_TOP.
// Usage: node tools/build_family.mjs [demo...]   (default: all portable)
// Requires TOOLCHAIN env (xpack arm-none-eabi) like the Makefiles.
import { readFileSync, writeFileSync, existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('..', import.meta.url));
const EXPECT_SP = { f401: '0x20010000', f411: '0x20020000', f429: '0x20040000' };
const TC = process.env.TOOLCHAIN || (process.env.HOME + '/.arduino15/packages/STMicroelectronics/tools/xpack-arm-none-eabi-gcc/14.2.1-1.1/bin/arm-none-eabi-');

const FAMS = {
    f401: { flash: '256K', ram: '64K', sp: '0x20010000' },
    f411: { flash: '512K', ram: '128K', sp: '0x20020000' },
    f429: { flash: '2M', ram: '256K', sp: '0x20040000' },
};
// demo -> { fams, flags }  (flags: soft | hard | O0 | freertos)
const DEMOS = {
    adc_demo: {}, pwm_demo: {}, oled_test: {}, tft_test: {}, buzzer_test: {},
    rtc_test: {}, audio_test: {}, audio_play_test: {}, watchdog_demo: {},
    wwdg_demo: {}, wwdg_window_demo: {}, tim_capture_demo: {},
    spi_flash_test: {}, flash_test: {}, exti_test: {}, rx_interrupt_test: {}, freertos_test: { flags: 'freertos', fams: ['f411', 'f429'] }, // .testvars pinned at 0x2001E000 needs 128K+ RAM — no F401
    comprehensive_test: {}, crypto_deep_test: {}, rx_crypto_test: {},
    fpu_test: { flags: 'hard' }, fpu_irq_test: { flags: 'hard' }, mpu_test: {},
    test_firmware: {}, i2s_sai_test: { flags: 'O0' }, spi_tft_test: { flags: 'O0' },
    can_demo: { fams: ['f429'] }, can_test: { fams: ['f429'] }, can_host_rx: { fams: ['f429'] },
    dac_demo: { fams: ['f429'] }, usb_cdc_test: { fams: ['f401', 'f429'] },
    dma2d_test: { fams: ['f429'] },
    gpio_k_test: { fams: ['f429'] },
    fsmc_test: { fams: ['f429'] }, dcmi_test: { fams: ['f429'] },
    qspi_test: { fams: ['f429'] }, deep_sleep_demo: {}, ltdc_test: { fams: ['f429'] },
    eth_http: { fams: ['f429'], srcs: ['startup.c', 'eth_http.ino'] },
    eth_dhcp: { fams: ['f429'], srcs: ['startup.c', 'eth_dhcp.ino'] },
    eth_test: { fams: ['f429'] },
    eth_feat_test: { fams: ['f429'] },
    lwip_demo: { fams: ['f429'], srcs: ['startup.c', 'main.c', 'netif_f4.c', 'arch/sys_arch.c',
        'lwip/core/init.c', 'lwip/core/mem.c', 'lwip/core/memp.c', 'lwip/core/netif.c',
        'lwip/core/pbuf.c', 'lwip/core/raw.c', 'lwip/core/stats.c', 'lwip/core/sys.c',
        'lwip/core/tcp.c', 'lwip/core/tcp_in.c', 'lwip/core/tcp_out.c', 'lwip/core/udp.c',
        'lwip/core/timeouts.c', 'lwip/core/def.c', 'lwip/core/inet_chksum.c',
        'lwip/core/dns.c', 'lwip/core/ip.c', 'lwip/core/ipv4/dhcp.c',
        'lwip/core/ipv4/etharp.c', 'lwip/core/ipv4/icmp.c', 'lwip/core/ipv4/ip4.c',
        'lwip/core/ipv4/ip4_addr.c', 'lwip/core/ipv4/ip4_frag.c',
        'lwip/netif/ethernet.c'],
        incs: ['-Ilwip/include', '-fno-builtin'] },    eth_irq_test: { fams: ['f429'] },
};
const FREERTOS_SRCS = ['startup.c', 'main.c', 'string.c', 'FreeRTOS/tasks.c', 'FreeRTOS/list.c',
    'FreeRTOS/queue.c', 'FreeRTOS/portable/GCC/ARM_CM3/port.c', 'FreeRTOS/portable/MemMang/heap_4.c'];
const FREERTOS_INC = '-I. -IFreeRTOS/include -IFreeRTOS/portable/GCC/ARM_CM3';
const FREERTOS_LD = '-specs=nosys.specs -specs=nano.specs';

const BASEFLAGS = {
    soft: '-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib -I.',
    hard: '-mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -O2 -g -ffreestanding -nostdlib -fsingle-precision-constant -Wdouble-promotion -Werror -I.',
    O0: '-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O0 -g -ffreestanding -nostdlib -I.',
    freertos: `-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -O2 -g -ffreestanding -nostdlib ${FREERTOS_INC}`,
};
const BASELD = {
    soft: '-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -Wl,-gc-sections',
    hard: '-mcpu=cortex-m4 -mthumb -mfpu=fpv4-sp-d16 -mfloat-abi=hard -nostdlib -Wl,-gc-sections',
    O0: '-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -Wl,-gc-sections',
    freertos: `-mcpu=cortex-m4 -mthumb -mfloat-abi=soft -nostdlib -Wl,-gc-sections ${FREERTOS_LD}`,
};

const only = new Set(process.argv.slice(2));
let fail = 0;
for (const [demo, cfg] of Object.entries(DEMOS)) {
    if (only.size && !only.has(demo)) continue;
    const flags = cfg.flags || 'soft';
    const fams = cfg.fams || ['f401', 'f411', 'f429'];
    const dir = `${root}/${demo}`;
    const linkSrc = readFileSync(`${dir}/link.ld`, 'utf8');
    if (!linkSrc.includes('LENGTH')) { console.log(`${demo}: no LENGTH link script, SKIP`); fail++; continue; }
    const srcs = flags === 'freertos' ? FREERTOS_SRCS : (cfg.srcs || ['startup.c', 'main.c']);
    for (const s of srcs) {
        if (!existsSync(`${dir}/${s}`)) { console.log(`${demo}: missing ${s}, SKIP fam loop`); fail++; continue; }
    }
    for (const fam of fams) {
        const F = FAMS[fam];
        const link = linkSrc
            .replace(/(FLASH[^:]*:\s*ORIGIN\s*=\s*0x08000000\s*,\s*LENGTH\s*=\s*)\S+/, `$1${F.flash}`)
            .replace(/(RAM[^:]*:\s*ORIGIN\s*=\s*0x20000000\s*,\s*LENGTH\s*=\s*)\S+/, `$1${F.ram}`);
        writeFileSync(`${dir}/link_${fam}.ld`, link);
        const out = `${demo}_${fam}`;
        try {
            execFileSync(`${TC}gcc`, [...BASEFLAGS[flags].split(' '), ...(cfg.incs || []), `-DSTACK_TOP=${F.sp}`, `-DEXPECT_SP=${EXPECT_SP[fam]}`,
                '-x', 'c', ...srcs, '-x', 'none', ...BASELD[flags].split(' '), '-T', `link_${fam}.ld`, '-o', `${out}.elf`],
                { cwd: dir, stdio: 'pipe' });
            execFileSync(`${TC}objcopy`, ['-O', 'binary', `${out}.elf`, `${out}.bin`], { cwd: dir });
            console.log(`${out}: ok`);
        } catch (e) {
            console.log(`${out}: BUILD FAIL\n${(e.stderr || '').toString().slice(0, 800)}`);
            fail++;
        }
    }
}
process.exit(fail ? 1 : 0);
