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

// Component-attachment API (attach devices to an emulator handle).
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
