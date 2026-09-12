// tools/sync-docs.mjs — copy user-facing markdown into site/docs-src/ so the
// static site (GitHub Pages serves ONLY site/) renders docs live.
// Run after editing any source doc, then commit the result together:
//   node tools/sync-docs.mjs
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'site', 'docs-src');
if (!existsSync(outDir)) mkdirSync(outDir, { recursive: true });

// [source path (repo root), site file, title, blurb]
const DOCS = [
    ['docs/intro.md', 'intro.md', 'Introduction', 'What this is, try-it links, boards overview'],
    ['docs/usage.md', 'usage.md', 'Usage guide', 'Console, CLI, firmware loading, options'],
    ['docs/boards.md', 'boards.md', 'Boards & chips', 'Variant table, verification matrix, preset wiring'],
    ['docs/boards/stm32f401.md', 'boards/stm32f401.md', 'Board: STM32F401', '36 presets, no CAN/ETH, BlackPill + Nucleo'],
    ['docs/boards/stm32f411.md', 'boards/stm32f411.md', 'Board: STM32F411', '36 presets, F401 + SPI5'],
    ['docs/boards/stm32f407.md', 'boards/stm32f407.md', 'Board: STM32F407', 'Reference target, 64 presets, feature matrix'],
    ['docs/boards/stm32f407ve.md', 'boards/stm32f407ve.md', 'Board: F407VE/ZE', '512K package, 66 presets'],
    ['docs/boards/stm32f429.md', 'boards/stm32f429.md', 'Board: STM32F429', 'DMA2D/LTDC/Ethernet proofs, 57 presets'],
    ['NETWORKING.md', 'networking.md', 'Networking / Ethernet', 'Per-board support matrix, bridge + gateway'],
    ['docs/peripherals.md', 'peripherals.md', 'Peripheral coverage', 'Model depth per peripheral, IRQs, gaps'],
    ['docs/architecture.md', 'architecture.md', 'Architecture', 'CPU core, driver, stepping, performance'],
    ['docs/components.md', 'components.md', 'Components / devices', 'OLED, TFT, buzzer, RTC panels'],
    ['docs/benchmarks.md', 'benchmarks.md', 'Benchmarks', 'MIPS, soak results, gateway throughput'],
    ['docs/mcp.md', 'mcp.md', 'MCP / AI usage', 'Driving the emulator from AI agents'],
    ['docs/progress-and-future.md', 'progress-and-future.md', 'Progress & future', 'Changelog direction, open items'],
];

const manifest = [];
for (const [src, file, title, blurb] of DOCS) {
    const text = readFileSync(join(root, src), 'utf8');
    const dest = join(outDir, file);
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(dest, text);
    manifest.push({ file, title, blurb });
}
writeFileSync(join(root, 'site', 'docs.json'), JSON.stringify(manifest, null, 1) + '\n');
console.log(`synced ${manifest.length} docs -> site/docs-src/ + site/docs.json`);
