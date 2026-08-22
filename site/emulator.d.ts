/// <reference types="node" />

// Type declarations for the STM32F407 universal emulator factory.
// Kept intentionally pragmatic: the peripheral `bindings`/`unicorn` objects are
// the wasm-bindgen / Unicorn WASM modules and are typed loosely.

export interface ExtraRamRegion {
  addr: number;
  size: number;
}

export interface ExtraMemSegment {
  addr: number;
  data: Uint8Array;
}

export interface CreateEmulatorOpts {
  /** Raw firmware image (Cortex-M vector table at the start). */
  firmware: Uint8Array;
  /** wasm-bindgen module (web or nodejs build). */
  bindings: any;
  /** Async factory returning the Unicorn module. */
  unicorn: any;
  /** SVD XML string for init_svd. */
  svdXml?: string;
  /** Optional wasm bytes for bindings.default() (Node). */
  wasmInit?: Uint8Array;

  flash_size?: number;
  ram_size?: number;
  vector_table?: number;

  tickEvery?: number;
  pollEvery?: number;
  minimalPolls?: boolean;
  blockCounting?: boolean;
  noCountHook?: boolean;
  perInstHook?: boolean;
  maxBatch?: number;

  onTx?: ((frame: Uint8Array, meta: unknown) => void) | null;
  eth?: Record<string, number>;
  ext_devices?: Record<string, unknown>;
  extra_ram?: ExtraRamRegion[];
  extra_mem?: ExtraMemSegment[];
  uart_addr?: number;

  enable_irqs?: boolean;
  irq_eth?: boolean;
  freertos?: boolean;
  /** Trace peripheral MMIO read/write to stderr (capped). */
  verbose?: boolean;
}

export interface StepResult {
  pc: number;
  stopped: boolean;
  instCount: number;
}

export interface RunResult {
  totalSteps: number;
  instCount: number;
}

export interface EmulatorHandle {
  uc: any;
  Module: any;
  step(maxInst?: number): StepResult;
  run(maxInstructions?: number): RunResult;
  drainUart(): Uint8Array;
  read32(addr: number): number;
  write32(addr: number, val: number): void;
  read16(addr: number): number;
  write16(addr: number, val: number): void;
  read8(addr: number): number;
  write8(addr: number, val: number): void;
  close(): void;
  // The handle exposes additional device/tap methods; kept open for forward
  // compatibility without forcing every addition into this file.
  [key: string]: any;
}

export function createEmulator(opts: CreateEmulatorOpts): Promise<EmulatorHandle>;
