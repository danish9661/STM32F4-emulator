// Headless-Chrome CDP smoke test for the WebSocket bridge.
// Starts ws-bridge.mjs with blinky firmware, serves the site, opens
// Chrome with ?fw=blinky&bridge=ws://…, and asserts UART markers appear.
// Run: node site/test_bridge_cdp.mjs   (needs google-chrome + python3)
import { spawn } from 'child_process';
import { mkdtempSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join, dirname } from 'path';
import { fileURLToPath } from 'url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const ROOT = join(__dirname, '..');
const CHROME = process.env.CHROME_BIN || '/usr/bin/google-chrome';
const BRIDGE_PORT = 8236;
const HTTP_PORT = 8138;

async function waitFor(pred, timeoutMs, label) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
        try { if (await pred()) return true; } catch { /* retry */ }
        await new Promise((r) => setTimeout(r, 300));
    }
    return false;
}

function spawnOrDie(cmd, args, opts) {
    const p = spawn(cmd, args, opts);
    p.on('error', (e) => { console.error(`failed to start ${cmd}: ${e.message}`); process.exit(1); });
    return p;
}

// ── Start the bridge server (no firmware on CLI — browser sends LOAD_IMAGE) ──
const bridge = spawnOrDie(process.execPath, [
    join(__dirname, 'ws-bridge.mjs'),
    '--port', String(BRIDGE_PORT),
], { stdio: ['ignore', 'pipe', 'pipe'] });

let bridgeOutput = '';
bridge.stdout.on('data', (d) => { bridgeOutput += d.toString(); });
bridge.stderr.on('data', (d) => { bridgeOutput += d.toString(); });

// Wait for bridge to be ready — the ws library responds to plain HTTP with
// 426 (Upgrade Required), so we can't use fetch to detect readiness; instead
// wait for the "listening" line in stdout/stderr.
const bridgeReady = await waitFor(() => bridgeOutput.includes('listening'), 10000, 'bridge');
if (!bridgeReady) { console.error('bridge did not start:\n' + bridgeOutput); bridge.kill('SIGKILL'); process.exit(1); }

// ── Start HTTP server ──
const http = spawnOrDie('python3', ['-m', 'http.server', String(HTTP_PORT), '--directory', join(ROOT, 'site')], { stdio: 'ignore' });

let ok = false, reason = '', pageErrors = [];
const userData = mkdtempSync(join(tmpdir(), 'cdp-bridge-'));
let chrome, ws;

try {
    // Wait for HTTP server
    if (!await waitFor(async () => {
        try { return (await fetch(`http://127.0.0.1:${HTTP_PORT}/index.html`)).ok; } catch { return false; }
    }, 10000, 'http server')) throw new Error('http server did not start');

    // Start Chrome
    chrome = spawnOrDie(CHROME, [
        '--headless=new', '--remote-debugging-port=0',
        '--user-data-dir=' + userData, '--disable-gpu', '--no-sandbox',
        '--disable-dev-shm-usage', 'about:blank',
    ], { stdio: ['ignore', 'pipe', 'pipe'] });

    // Extract CDP port from Chrome's stderr
    let cdpPort = 0;
    const cdpPromise = new Promise((res, rej) => {
        const timeout = setTimeout(() => rej(new Error('chrome did not print CDP port')), 15000);
        chrome.stderr.on('data', (d) => {
            const s = d.toString();
            const m = s.match(/DevTools listening on ws:\/\/127\.0\.0\.1:(\d+)\//);
            if (m) { cdpPort = parseInt(m[1], 10); clearTimeout(timeout); res(); }
        });
    });
    await cdpPromise;

    // Connect to page-level debugger
    const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json();
    const page = targets.find((t) => t.type === 'page');
    if (!page) throw new Error('no page target in CDP');

    ws = new WebSocket(page.webSocketDebuggerUrl);
    await new Promise((res, rej) => { ws.addEventListener('open', res); ws.addEventListener('error', rej); });

    const pending = new Map();
    let nextId = 1;
    const send = (method, params = {}) => new Promise((res) => {
        const id = nextId++;
        pending.set(id, res);
        ws.send(JSON.stringify({ id, method, params }));
    });
    ws.addEventListener('message', (d) => {
        const m = JSON.parse(typeof d.data === 'string' ? d.data : d.data.toString());
        if (m.id && pending.has(m.id)) { pending.get(m.id)(m.result); pending.delete(m.id); }
        else if (m.method === 'Runtime.exceptionThrown') {
            const e = m.params.exceptionDetails;
            pageErrors.push((e.exception && (e.exception.description || e.exception.value)) || e.text);
        }
    });

    await send('Page.enable');
    await send('Runtime.enable');

    // Navigate with bridge param — firmware is already loaded by the bridge,
    // so the page connects and starts receiving UART immediately.
    const url = `http://127.0.0.1:${HTTP_PORT}/?fw=blinky&bridge=ws://127.0.0.1:${BRIDGE_PORT}`;
    await send('Page.navigate', { url });

    // Wait for boot() to complete (emu is set in bridge mode after LOAD_IMAGE)
    const bootReady = await waitFor(async () => {
        const r = await send('Runtime.evaluate', {
            expression: "!!window.__emu",
            returnByValue: true,
        });
        return r && r.result && r.result.value === true;
    }, 30000, 'boot');
    if (!bootReady) {
        // Dump diagnostics
        const ps = await send('Runtime.evaluate', {
            expression: "JSON.stringify({emu:!!window.__emu, status:document.getElementById('status')?.textContent||''})",
            returnByValue: true,
        });
        console.error('boot timeout, page state:', JSON.stringify(ps));
    }

    // Poll for UART marker
    const timeoutMs = 60000;
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
        const r = await send('Runtime.evaluate', {
            expression: "document.getElementById('uart') ? document.getElementById('uart').textContent : ''",
            returnByValue: true,
        });
        const txt = (r && r.result && r.result.value) || '';
        if (txt.includes('FAIL')) { ok = false; reason = 'fail marker in UART'; break; }
        if (txt.includes('LED=ON')) { ok = true; reason = 'marker: LED=ON (bridge)'; break; }
        await new Promise((res) => setTimeout(res, 500));
    }
    if (!ok && !reason) reason = 'timeout waiting for LED=ON via bridge';

    // Also verify the bridge actually connected — the page should show
    // the firmware name in the status bar.
    const statusR = await send('Runtime.evaluate', {
        expression: "document.getElementById('status') ? document.getElementById('status').textContent : ''",
        returnByValue: true,
    });
    const statusText = (statusR && statusR.result && statusR.result.value) || '';
    if (ok && !statusText.includes('blinky')) {
        reason += ' (status bar: ' + statusText.trim() + ')';
    }

} catch (e) {
    ok = false; reason = e.message;
} finally {
    try { ws.close(); } catch {}
    try { chrome.kill('SIGKILL'); } catch {}
    try { http.kill('SIGKILL'); } catch {}
    try { bridge.kill('SIGKILL'); } catch {}
    try { rmSync(userData, { recursive: true, force: true }); } catch {}
}

console.log(`[${ok ? 'PASS' : 'FAIL'}] bridge CDP — ${reason}`);
if (pageErrors.length) console.log('   page errors:', pageErrors.slice(0, 5).join(' | '));
process.exitCode = ok ? 0 : 1;
