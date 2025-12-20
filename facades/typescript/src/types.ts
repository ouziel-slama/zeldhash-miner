export enum ZeldMinerErrorCode {
  INVALID_ADDRESS = "INVALID_ADDRESS",
  UNSUPPORTED_ADDRESS_TYPE = "UNSUPPORTED_ADDRESS_TYPE",
  INSUFFICIENT_FUNDS = "INSUFFICIENT_FUNDS",
  NO_CHANGE_OUTPUT = "NO_CHANGE_OUTPUT",
  MULTIPLE_CHANGE_OUTPUTS = "MULTIPLE_CHANGE_OUTPUTS",
  INVALID_INPUT = "INVALID_INPUT",
  WEBGPU_NOT_AVAILABLE = "WEBGPU_NOT_AVAILABLE",
  WORKER_ERROR = "WORKER_ERROR",
  MINING_ABORTED = "MINING_ABORTED",
  DUST_OUTPUT = "DUST_OUTPUT",
}

export type ZeldMinerErrorDetails = Record<string, unknown>;

export type Network = "mainnet" | "testnet" | "signet" | "regtest";

export interface ZeldMinerOptions {
  network: Network;
  batchSize: number;
  useWebGPU: boolean;
  workerThreads: number;
  satsPerVbyte: number;
}

export interface MiningCoordinatorOptions {
  mode: WorkerMode;
  batchSize: number;
  workerThreads: number;
}

export interface TxInput {
  txid: string;
  vout: number;
  scriptPubKey: string;
  amount: number;
}

export interface TxOutput {
  address: string;
  amount?: number;
  change: boolean;
}

export interface MineResult {
  psbt: string;
  txid: string;
  nonce: bigint;
  attempts: bigint;
  duration: number;
  hashRate: number;
}

export interface ProgressEvent {
  hashesProcessed: bigint;
  hashRate: number;
  elapsedMs?: number;
  lastNonce?: bigint;
  workerId?: string;
}

export type WorkerMode = "cpu" | "gpu";

export type WorkerMessage =
  | { type: "init"; mode: WorkerMode }
  | {
      type: "mine";
      inputs: TxInput[];
      outputs: TxOutput[];
      network: Network;
      satsPerVbyte: number;
      template: WorkerTemplate;
      startNonce: bigint;
      batchSize: number;
      targetZeros: number;
      nonceStep?: bigint;
      distribution?: bigint[];
    }
  | { type: "stop" };

export type WorkerResponse =
  | { type: "ready"; workerId?: string }
  | {
      type: "progress";
      hashesProcessed: bigint;
      hashRate: number;
      lastNonce?: bigint;
      workerId?: string;
    }
  | {
      type: "found";
      result: MineResult;
      hashesProcessed?: bigint;
      hashRate?: number;
      lastNonce?: bigint;
      workerId?: string;
    }
  | { type: "batch_complete"; lastNonce: bigint; workerId?: string }
  | {
      type: "error";
      message: string;
      code?: ZeldMinerErrorCode;
      details?: ZeldMinerErrorDetails;
      workerId?: string;
    };

export interface MiningState {
  mode: WorkerMode;
  running: boolean;
  startNonce: bigint;
  nextNonce: bigint;
  batchSize: number;
  targetZeros: number;
  hashesProcessed: bigint;
  lastHashRate?: number;
}

export interface MiningTemplate {
  prefix: Uint8Array;
  suffix: Uint8Array;
  useCborNonce?: boolean;
}

export interface WorkerTemplate extends MiningTemplate {
  nonceLength: number;
  useCborNonce?: boolean;
}

export interface ValidationResult {
  ok: boolean;
  error?: string;
  addressType?: "p2wpkh" | "p2tr";
  network?: Network;
}

export interface WasmExports {
  init_panic_hook: () => void;
  mine_batch_wasm: (
    txPrefix: Uint8Array,
    txSuffix: Uint8Array,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    useCborNonce?: boolean
  ) => ValidationResult | MineResult | null;
  mine_batch_gpu?: (
    txPrefix: Uint8Array,
    txSuffix: Uint8Array,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    useCborNonce?: boolean
  ) => Promise<ValidationResult | MineResult | null>;
  mine_range_wasm: (
    inputs: TxInput[],
    outputs: TxOutput[],
    network: Network,
    satsPerVbyte: bigint,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    distribution?: bigint[] | null
  ) => ValidationResult | MineResult | null;
  mine_range_gpu?: (
    inputs: TxInput[],
    outputs: TxOutput[],
    network: Network,
    satsPerVbyte: bigint,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    distribution?: bigint[] | null
  ) => Promise<ValidationResult | MineResult | null>;
  validate_address: (addr: string, network: Network) => ValidationResult;
  build_psbt: (
    inputs: TxInput[],
    outputs: TxOutput[],
    network: Network,
    satsPerVbyte: bigint,
    nonce: bigint,
    distribution?: bigint[] | null
  ) => string;
  build_mining_template: (
    inputs: TxInput[],
    outputs: TxOutput[],
    network: Network,
    satsPerVbyte: bigint,
    startNonce: bigint,
    batchSize: number,
    distribution?: bigint[] | null
  ) => MiningTemplate;
  compute_txid: (txBytes: Uint8Array) => string;
  init_gpu?: () => Promise<unknown>;
  calibrate_batch_size?: () => Promise<number>;
}

export type ProgressStats = ProgressEvent;

export interface MineParams {
  inputs: TxInput[];
  outputs: TxOutput[];
  targetZeros: number;
  startNonce?: bigint;
  batchSize?: number;
  signal?: AbortSignal;
  distribution?: bigint[];
}

