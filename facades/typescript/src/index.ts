import { MiningCoordinator } from "./coordinator";
import { TransactionBuilder } from "./builder";
import { splitNonceSegments, splitNonceSegmentsCbor } from "./nonce";
import type {
  MineParams,
  MineResult,
  ProgressStats,
  WorkerMode,
  ZeldMinerOptions,
} from "./types";
import { ZeldMinerErrorCode } from "./types";
import { createMinerError, toZeldMinerError, ZeldMinerError } from "./errors";

type ZeldMinerEvent = "progress" | "found" | "error" | "stopped";

type ZeldMinerEventMap = {
  progress: ProgressStats;
  found: MineResult;
  error: ZeldMinerError;
  stopped: void;
};

const MIN_TARGET_ZEROS = 1;
const MAX_TARGET_ZEROS = 32;

export class ZeldMiner {
  private readonly options: ZeldMinerOptions;
  private readonly builder: TransactionBuilder;
  private coordinator: MiningCoordinator | null = null;
  private readonly listeners: {
    [K in ZeldMinerEvent]: Set<(payload: ZeldMinerEventMap[K]) => void>;
  };
  private state: "idle" | "running" | "paused" = "idle";
  private stopRequested = false;

  constructor(options: ZeldMinerOptions) {
    this.options = this.validateOptions(options);
    this.builder = new TransactionBuilder(options.network, options.satsPerVbyte);
    this.listeners = {
      progress: new Set(),
      found: new Set(),
      error: new Set(),
      stopped: new Set(),
    };
  }

  on<K extends ZeldMinerEvent>(
    event: K,
    handler: (payload: ZeldMinerEventMap[K]) => void
  ): void {
    this.listeners[event].add(handler);
  }

  off<K extends ZeldMinerEvent>(
    event: K,
    handler: (payload: ZeldMinerEventMap[K]) => void
  ): void {
    this.listeners[event].delete(handler);
  }

  private emit<K extends ZeldMinerEvent>(
    event: K,
    payload: ZeldMinerEventMap[K]
  ): void {
    this.listeners[event].forEach((handler) => handler(payload));
  }

  private validateOptions(options: ZeldMinerOptions): ZeldMinerOptions {
    if (!Number.isInteger(options.batchSize) || options.batchSize <= 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "batchSize must be a positive integer",
        { field: "batchSize" }
      );
    }
    if (!Number.isInteger(options.workerThreads) || options.workerThreads <= 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "workerThreads must be a positive integer",
        { field: "workerThreads" }
      );
    }
    if (!Number.isFinite(options.satsPerVbyte) || options.satsPerVbyte <= 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "satsPerVbyte must be a positive number",
        { field: "satsPerVbyte" }
      );
    }
    return options;
  }

  private async selectBackend(): Promise<WorkerMode> {
    if (!this.options.useWebGPU) return "cpu";
    if (typeof navigator === "undefined") return "cpu";

    const gpu = (navigator as Navigator & { gpu?: { requestAdapter?: () => Promise<unknown> } }).gpu;
    const adapter =
      typeof gpu?.requestAdapter === "function" ? await gpu.requestAdapter() : null;
    return adapter ? "gpu" : "cpu";
  }

  private clearCoordinatorHandlers(
    coordinator: MiningCoordinator,
    handlers: Partial<Record<ZeldMinerEvent, (...args: any[]) => void>>
  ): void {
    if (handlers.progress) {
      coordinator.off("progress", handlers.progress as (payload: ProgressStats) => void);
    }
    if (handlers.found) {
      coordinator.off("found", handlers.found as (payload: MineResult) => void);
    }
    if (handlers.error) {
      coordinator.off("error", handlers.error as (payload: Error) => void);
    }
    if (handlers.stopped) {
      coordinator.off("stopped", handlers.stopped as (payload: void) => void);
    }
  }

  async mineTransaction(params: MineParams): Promise<MineResult> {
    if (this.state === "running") {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "Mining is already in progress"
      );
    }

    if (
      !Number.isInteger(params.targetZeros) ||
      params.targetZeros < MIN_TARGET_ZEROS ||
      params.targetZeros > MAX_TARGET_ZEROS
    ) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        `targetZeros must be an integer between ${MIN_TARGET_ZEROS} and ${MAX_TARGET_ZEROS}`,
        { targetZeros: params.targetZeros }
      );
    }

    const startNonce = params.startNonce ?? 0n;
    const batchSize = params.batchSize ?? this.options.batchSize;

    if (!Number.isInteger(batchSize) || batchSize <= 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "batchSize must be a positive integer",
        { batchSize }
      );
    }

    if (params.signal?.aborted) {
      throw createMinerError(
        ZeldMinerErrorCode.MINING_ABORTED,
        "Abort signal already triggered"
      );
    }

    const distribution = params.distribution;
    const useCborNonce = Boolean(distribution && distribution.length > 0);
    let firstSegmentSize = batchSize;
    try {
      const [firstSegment] = useCborNonce
        ? splitNonceSegmentsCbor(startNonce, batchSize)
        : splitNonceSegments(startNonce, batchSize);
      if (!firstSegment) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          "Failed to compute nonce segments",
          { startNonce, batchSize }
        );
      }
      firstSegmentSize = firstSegment.size;
    } catch (err) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        err instanceof Error ? err.message : String(err),
        { startNonce, batchSize }
      );
    }

    const mode = await this.selectBackend();
    const template = await this.builder.buildMiningTemplate({
      inputs: params.inputs,
      outputs: params.outputs,
      startNonce,
      batchSize: firstSegmentSize,
      distribution,
    });

    const network =
      this.options.network === "signet" ? "testnet" : this.options.network;
    const coordinator = new MiningCoordinator({
      mode,
      batchSize,
      workerThreads: this.options.workerThreads,
    });

    this.coordinator = coordinator;
    this.state = "running";
    this.stopRequested = false;

    return new Promise<MineResult>((resolve, reject) => {
      let settled = false;

      const cleanup = (): void => {
        this.clearCoordinatorHandlers(coordinator, handlers);
        if (this.coordinator === coordinator) {
          this.coordinator = null;
        }
        this.state = "idle";
        this.stopRequested = false;
      };

      const resolveOnce = (value: MineResult): void => {
        if (settled) return;
        settled = true;
        cleanup();
        resolve(value);
      };

      const rejectOnce = (err: unknown): void => {
        if (settled) return;
        settled = true;
        const error = toZeldMinerError(err);
        cleanup();
        this.emit("error", error);
        reject(error);
      };

      const handlers: Partial<Record<ZeldMinerEvent, (...args: any[]) => void>> = {};

      handlers.progress = (stats: ProgressStats): void => {
        this.emit("progress", stats);
      };

      handlers.found = async (result: MineResult): Promise<void> => {
        try {
          const psbt = await this.builder.buildPsbt({
            inputs: params.inputs,
            outputs: params.outputs,
            nonce: result.nonce,
            distribution,
          });

          const finalResult: MineResult = { ...result, psbt };
          this.emit("found", finalResult);
          resolveOnce(finalResult);
        } catch (err) {
          rejectOnce(err);
        }
      };

      handlers.error = (err: Error): void => {
        rejectOnce(err);
      };

      handlers.stopped = (): void => {
        if (settled) return;
        const reason = this.stopRequested
          ? createMinerError(
              ZeldMinerErrorCode.MINING_ABORTED,
              "Mining stopped by caller"
            )
          : createMinerError(
              ZeldMinerErrorCode.MINING_ABORTED,
              "Mining halted"
            );
        this.emit("stopped", undefined as unknown as void);
        rejectOnce(reason);
      };

      coordinator.on("progress", handlers.progress);
      coordinator.on("found", handlers.found);
      coordinator.on("error", handlers.error);
      coordinator.on("stopped", handlers.stopped);

      coordinator
        .start({
          inputs: params.inputs,
          outputs: params.outputs,
          network,
          satsPerVbyte: this.options.satsPerVbyte,
          template,
          targetZeros: params.targetZeros,
          startNonce,
          signal: params.signal,
          distribution,
        })
        .catch(rejectOnce);
    });
  }

  stop(): void {
    if (!this.coordinator) return;
    this.stopRequested = true;
    this.coordinator.stop();
  }

  pause(): void {
    if (this.state !== "running" || !this.coordinator) return;
    this.state = "paused";
    this.coordinator.pause();
  }

  async resume(): Promise<void> {
    if (this.state !== "paused" || !this.coordinator) return;
    this.state = "running";
    await this.coordinator.resume();
  }
}

export type {
  ZeldMinerOptions,
  MineParams,
  MineResult,
  ProgressStats,
  Network,
  TxInput,
  TxOutput,
} from "./types";
export { ZeldMinerErrorCode } from "./types";
export { TransactionBuilder } from "./builder";
export { MiningCoordinator } from "./coordinator";
export {
  ZeldMinerError,
  createMinerError,
  toZeldMinerError,
} from "./errors";

