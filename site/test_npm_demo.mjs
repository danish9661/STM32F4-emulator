// npm demo smoke test — the one command a fresh consumer runs.
// Boots the bundled `blinky` demo through the PUBLISHED entry point
// (index.mjs, the same import an `npm install stm32f4-emu` user gets),
// asserts the boot banner + LED toggles, prints PASS, exit 0.
// NOTE: deliberately uses only index.mjs + the MIT/BSD bundle. It never
// touches demo-only GPL payload (firmware/doom, site/doom*.html/js, WAD).
// Usage: npm run test:demo  (also runs as the last step of `npm test`)
import { createSTM32F407, decodeFirmware } from '../index.mjs';

const emu = await createSTM32F407({ firmware: decodeFirmware('blinky') });

const uart = [];
let prevOdr = -1, toggles = 0;
for (let i = 0; i < 400 && toggles < 8; i++) {
    emu.step(100000);
    uart.push(emu.drainUart());
    const odr = emu.read32(0x40020014) & 0x20;
    if (prevOdr >= 0 && odr !== prevOdr) toggles++;
    prevOdr = odr;
}
const all = uart.join('');
const count = (s) => (all.split(s).length - 1);
console.log(`led_toggles=${toggles} ledOn=${count(' LED=ON')} ledOff=${count(' LED=OFF')}`);
console.log('uart tail:', all.replace(/\r/g, '').split('\n').filter(Boolean).slice(-3).join(' | '));

const pass =
    all.includes('=== Blinky ===') &&
    all.includes('No ethernet required') &&
    count('tick 0 LED=ON') === 1 &&
    toggles >= 2;
console.log(pass ? 'PASS' : 'FAIL');
emu.close();
process.exit(pass ? 0 : 1);
