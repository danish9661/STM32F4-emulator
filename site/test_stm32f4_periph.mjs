// Wokwi-style virtual-peripheral API tests: SPI onTransfer + I2C
// onStart/onWrite/onStop over the existing bus taps. Run:
//   node site/test_stm32f4_periph.mjs
import { STM32F4, decodeFirmware } from '../index.mjs';

let failures = 0;
function check(cond, msg) {
    if (!cond) { failures++; console.error('FAIL:', msg); }
    else console.log('ok  :', msg);
}

// ── I2C: drive oled_test (SSD1306 over I2C1@0x3C) ──
let i2cStarts = 0, i2cStartAddr = null, i2cWrites = 0, i2cWriteBytes = [];
const i2cMcu = await STM32F4.create({
    spi: [],
    i2c: [{
        peripheral: 'I2C1', address: 0x3C,
        onStart: (addr, isRead) => { i2cStarts++; i2cStartAddr = addr; },
        onWrite: (b) => { i2cWrites++; i2cWriteBytes.push(b); },
        onStop: () => {},
    }],
});
i2cMcu.loadBin(decodeFirmware('oled_test'));
for (let i = 0; i < 400 && i2cWrites < 4; i++) i2cMcu.execute(50000);
check(i2cStarts >= 1, 'I2C: onStart fired');
check(i2cStartAddr === 0x3C, 'I2C: onStart address is 0x3C');
check(i2cWrites >= 4, `I2C: onWrite fired (${i2cWrites} bytes)`);
check(i2cWriteBytes.length >= 4 && i2cWriteBytes.every((b) => typeof b === 'number'), 'I2C: onWrite received byte values');
i2cMcu.close();

// ── SPI: drive tft_test (ILI9341 over SPI2, CS PB12, DC PB11) ──
let spiTransfers = 0, spiTxBytes = 0;
const spiMcu = await STM32F4.create({
    spi: [{
        peripheral: 'SPI2', cs: 'PB12', dc: 'PB11',
        onTransfer: (ch, tx, rx) => { spiTransfers++; spiTxBytes += tx.length; },
    }],
});
spiMcu.loadBin(decodeFirmware('tft_test'));
for (let i = 0; i < 400 && spiTransfers < 2; i++) spiMcu.execute(50000);
check(spiTransfers >= 1, `SPI: onTransfer fired (${spiTransfers} transfers, ${spiTxBytes} bytes)`);
check(spiTxBytes >= 4, 'SPI: onTransfer received non-empty TX buffer');
spiMcu.close();

// ── push helpers exist and are callable ──
const mcu = await STM32F4.create();
mcu.spi.pushMiso('SPI2', [0xAA]);
mcu.i2c.pushRx('I2C1', [0x55]);
check(true, 'spi.pushMiso / i2c.pushRx callable');
mcu.close();

if (failures) { console.error(`\n${failures} FAILED`); process.exit(1); }
console.log('\nALL PASS');
