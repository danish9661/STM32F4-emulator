// Reusable headless-Chrome CDP smoke harness for the in-page STM32 demo.
// Boots `?fw=<preset>` in the browser (served from ./site) and waits for any
// of `markers` to appear in the UART terminal, or fails early on `failMarkers`.
// Requires google-chrome (or CHROME_BIN) + python3. Thin wrapper over CDP:
// spins up a static server + Chrome, connects to the page-level debugger
// socket, navigates, and polls the #uart textContent.
import { spawn } from 'child_process';
import { mkdtempSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const CHROME = process.env.CHROME_BIN || '/usr/bin/google-chrome';

async function waitFor(pred, timeoutMs, label) {
    const t0 = Date.now();
    while (Date.now() - t0 < timeoutMs) {
        try { if (await pred()) return true; } catch { /* retry */ }
        await new Promise((r) => setTimeout(r, 300));
    }
    return false;
}

export async function runCdpSmoke({ fw, markers, failMarkers = [], timeoutMs = 60000, httpPort = 8137, cdpPort = 9337 }) {
    const http = spawn('python3', ['-m', 'http.server', String(httpPort), '--directory', 'site'], { stdio: 'ignore' });
    const userData = mkdtempSync(join(tmpdir(), 'cdp-smoke-'));
    const chrome = spawn(CHROME, [
        '--headless=new', '--remote-debugging-port=' + cdpPort,
        '--user-data-dir=' + userData, '--disable-gpu', '--no-sandbox',
        '--disable-dev-shm-usage', 'about:blank',
    ], { stdio: 'ignore' });

    let ok = false, reason = '', pageErrors = [];
    try {
        if (!await waitFor(async () => {
            try { return (await fetch(`http://127.0.0.1:${httpPort}/index.html`)).ok; } catch { return false; }
        }, 10000, 'http server')) throw new Error('http server did not start');
        if (!await waitFor(async () => {
            try { return (await fetch(`http://127.0.0.1:${cdpPort}/json/version`)).ok; } catch { return false; }
        }, 15000, 'chrome')) throw new Error('chrome did not start');

        const targets = await (await fetch(`http://127.0.0.1:${cdpPort}/json`)).json();
        const page = targets.find((t) => t.type === 'page');
        const ws = new WebSocket(page.webSocketDebuggerUrl);
        await new Promise((res, rej) => { ws.addEventListener('open', res); ws.addEventListener('error', rej); });

        const pending = new Map();
        let next = 1;
        const send = (method, params = {}) => new Promise((res) => {
            const id = next++;
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
        await send('Page.navigate', { url: `http://127.0.0.1:${httpPort}/?fw=${encodeURIComponent(fw)}` });

        const t0 = Date.now();
        while (Date.now() - t0 < timeoutMs) {
            const r = await send('Runtime.evaluate', {
                expression: "document.getElementById('uart') ? document.getElementById('uart').textContent : ''",
                returnByValue: true,
            });
            const txt = (r && r.result && r.result.value) || '';
            const fail = failMarkers.find((f) => txt.includes(f));
            if (fail) { ok = false; reason = 'fail marker: ' + fail; break; }
            const hit = markers.find((m) => txt.includes(m));
            if (hit) { ok = true; reason = 'marker: ' + hit; break; }
            await new Promise((res) => setTimeout(res, 500));
        }
        if (!ok && !reason) reason = 'timeout waiting for marker';
    } catch (e) {
        ok = false; reason = e.message;
    } finally {
        try { chrome.kill('SIGKILL'); } catch { /* ignore */ }
        try { http.kill('SIGKILL'); } catch { /* ignore */ }
        try { rmSync(userData, { recursive: true, force: true }); } catch { /* ignore */ }
    }
    return { ok, reason, pageErrors };
}
