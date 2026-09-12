// Combined headless-Chrome CDP smoke for the in-page demo.
// Boots each preset in a real browser and asserts the expected UART marker,
// giving true in-browser regression coverage (not just the node path).
// Run: node site/test_browser.mjs   (needs google-chrome + python3)
import { runCdpSmoke } from './cdp_smoke.mjs';

const CASES = [
    { label: 'blinky',  fw: 'blinky',    markers: ['LED=ON'],                         timeoutMs: 60000 },
    { label: 'eth_http flow', fw: 'eth_http', markers: ['TCP connected'],            timeoutMs: 120000 },
    { label: 'oled',    fw: 'oled_test', markers: ['OLED draw done'],                timeoutMs: 60000 },
    { label: 'tft',     fw: 'tft_test',  markers: ['TFT fill done'],                 timeoutMs: 60000 },
    { label: 'ltdc',    fw: 'ltdc_test', markers: ['LTDC pixels OK'],                timeoutMs: 60000 },
    // Peripheral-function smokes: the marker proves the peripheral itself ran,
    // not just that the firmware booted.
    { label: 'can_test',        fw: 'can_test',        markers: ['CAN loopback OK'],              failMarkers: ['CAN Test: FAIL'], timeoutMs: 60000 },
    { label: 'watchdog_demo',   fw: 'watchdog_demo',   markers: ['IWDG reset detected'],           timeoutMs: 60000 },
    { label: 'rtc_test',        fw: 'rtc_test',        markers: ['RTC verify OK'],                failMarkers: ['RTC verify FAIL'], timeoutMs: 60000 },
    { label: 'audio_play_test', fw: 'audio_play_test', markers: ['I2S1 TX sine 256 samples'],     timeoutMs: 60000 },
    { label: 'deep_sleep_demo', fw: 'deep_sleep_demo', markers: ['WOKE FROM STOP'],               timeoutMs: 60000 },
    { label: 'fpu_test',        fw: 'fpu_test',        markers: ['FPU all PASS'],                 failMarkers: ['FAIL'], timeoutMs: 60000 },
    { label: 'fpu_irq_test',    fw: 'fpu_irq_test',    markers: ['FPU IRQ all PASS'],             failMarkers: ['FAIL'], timeoutMs: 90000 },
    { label: 'mpu_test',        fw: 'mpu_test',        markers: ['MPU all PASS', 'MPU done'],   failMarkers: ['FAIL'], timeoutMs: 60000 },
    { label: 'usb_cdc_test',    fw: 'usb_cdc_test',    markers: ['USB echo OK', 'USB done'],     failMarkers: ['USB FAIL'], timeoutMs: 120000 },
    { label: 'blinky_f401',     fw: 'blinky_f401',     markers: ['Blinky F401 done'],             timeoutMs: 60000 },
    { label: 'blinky_f411',     fw: 'blinky_f411',     markers: ['Blinky F411 done'],             timeoutMs: 60000 },
    { label: 'blinky_f407g',    fw: 'blinky_f407g',    markers: ['Blinky F407G done'],            timeoutMs: 60000 },
    { label: 'blinky_f429',     fw: 'blinky_f429',     markers: ['Blinky F429 done'],             timeoutMs: 60000 },
    { label: 'blinky_nucleo_f401', fw: 'blinky_nucleo_f401', markers: ['Blinky Nucleo-F401 done'], timeoutMs: 60000 },
    { label: 'blinky_nucleo_f411', fw: 'blinky_nucleo_f411', markers: ['Blinky Nucleo-F411 done'], timeoutMs: 60000 },
    // Board-filter + new-preset coverage: Arduino on real variant maps.
    { label: 'arduino_f407vg', fw: 'arduino_disco_f407vg', markers: ['Arduino done'], timeoutMs: 120000 },
    { label: 'arduino_f401re', fw: 'arduino_nucleo_f401re', markers: ['Arduino done'], timeoutMs: 120000 },
    { label: 'flash_test',     fw: 'flash_test',     markers: ['FLASH TEST DONE'],              timeoutMs: 60000 },
    // NOTE: no 'FAIL' failMarker here — both firmwares print a zero-count
    // "FAIL: 00000000" summary line on success (same convention as
    // comprehensive_test); DONE is the pass signal, hangs time out.
    { label: 'spi_flash_test', fw: 'spi_flash_test', markers: ['SPI FLASH TEST DONE'],     timeoutMs: 60000 },
    { label: 'fsmc_test',      fw: 'fsmc_test',      markers: ['=== FSMC Test: done ==='],      failMarkers: ['FAIL'], timeoutMs: 60000 },
    { label: 'dcmi_test',      fw: 'dcmi_test',      markers: ['=== DCMI Test: done ==='],      failMarkers: ['FAIL'], timeoutMs: 120000 },
    { label: 'freertos_test',  fw: 'freertos_test',  markers: ['TIM TEST PASS'],                failMarkers: ['FAIL'], timeoutMs: 120000 },
    { label: 'oled_f401', fw: 'oled_test_f401', markers: ['OLED draw done'], timeoutMs: 60000 },
    { label: 'audio_f411', fw: 'audio_test_f411', markers: ['=== Audio Test: done'], timeoutMs: 60000 },
    { label: 'freertos_f429', fw: 'freertos_test_f429', markers: ['TIM TEST PASS'], timeoutMs: 120000 },
    { label: 'usb_f429', fw: 'usb_cdc_test_f429', markers: ['USB echo OK', 'USB done'], timeoutMs: 120000 },
    { label: 'fsmc_f429', fw: 'fsmc_test_f429', markers: ['=== FSMC Test: done ==='], timeoutMs: 60000 },
    { label: 'dcmi_f429', fw: 'dcmi_test_f429', markers: ['=== DCMI Test: done ==='], timeoutMs: 120000 },
    { label: 'adc_f429', fw: 'adc_demo_f429', markers: ['=== ADC Demo: done ==='], timeoutMs: 60000 },
    { label: 'mpu_f401', fw: 'mpu_test_f401', markers: ['MPU all PASS', 'MPU done'], timeoutMs: 60000 },
    { label: 'dma2d', fw: 'dma2d_test', markers: ['=== DMA2D Test: done ==='], timeoutMs: 60000 },
    { label: 'comprehensive_f429', fw: 'comprehensive_test_f429', markers: ['=== DONE ===', 'FAIL: 00000000'], timeoutMs: 120000 },
];

let failed = 0;
let skipped = 0;
for (const c of CASES) {
    const { ok, reason, pageErrors } = await runCdpSmoke(c);
    if (reason.includes('skipped')) {
        console.log(`[SKIP] ${c.label} — ${reason}`);
        skipped++;
        continue;
    }
    console.log(`[${ok ? 'PASS' : 'FAIL'}] ${c.label} — ${reason}`);
    if (!ok) {
        failed++;
        if (pageErrors.length) console.log('   page errors:', pageErrors.slice(0, 5).join(' | '));
    }
}
if (skipped && !failed) console.log(`\n${skipped} browser tests skipped (chrome/python not available)`);
process.exitCode = failed ? 1 : 0;
