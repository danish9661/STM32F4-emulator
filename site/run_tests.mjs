// Run each test individually so failures don't break the chain.
// Usage: node site/run_tests.mjs [--filter substring]
import { spawn } from 'child_process';
import { writeSync } from 'fs';

const log = (msg) => { writeSync(1, msg + '\n'); };

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

function runTest(testFile) {
    return new Promise((resolve) => {
        const child = spawn('node', [testFile], { stdio: ['ignore', 'pipe', 'pipe'] });
        let stdout = '', stderr = '';
        child.stdout.on('data', (d) => { stdout += d; process.stdout.write(d); });
        child.stderr.on('data', (d) => { stderr += d; process.stderr.write(d); });
        const timer = setTimeout(() => { child.kill('SIGKILL'); }, 180_000);
        child.on('close', (code) => {
            clearTimeout(timer);
            resolve({ code, stdout, stderr });
        });
        child.on('error', (e) => {
            clearTimeout(timer);
            resolve({ code: -1, stdout, stderr: e.message });
        });
    });
}

for (const t of tests) {
    log(`\n========== ${t} ==========`);
    const { code, stdout, stderr } = await runTest(t);
    if (code === 0) {
        log(`>>> PASS: ${t}`);
        passed++;
    } else {
        log(`>>> FAIL: ${t} (exit ${code})`);
        const tail = (stdout + stderr).split('\n').filter(Boolean).slice(-10).join('\n');
        if (tail) log(`  Last lines:\n${tail}`);
        failed++;
    }
}

log(`\n========================================`);
log(`Results: ${passed} passed, ${failed} failed out of ${tests.length} tests`);
process.exitCode = failed ? 1 : 0;
