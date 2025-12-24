import type {
  MineResult,
  MiningCoordinatorOptions,
  ProgressEvent as MiningProgressEvent,
  Network,
  TxInput,
  TxOutput,
  WorkerMessage,
  WorkerMode,
  WorkerResponse,
  WorkerTemplate,
} from "./types";
import { ZeldMinerErrorCode } from "./types";
import { ZeldMinerError } from "./errors";

type CoordinatorEvent = "ready" | "progress" | "found" | "error" | "stopped";

type CoordinatorEventMap = {
  ready: void;
  progress: MiningProgressEvent;
  found: MineResult;
  error: ZeldMinerError;
  stopped: void;
};

type CoordinatorListener<K extends CoordinatorEvent> = (
  payload: CoordinatorEventMap[K]
) => void;

interface WorkerState {
  worker: Worker;
  hashesProcessed: bigint;
  hashRate: number;
  lastNonce?: bigint;
  nextNonce: bigint;
  processedBase: bigint;
  terminated?: boolean;
}

interface StartParams {
  inputs: TxInput[];
  outputs: TxOutput[];
  network: Network;
  satsPerVbyte: number;
  template: WorkerTemplate;
  targetZeros: number;
  startNonce?: bigint;
  signal?: AbortSignal;
  distribution?: bigint[];
}

const safeBigIntToNumber = (value: bigint): number => {
  const max = BigInt(Number.MAX_SAFE_INTEGER);
  if (value > max) return Number.MAX_SAFE_INTEGER;
  if (value < -max) return -Number.MAX_SAFE_INTEGER;
  return Number(value);
};

export class MiningCoordinator {
  private readonly mode: WorkerMode;
  private readonly batchSize: number;
  private readonly workerCount: number;
  private readonly listeners: {
    [K in CoordinatorEvent]: Set<CoordinatorListener<K>>;
  };
  private readonly workers: WorkerState[] = [];
  private readonly readyPromise: Promise<void>;
  private readonly abortHandler: () => void;

  private stride: bigint;
  private cleanupExternalAbort = (): void => {
    if (this.externalAbort) {
      this.externalAbort.removeEventListener("abort", this.abortHandler);
      this.externalAbort = undefined;
    }
  };
  private running = false;
  private paused = false;
  private startedAt: number | null = null;
  private txInputs?: TxInput[];
  private txOutputs?: TxOutput[];
  private txNetwork?: Network;
  private satsPerVbyte?: number;
  private template?: WorkerTemplate;
  private targetZeros?: number;
  private startNonce: bigint = 0n;
  private txDistribution?: bigint[];
  private externalAbort?: AbortSignal;
  private terminated = false;

  constructor(options: MiningCoordinatorOptions) {
    this.mode = options.mode;
    this.batchSize = options.batchSize;
    this.workerCount =
      this.mode === "gpu" ? 1 : Math.max(1, options.workerThreads);
    this.stride =
      this.mode === "gpu"
        ? BigInt(this.batchSize)
        : BigInt(this.batchSize) * BigInt(this.workerCount);

    this.listeners = {
      ready: new Set(),
      progress: new Set(),
      found: new Set(),
      error: new Set(),
      stopped: new Set(),
    };

    this.abortHandler = () => this.stop();
    this.readyPromise = this.spawnWorkers();
  }

  on<K extends CoordinatorEvent>(
    event: K,
    handler: CoordinatorListener<K>
  ): void {
    this.listeners[event].add(handler);
  }

  off<K extends CoordinatorEvent>(
    event: K,
    handler: CoordinatorListener<K>
  ): void {
    this.listeners[event].delete(handler);
  }

  private emit<K extends CoordinatorEvent>(
    event: K,
    payload: CoordinatorEventMap[K]
  ): void {
    this.listeners[event].forEach((handler) => handler(payload));
  }

  private async spawnWorkers(): Promise<void> {
    const readySignals: Promise<void>[] = [];

    for (let i = 0; i < this.workerCount; i += 1) {
      const worker = new Worker(new URL("./worker.ts", import.meta.url), {
        type: "module",
        /* @vite-ignore */ name: `zeldminer-worker-${i}`,
      });

      const state: WorkerState = {
        worker,
        hashesProcessed: 0n,
        hashRate: 0,
        nextNonce: 0n,
        processedBase: 0n,
      };

      this.workers.push(state);

      readySignals.push(
        new Promise<void>((resolve) => {
          const handleReady = (event: MessageEvent<WorkerResponse>): void => {
            if (event.data.type === "ready") {
              worker.removeEventListener("message", handleReady);
              resolve();
            }
          };

          worker.addEventListener("message", handleReady);
        })
      );

      worker.addEventListener("message", (event: MessageEvent<WorkerResponse>) =>
        this.handleWorkerMessage(state, event.data)
      );

      const initMessage: WorkerMessage = { type: "init", mode: this.mode };
      worker.postMessage(initMessage);
    }

    await Promise.all(readySignals);
    this.emit("ready", undefined as unknown as void);
  }

  private handleWorkerMessage(
    state: WorkerState,
    message: WorkerResponse
  ): void {
    switch (message.type) {
      case "ready":
        break;
      case "progress":
        state.hashesProcessed = state.processedBase + message.hashesProcessed;
        state.hashRate = message.hashRate;
        state.lastNonce = message.lastNonce ?? state.lastNonce;
        this.emitProgress();
        break;
      case "batch_complete":
        state.lastNonce = message.lastNonce;
        state.nextNonce = this.computeNextNonce(message.lastNonce);
        break;
      case "found":
        state.hashesProcessed =
          state.processedBase + (message.hashesProcessed ?? state.hashesProcessed);
        state.hashRate = message.hashRate ?? state.hashRate;
        state.lastNonce = message.lastNonce ?? state.lastNonce;

        const attempts = this.workers.reduce(
          (total, worker) => total + worker.hashesProcessed,
          0n
        );
        const duration = this.startedAt ? performance.now() - this.startedAt : 0;
        const hashRate =
          duration > 0
            ? safeBigIntToNumber(attempts) / (duration / 1000)
            : 0;

        const adjustedResult: MineResult = {
          ...message.result,
          attempts,
          duration,
          hashRate,
        };

        this.running = false;
        this.paused = false;
        this.terminateWorkers();
        this.cleanupExternalAbort();
        this.emit("found", adjustedResult);
        break;
      case "error":
        this.running = false;
        this.paused = false;
        this.terminateWorkers();
        this.cleanupExternalAbort();
        this.emit(
          "error",
          new ZeldMinerError(
            message.code ?? ZeldMinerErrorCode.WORKER_ERROR,
            message.message,
            {
              workerId: message.workerId,
              ...(message.details ?? {}),
            }
          )
        );
        break;
    }
  }

  private emitProgress(): void {
    if (!this.running) return;

    const hashesProcessed = this.workers.reduce(
      (total, worker) => total + worker.hashesProcessed,
      0n
    );
    const elapsedMs = this.startedAt ? performance.now() - this.startedAt : 0;
    // Derive rate from total attempts over wall-clock time to avoid overstating
    // throughput on multi-worker CPU runs.
    const hashRate =
      elapsedMs > 0
        ? safeBigIntToNumber(hashesProcessed) / (elapsedMs / 1000)
        : 0;

    const progress: MiningProgressEvent = {
      hashesProcessed,
      hashRate,
      elapsedMs,
    };

    this.emit("progress", progress);
  }

  private computeNextNonce(lastNonce: bigint): bigint {
    const batchAdvance = this.stride - BigInt(this.batchSize);
    return lastNonce + 1n + batchAdvance;
  }

  private stopWorkers(): void {
    if (this.terminated) return;
    this.workers.forEach((state) =>
      state.worker.postMessage({ type: "stop" } satisfies WorkerMessage)
    );
  }

  private terminateWorkers(): void {
    if (this.terminated) return;
    this.stopWorkers();
    this.workers.forEach((state) => {
      try {
        state.worker.terminate();
        state.terminated = true;
      } catch {
        /* ignore termination errors */
      }
    });
    this.terminated = true;
  }

  async start(params: StartParams): Promise<void> {
    await this.readyPromise;
    this.stopWorkers();

    this.running = true;
    this.paused = false;
    this.startedAt = performance.now();
    this.txInputs = params.inputs;
    this.txOutputs = params.outputs;
    this.txNetwork = params.network === "signet" ? "testnet" : params.network;
    this.satsPerVbyte = params.satsPerVbyte;
    this.template = params.template;
    this.targetZeros = params.targetZeros;
    this.startNonce = params.startNonce ?? 0n;
    this.txDistribution = params.distribution;

    if (this.externalAbort && this.externalAbort !== params.signal) {
      this.cleanupExternalAbort();
    }

    if (params.signal) {
      this.externalAbort = params.signal;
      params.signal.addEventListener("abort", this.abortHandler, { once: true });
    }

    if (
      !this.template ||
      !this.txInputs ||
      !this.txOutputs ||
      this.targetZeros === undefined ||
      this.satsPerVbyte === undefined ||
      !this.txNetwork
    ) {
      throw new Error("Mining parameters are missing");
    }

    const inputs = this.txInputs;
    const outputs = this.txOutputs;
    const network = this.txNetwork;
    const satsPerVbyte = this.satsPerVbyte;
    const template = this.template;
    const targetZeros = this.targetZeros;
    const distribution = this.txDistribution;
    const stride = this.stride;

    this.workers.forEach((state, idx) => {
      state.processedBase = 0n;
      state.hashesProcessed = 0n;
      state.hashRate = 0;
      state.lastNonce = undefined;
      const workerStart =
        this.startNonce + BigInt(idx) * BigInt(this.batchSize);
      state.nextNonce = workerStart;

      const message: WorkerMessage = {
        type: "mine",
        inputs,
        outputs,
        network,
        satsPerVbyte,
        template,
        startNonce: workerStart,
        batchSize: this.batchSize,
        targetZeros,
        nonceStep: stride,
        distribution,
      };

      state.worker.postMessage(message);
    });
  }

  pause(): void {
    if (!this.running) return;
    this.running = false;
    this.paused = true;
    this.stopWorkers();
  }

  async resume(): Promise<void> {
    if (
      !this.paused ||
      !this.template ||
      !this.txInputs ||
      !this.txOutputs ||
      this.targetZeros === undefined ||
      this.satsPerVbyte === undefined ||
      !this.txNetwork
    )
      return;
    await this.readyPromise;

    this.running = true;
    this.paused = false;
    if (!this.startedAt) {
      this.startedAt = performance.now();
    }

    const inputs = this.txInputs;
    const outputs = this.txOutputs;
    const network = this.txNetwork;
    const satsPerVbyte = this.satsPerVbyte;
    const template = this.template;
    const targetZeros = this.targetZeros as number;
    const distribution = this.txDistribution;
    const stride = this.stride;

    this.workers.forEach((state, idx) => {
      state.processedBase = state.hashesProcessed;
      state.hashRate = 0;
      const workerStart =
        state.nextNonce ||
        this.startNonce + BigInt(idx) * BigInt(this.batchSize);

      const message: WorkerMessage = {
        type: "mine",
        inputs,
        outputs,
        network,
        satsPerVbyte,
        template,
        startNonce: workerStart,
        batchSize: this.batchSize,
        targetZeros,
        nonceStep: stride,
        distribution,
      };

      state.worker.postMessage(message);
    });
  }

  stop(): void {
    if (!this.running && !this.paused) return;
    this.running = false;
    this.paused = false;
    this.terminateWorkers();
    this.cleanupExternalAbort();
    this.emit("stopped", undefined as unknown as void);
  }
}

