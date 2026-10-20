// Version-literal consistency guard for the served page.
//
// Why this exists: a stale `?v=` is invisible in development and shows up as a
// mixed old/new build in any browser that already cached the asset. The bug
// this pins (2026-10-20): `emu-worker.js` still imported
// `vendor/stm32_periph_wasm.js?v=55` after VENDOR_V moved to 56, so the
// console page's default (worker) path ran the previous wasm model against the
// current glue while app.js's main-thread path ran the new one.
//
// Checked here (reference graph, not file contents):
//   1. every asset is referenced with exactly ONE version across all files,
//   2. the vendor group (glue JS + wasm + board SVDs) moves together,
//   3. every referenced file exists (root or site/vendor),
//   4. __doomVer is present and numeric (the page's build stamp).
//
// Usage: node site/test_versions.mjs   (exit 0 = consistent)
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const SITE = dirname(fileURLToPath(import.meta.url));
// Firmware blobs are data, not reference graphs (and are megabytes).
const SKIP = new Set(['firmware.js', 'firmware-doom.js']);
const SOURCES = readdirSync(SITE).filter((f) => /\.(js|html)$/.test(f) && !SKIP.has(f));

// Assets that must share one number: the wasm model and the map it is booted
// against are one build (a glue/wasm split desyncs the ABI silently).
const VENDOR = new Set([
    'stm32_periph_wasm.js',
    'stm32_periph_wasm_bg.wasm',
    'stm32f401.svd',
    'stm32f407.svd',
    'stm32f411.svd',
    'stm32f429.svd',
]);

const refs = [];
for (const file of SOURCES) {
    const lines = readFileSync(join(SITE, file), 'utf8').split('\n');
    lines.forEach((line, i) => {
        for (const m of line.matchAll(/([A-Za-z0-9_./-]+\.(?:js|wasm|svd))\?v=(\d+)/g)) {
            refs.push({ asset: m[1].split('/').pop(), version: Number(m[2]), file, line: i + 1 });
        }
        // Concatenated SVD form: fetch('vendor/' + board.svd + '?v=56')
        for (const m of line.matchAll(/\bsvd'?\s*\+\s*'\?v=(\d+)/g)) {
            refs.push({ asset: 'stm32f407.svd', version: Number(m[1]), file, line: i + 1 });
        }
    });
}

const problems = [];
const groups = new Map();
for (const r of refs) {
    const key = VENDOR.has(r.asset) ? 'vendor' : r.asset;
    if (!groups.has(key)) groups.set(key, []);
    groups.get(key).push(r);
}

// 1 + 2: one version per asset, vendor group together.
for (const [key, list] of [...groups].sort((a, b) => a[0].localeCompare(b[0]))) {
    const versions = [...new Set(list.map((r) => r.version))];
    if (versions.length > 1) {
        const where = list.map((r) => `${r.file}:${r.line}=${r.version}`).join(', ');
        problems.push(`${key} is referenced with ${versions.length} versions (${versions.join(', ')}): ${where}`);
    }
}
const vendorVersions = [...new Set((groups.get('vendor') || []).map((r) => r.version))];
for (const asset of ['stm32_periph_wasm.js', 'stm32_periph_wasm_bg.wasm', 'stm32f407.svd']) {
    if (!refs.some((r) => r.asset === asset)) problems.push(`vendor asset ${asset} is not referenced anywhere (scan broke?)`);
}

// 3: referenced files exist.
for (const r of refs) {
    if (!existsSync(join(SITE, r.asset)) && !existsSync(join(SITE, 'vendor', r.asset))) {
        problems.push(`${r.asset} referenced at ${r.file}:${r.line} does not exist`);
    }
}

// 4: the DOOM build stamp.
const doom = readFileSync(join(SITE, 'doom.js'), 'utf8');
const stamp = doom.match(/__doomVer\s*=\s*['"`]?(\d+)/) || doom.match(/__doomVer[^\n]*?(\d+)/);
if (!stamp) problems.push('doom.js does not define __doomVer (the page build stamp)');

// Anti-vacuity: the scan must see the real graph.
if (refs.length < 15) problems.push(`only ${refs.length} versioned references found (expected >= 15; scan broke?)`);
if (vendorVersions.length === 0) problems.push('no vendor references found');

for (const [key, list] of [...groups].sort((a, b) => a[0].localeCompare(b[0]))) {
    const v = [...new Set(list.map((r) => r.version))].join('/');
    const where = [...new Set(list.map((r) => `${r.file}:${r.line}`))].join(', ');
    console.log(`  ${key.padEnd(24)} v${v.padEnd(5)} ${where}`);
}
if (problems.length) {
    console.log('');
    for (const p of problems) console.log(`FAIL ${p}`);
    console.log(`\nVERSIONS FAIL (${problems.length} problem${problems.length === 1 ? '' : 's'})`);
    process.exit(1);
}
console.log(`\nVERSIONS PASS (${refs.length} references, ${groups.size} groups, vendor v${vendorVersions[0]}, __doomVer ${stamp[1]})`);
