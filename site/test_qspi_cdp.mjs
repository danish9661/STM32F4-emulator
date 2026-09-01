// Headless-Chrome CDP smoke for the in-page QSPI demo.
// Boots ?fw=qspi_test in the browser and asserts "QSPI OK" appears on the UART.
// Requires: google-chrome (or CHROME_BIN) and python3.
import { runCdpSmoke } from './cdp_smoke.mjs';

const r = await runCdpSmoke({ fw: 'qspi_test', markers: ['QSPI OK'], failMarkers: ['QSPI FAIL'], timeoutMs: 60000 });
if (r.ok) {
    console.log('QSPI CDP PASS: "QSPI OK" seen in browser UART');
    process.exitCode = 0;
} else if (r.reason.includes('skipped')) {
    console.log('QSPI CDP SKIP:', r.reason);
    process.exitCode = 0;
} else {
    console.log('QSPI CDP FAIL:', r.reason, r.pageErrors.slice(0, 3).join(' | '));
    process.exitCode = 1;
}
