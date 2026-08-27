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
];

let failed = 0;
for (const c of CASES) {
    const { ok, reason, pageErrors } = await runCdpSmoke(c);
    console.log(`[${ok ? 'PASS' : 'FAIL'}] ${c.label} — ${reason}`);
    if (!ok) {
        failed++;
        if (pageErrors.length) console.log('   page errors:', pageErrors.slice(0, 5).join(' | '));
    }
}
process.exitCode = failed ? 1 : 0;
