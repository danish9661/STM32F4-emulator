// tools/sync-website-docs.mjs — regenerate website/docs/ from docs/ (single source).
// The Docusaurus site needs front-matter the raw docs don't carry, so this
// script prepends it (titles/descriptions kept here, next to the manifest).
// Run after editing any source doc, together with sync-docs.mjs:
//   node tools/sync-docs.mjs && node tools/sync-website-docs.mjs
import { readFileSync, writeFileSync, mkdirSync, existsSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const outDir = join(root, 'website', 'docs');

// [source path (repo root), website file, front-matter lines]
const DOCS = [
    ['docs/intro.md', 'intro.md',
        ['slug: /', 'sidebar_position: 1', 'title: Introduction']],
    ['docs/usage.md', 'usage.md',
        ['sidebar_position: 2', 'title: Usage',
         'description: How to run the emulator from the CLI (Node.js) and the browser console.']],
    ['NETWORKING.md', 'networking.md',
        ['sidebar_position: 3', 'title: Networking',
         'description: Ethernet on the emulated F407/F429 — MAC/PHY, filters, PTP, WOL, LwIP sockets, netsim and gateway.']],
    ['docs/boards.md', 'boards.md',
        ['sidebar_position: 3', 'title: Boards',
         'description: Supported STM32F4 boards and chips — what is implemented, what was verified, and what is left.']],
    ['docs/boards/stm32f401.md', 'boards/stm32f401.md', ['title: Board STM32F401']],
    ['docs/boards/stm32f411.md', 'boards/stm32f411.md', ['title: Board STM32F411']],
    ['docs/boards/stm32f407.md', 'boards/stm32f407.md', ['title: Board STM32F407']],
    ['docs/boards/stm32f407ve.md', 'boards/stm32f407ve.md', ['title: Board STM32F407VE/ZE']],
    ['docs/boards/stm32f429.md', 'boards/stm32f429.md', ['title: Board STM32F429']],
    ['docs/components.md', 'components.md',
        ['sidebar_position: 3', 'title: Components',
         'description: Public JS API for wiring virtual hardware — LEDs, buttons, displays, ADC channels, SPI/I2C devices.']],
    ['docs/architecture.md', 'architecture.md',
        ['sidebar_position: 4', 'title: Architecture',
         'description: How the STM32F407 emulator is structured — CPU core, Rust peripheral model, JS drivers, and how they communicate.']],
    ['docs/peripherals.md', 'peripherals.md',
        ['sidebar_position: 5', 'title: Peripherals',
         'description: Full implementation matrix of all 40 emulated peripherals — register coverage, behavior level, and firmware coverage.']],
    ['docs/benchmarks.md', 'benchmarks.md',
        ['sidebar_position: 6', 'title: Benchmarks',
         'description: Performance measurements — throughput, soak results, tuning history, and environment reproducibility.']],
    ['docs/mcp.md', 'mcp.md',
        ['sidebar_position: 7', 'title: MCP Server',
         'description: Expose the emulator as Model Context Protocol tools — firmware debugging driven by an AI agent.']],
    ['docs/progress-and-future.md', 'progress-and-future.md',
        ['sidebar_position: 8', 'title: Progress & Future',
         'description: What works today, known limitations, and the implementation roadmap.']],
];

for (const [src, file, fm] of DOCS) {
    const text = readFileSync(join(root, src), 'utf8');
    const dest = join(outDir, file);
    mkdirSync(dirname(dest), { recursive: true });
    writeFileSync(dest, '---\n' + fm.join('\n') + '\n---\n\n' + text);
}
console.log(`synced ${DOCS.length} docs -> website/docs/`);
