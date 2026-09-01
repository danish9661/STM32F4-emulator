// Run each test individually so failures don't break the chain.
// Usage: node site/run_tests.mjs [--filter substring]
import { execFileSync } from 'child_process';
import { fileURLToPath } from 'url';

const ALL_TESTS = [
    'site/test_stm32f4_api.mjs',
    'site/test_stm32f4_periph.mjs',
    'site/test_ws_bridge.mjs',
    'site/test_flow.mjs',
    'site/test_blinky.mjs',
    'site/test_rx_interrupt.mjs',
    'site/test_component_led.mjs',
    'site/test_component_button.mjs',
    'site/test_component_pwm.mjs',
    'site/test_component_i2cregfile.mjs',
    'site/test_component_adc.mjs',
    'site/test_multi_instance.mjs',
    'site/test_fsmc_dcmi.mjs',
    'site/test_fsmc.mjs',
    'site/test_dcmi.mjs',
    'site/probe_freertos.mjs',
    'site/test_candemo.mjs',
    'site/test_lowpower.mjs',
    'site/test_edge_cases.mjs',
    'site/test_can_inject.mjs',
    'site/test_watchdog.mjs',
    'site/test_wwdg.mjs',
    'site/test_wwdg_window.mjs',
    'site/test_tim_capture.mjs',
    'site/test_qspi.mjs',
];

const filter = process.argv.includes('--filter')
    ? process.argv[process.argv.indexOf('--filter') + 1]
    : null;

const tests = filter ? ALL_TESTS.filter(t => t.includes(filter)) : ALL_TESTS;
let passed = 0, failed = 0;

for (const t of tests) {
    process.stdout.write(`>>> ${t} ... `);
    try {
        execFileSync('node', [t], { stdio: 'pipe', timeout: 180_000 });
        console.log('OK');
        passed++;
    } catch (e) {
        const out = (e.stdout || '').toString().split('\n').filter(Boolean).slice(-3).join('\n');
        const err = (e.stderr || '').toString().split('\n').filter(Boolean).slice(-3).join('\n');
        console.log(`FAIL (exit ${e.status})`);
        if (out) console.log(`  stdout: ${out}`);
        if (err) console.log(`  stderr: ${err}`);
        failed++;
    }
}

console.log(`\nResults: ${passed} passed, ${failed} failed out of ${tests.length} tests`);
process.exitCode = failed;
