/// <reference types="node" />

import { createEmulator, CreateEmulatorOpts, EmulatorHandle } from './site/emulator.js';

export * from './site/emulator.js';

export interface CreateSTM32F407Opts extends CreateEmulatorOpts {
  /** A Uint8Array, or a key into the bundled FIRMWARES table. */
  firmware: Uint8Array | string;
}

export function createSTM32F407(opts?: CreateSTM32F407Opts): Promise<EmulatorHandle>;
export function decodeFirmware(key: string): Uint8Array;
export function createNetSim(opts?: any): any;

export const FIRMWARES: Record<string, { bytes: string; [key: string]: unknown }>;
export const bindings: any;
export const unicornFactory: any;
export const svdXml: string;

// STM32F4 high-level facade (see site/stm32f4.js). F1-API parity surface:
// factories, execute/step results, GPIO/USART/SPI/I2C classes, symbol
// helpers, and engine pass-throughs. TX is a single shared model buffer
// (only usart1.onData/output fire); event callbacks beyond SPI/I2C taps
// are absent by design (no core event queue on F4).
export interface ExecuteResult { instCount: number; stopped: boolean; }
export interface FacadeStepResult { pc: number; instCount: number; stopped: boolean; }
export class GPIOPin {
  constructor(mcu: STM32F4, port: string, pin: number);
  on(event: 'change', cb: (high: boolean) => void): () => void;
  addListener(cb: (high: boolean) => void): () => void;
  read(): boolean;
  readInput(): boolean;
  setInput(high: boolean): void;
  setInputValue(high: boolean): void;
  detach(): void;
}
export class GPIO {
  constructor(mcu: STM32F4);
  pin(port: string | number, pin: number): GPIOPin;
}
export interface ChipRecord {
  key: string;
  svd: string;
  flash_size: number;
  ram_size: number;
  maxClockMHz: number;
  label: string;
  idcode: number;
  usarts: number[];
  timers: number[];
  can: number[];
  dac: boolean;
  ltdc: boolean;
  gpioBanks: number;
  spi: number[];
  i2c: number[];
  uart78: number[];
  i2s: number[];
  sai: number[];
  sdio: boolean;
  dcmi: boolean;
  crc: boolean;
  cryp: boolean;
  hash: boolean;
  rng: boolean;
  fsmc: boolean;
  dma2d: boolean;
  qspi: boolean;
}
export declare const CHIPS: Record<string, Omit<ChipRecord, 'key'>>;
export declare function chipInfo(key?: string): ChipRecord;
export class USART {
  readonly n: number;
  onData: ((byte: number) => void) | null;
  constructor(mcu: STM32F4, n: number);
  send(data: string | Uint8Array | number[]): void;
  sendData(data: string | Uint8Array | number[]): void;
  get output(): string;
}
export class SPI {
  readonly ch: number;
  onTransfer: ((channel: string, tx: number[], rx: number[]) => void) | null;
  constructor(mcu: STM32F4, ch: number);
  injectMiso(bytes: Uint8Array | number[]): void;
}
export class I2C {
  readonly ch: number;
  onStart: ((addr: number, isRead: boolean) => void) | null;
  onWrite: ((byte: number) => void) | null;
  onRead: (() => void) | null;
  onStop: (() => void) | null;
  constructor(mcu: STM32F4, ch: number);
  injectRx(bytes: Uint8Array | number[]): void;
}
export class DMAStream {
  constructor(mcu: STM32F4, streamIndex: number);
  pendingCount(): number;
  setCompleted(success?: boolean): void;
  tcif(): boolean;
  htif(): boolean;
  cr(): number;
  ndtr(): number;
}
export class DMAController {
  constructor(mcu: STM32F4, controller?: 1 | 2);
  lisr(): number;
  hisr(): number;
  tcif(s: number): boolean;
  htif(s: number): boolean;
  cr(s: number): number;
  ndtr(s: number): number;
  par(s: number): number;
  m0ar(s: number): number;
  m1ar(s: number): number;
  fcr(s: number): number;
  stream(s: number): DMAStream;
}
export class Display {
  constructor(mcu: STM32F4);
  get oled(): { w: number; h: number; fb: Uint8Array; frame: number } | null;
  get tft(): { w: number; h: number; fb: Uint8Array; frame: () => number } | null;
  ltdc(): { w: number; h: number; pf: string; fb: Uint8Array; addr: number; pitch: number } | null;
}
export class STM32F4 {
  readonly gpio: GPIO;
  readonly usart: USART;
  // Slots for absent-silicon USARTs are null (F401/F411: 3,4,5).
  readonly usart1: USART; readonly usart2: USART; readonly usart3: USART | null;
  readonly usart4: USART | null; readonly usart5: USART | null; readonly usart6: USART;
  readonly usart7: USART | null; readonly usart8: USART | null;
  readonly usarts: Record<number, USART>;
  readonly spi1: SPI | null; readonly spi2: SPI; readonly spi3: SPI;
  readonly spi4: SPI | null; readonly spi5: SPI | null; readonly spi6: SPI | null;
  readonly spiBus: Record<number, SPI>;
  readonly i2c1: I2C; readonly i2c2: I2C; readonly i2c3: I2C;
  readonly i2cBus: Record<number, I2C>;
  readonly spi: { specs: any[]; pushMiso(peripheral: string, bytes: Uint8Array | number[]): void };
  readonly i2c: { specs: any[]; pushRx(peripheral: string, bytes: Uint8Array | number[]): void };
  readonly dma: { stream(index: number): DMAStream; controller(ctl: 1 | 2): DMAController };
  readonly display: Display;
  readonly chip: ChipRecord;
  readonly chipSvdXml: string | null;
  // Wokwi/OpenHW/Velxio integration callbacks (polled dispatch, see site/stm32f4.js).
  onExtiEdge: ((line: number) => void) | null;
  onCanTx: ((can: number) => void) | null;
  onCanRx: ((can: number, id: number, len: number, data: number[]) => void) | null;
  onTimUpdate: ((tim: number) => void) | null;
  onTimCapture: ((tim: number, ch: number, val: number) => void) | null;
  onDmaTc: ((controller: number, stream: number) => void) | null;
  onWdogReset: ((which: number) => void) | null;
  onUsbIn: ((ep: number, data: number[]) => void) | null;
  onUsbHsIn: ((ep: number, data: number[]) => void) | null;
  onItmByte: ((port: number, byte: number) => void) | null;
  onFsmcAccess: ((bank: number, off: number, write: boolean, size: number, val: number) => void) | null;
  onAdcDone: ((adc: number, chan: number, value: number) => void) | null;
  onDacWrite: ((chan: number, value: number) => void) | null;
  onCrcResult: ((value: number) => void) | null;
  onRtcAlarm: ((which: number) => void) | null;
  onI2cAlert: ((peripheral: number, asserted: boolean) => void) | null;
  onHostTx: ((...args: any[]) => void) | null;
  onHostRx: ((...args: any[]) => void) | null;
  constructor(emu: any, bindings?: any, chip?: ChipRecord, chipSvdXml?: string | null);
  static create(opts?: any): Promise<STM32F4>;
  static fromELF(buf: Uint8Array | ArrayBuffer, opts?: any): Promise<STM32F4>;
  static fromBin(buf: Uint8Array | ArrayBuffer, opts?: any): Promise<STM32F4>;
  static fromHex(text: string, opts?: any): Promise<STM32F4>;
  loadBin(bytes: Uint8Array | ArrayBuffer, base?: number): void;
  loadHex(text: string): void;
  loadELF(bytes: Uint8Array | ArrayBuffer): void;
  // Live peripheral injection (anytime unless noted).
  setAdcChannel(peripheral: string, channel: number, value: number): void;
  clearAdcChannel(peripheral: string, channel: number): void;
  takeAdcDma(): number[];
  canInject(id: number, dlc: number, data: Uint8Array | number[]): void;
  canInjectFd(id: number, data: Uint8Array | number[], brs?: boolean): void;
  timInjectCapture(timer: number, ch: number): void;
  timPwmPulseUs(timer: number, ch: number, clockHz: number): number;
  timOcMode(timer: number, ch: number): number;
  timMoe(timer: number): boolean;
  timEncoderStep(timer: number, ti: number, rising: boolean): void;
  timBreakInput(timer: number, asserted: boolean): void;
  dacTrigger(ch: number, src: number, dmaStaged?: boolean): void;
  dacUnderrun(ch: number): boolean;
  takeSpeakerSamples(): Float32Array;
  usbInjectSetup(bytes: Uint8Array | number[]): boolean;
  usbInjectOut(ep: number, bytes: Uint8Array | number[]): boolean;
  usbTakeIn(ep: number): number[];
  usbReset(): void;
  usbEnumerated(): void;
  usbHsInjectSetup(bytes: Uint8Array | number[]): boolean;
  usbHsInjectOut(ep: number, bytes: Uint8Array | number[]): boolean;
  usbHsTakeIn(ep: number): number[];
  usbHsReset(): void;
  usbHsEnumerated(): void;
  ethInject(frame: Uint8Array | number[]): void;
  // Fault / model-harness methods (mirror wasm exports 1:1).
  uartSetCts(base: number, asserted: boolean): void;
  uartFaultRx(base: number, fe: boolean, pe: boolean): void;
  uartIdle(base: number): void;
  uartLinBreak(base: number): void;
  uartBreakPending(base: number): boolean;
  uartTxLen(base: number): number;
  uartMuted(base: number): boolean;
  spiFaultCrc(base: number): void;
  spiFaultModf(base: number): void;
  spiSlaveSelect(base: number, asserted: boolean): void;
  spiSlaveClock(base: number, mosi: number): number;
  spiSlaveGate(base: number): boolean;
  i2cArmArbLoss(base: number): void;
  i2cArmSmbusAlert(base: number, addr: number): void;
  i2cSlaveAddress(base: number, addr: number, isRead: boolean): boolean;
  i2cSlaveWrite(base: number, byte: number): void;
  i2cSlaveRead(base: number): number;
  i2cSlaveStop(base: number): void;
  i2cSlaveStatus(base: number): number;
  sdioFaultDataCrc(): void;
  rccInjectFailure(srcMask: number, dead: boolean): void;
  flashRdpLevel(): number;
  flashSetRdp(levelByte: number): void;
  rngSeedEntropy(words: number[] | Uint32Array): void;
  rngEntropyAvail(): number;
  rtcTamperPin(level: boolean): void;
  rtcTimestamp(): void;
  setIntrPending(irq: number): void;
  hasPendingInterrupt(): boolean;
  // Scope probes (read-only model state).
  adcDualLatched(): boolean;
  ethPpsCount(): number;
  ethPpsLevel(): boolean;
  ethLinkUp(): boolean;
  ethSetLink(up: boolean): void;
  ltdcScanline(): number;
  ltdcFrameCount(): number;
  audioRemaining(): number;
  audioClear(): void;
  dcmiSync(vsync: boolean, hsync: boolean, pclkDiv: number): void;
  qspiMmapLive(): boolean;
  qspiMmapRead(offset: number): number;
  sdioBusWidth(): number;
  sdioCardBlocks(): number;
  sdioReadBlock(block: number): number[];
  takeItm(port: number): number[];
  itmPending(port: number): number;
  takeFsmc(bank: number): number[];
  pushFsmc(bank: number, values: number[] | Uint32Array): void;
  regfileGet(peripheral: string, offset: number): number;
  regfileSet(peripheral: string, offset: number, value: number): void;
  get oled(): any;
  get tft(): any;
  get rtc(): any;
  get buzzer(): any;
  get camera(): any;
  execute(cycles?: number): ExecuteResult;
  step(cycles?: number): FacadeStepResult;
  uartRx(byte: number): boolean;
  get uartOutput(): string;
  setSymbols(mapText: string): number;
  resolveSymbol(pc: number): string | null;
  read32(addr: number): number;
  write32(addr: number, val: number): void;
  memRead32(addr: number): number;
  memWriteBytes(addr: number, bytes: Uint8Array | number[]): void;
  periphRead(addr: number, width?: number): number;
  periphWrite(addr: number, width: number, value: number): void;
  addJsPeripheral(base: number, size: number, read: (addr: number, size: number) => number, write: (addr: number, value: number, size: number) => void): boolean;
  pwrMode(): number;
  pwrEstimate(): number;
  swdHalted(): boolean;
  swdHalt(): void;
  swdResume(): void;
  swdStep(): number;
  swdAddWatch(kind: number, addr: number, len: number): number;
  swdRemoveWatch(slot: number): void;
  swdTakeTrip(): number[];
  swdDpRead(addr: number): number;
  swdDpWrite(addr: number, value: number): void;
  swdApRead(bank: number, reg: number): number;
  swdApWrite(bank: number, reg: number, value: number): void;
  swdRegRead(idx: number): number;
  swdRegWrite(idx: number, value: number): void;
  jtagReset(): void;
  jtagIr(ir: number): void;
  jtagIdcode(): number;
  jtagDp(addr: number, rnw: boolean, wdata: number): number;
  jtagAp(bank: number, reg: number, rnw: boolean, wdata: number): number;
  dmaController(ctl?: 1 | 2): DMAController;
  getRegisters(): any;
  getPc(): number;
  getSp(): number;
  faultInfo(): any;
  takeFault(): [number, number] | null;
  stop(): void;
  reset(): void;
  close(): void;
  resetCpu(): void;
  setNrst(asserted: boolean): boolean;
  isNrstAsserted(): boolean;
  bootPreset(image: any): void;
  ledStatus(fwName: string, boardKey: string): any;
}
export class LED {
  constructor(emu: any, port: string, num: number, opts?: { activeLow?: boolean });
  on(): void;
  off(): void;
  toggle(): void;
  read(): boolean;
  onChange(cb: (on: boolean) => void): void;
}
export class Button {
  constructor(emu: any, port: string, num: number, opts?: { activeLow?: boolean });
  press(): void;
  release(): void;
  read(): boolean;
  on(evt: 'down' | 'up', cb: () => void): void;
}
export class Pwm {
  constructor(emu: any, timer: string, channel?: number, opts?: { clockHz?: number });
  setDuty(percent: number): void;
  read(): number;
}
export class Potentiometer {
  constructor(emu: any, peripheral: string, channel?: number, opts?: { min?: number; max?: number });
  set(value: number): void;
}
export class I2cRegisterDevice {
  constructor(emu: any, peripheral: string, opts?: any);
}

// Component-attachment API (attach devices to an emulator handle).
// Board LED map + host reset/boot control (see site/boards.js,
// site/emulator.js, site/stm32f4.js).
export interface BoardLed { bank: number; pin: number; label: string; }
export declare const BOARD_LED: Record<string, BoardLed>;
export declare const BOARD_LED_ALIASES: Record<string, BoardLed>;
export declare function boardLed(fwName: string, boardKey: string): BoardLed;
