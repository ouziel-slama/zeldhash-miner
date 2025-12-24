import type {
  MineResult,
  ValidationResult,
  WasmExports,
  WorkerMessage,
  WorkerMode,
  WorkerTemplate,
  WorkerResponse,
} from "./types";
import { ZeldMinerErrorCode } from "./types";
import { splitNonceSegments, splitNonceSegmentsCbor } from "./nonce";
import { loadWasm } from "./wasm";

type MineMessage = Extract<WorkerMessage, { type: "mine" }>;

const ctx = self as unknown as DedicatedWorkerGlobalScope;
const workerId =
  (ctx as DedicatedWorkerGlobalScope & { name?: string }).name ?? undefined;

const formatError = (err: unknown): string =>
  err instanceof Error ? err.message : String(err);

let currentMode: WorkerMode = "cpu";
let miningAbort: AbortController | null = null;
let miningPromise: Promise<void> | null = null;

const post = (message: WorkerResponse): void => {
  ctx.postMessage({ ...message, workerId });
};

const postError = (
  message: string,
  code: ZeldMinerErrorCode = ZeldMinerErrorCode.WORKER_ERROR,
  details?: Record<string, unknown>
): void => {
  post({ type: "error", message, code, details });
};

const isValidationResult = (val: unknown): val is ValidationResult =>
  typeof val === "object" && val !== null && "ok" in (val as object);

const isMineResult = (
  val: unknown
): val is { nonce: bigint; txid: string } =>
  typeof val === "object" &&
  val !== null &&
  "nonce" in (val as object) &&
  "txid" in (val as object);

const safeBigIntToNumber = (value: bigint): number => {
  const max = BigInt(Number.MAX_SAFE_INTEGER);
  if (value > max) return Number.MAX_SAFE_INTEGER;
  if (value < -max) return -Number.MAX_SAFE_INTEGER;
  return Number(value);
};

type TemplateCache = Map<number, WorkerTemplate>;

const cloneTemplate = (template: WorkerTemplate): WorkerTemplate => ({
  nonceLength: template.nonceLength,
  prefix: new Uint8Array(template.prefix),
  suffix: new Uint8Array(template.suffix),
  useCborNonce: template.useCborNonce,
});

const cacheTemplate = (cache: TemplateCache, template: WorkerTemplate): void => {
  cache.set(template.nonceLength, cloneTemplate(template));
};

const ensureTemplateForSegment = async (
  cache: TemplateCache,
  wasm: WasmExports,
  params: {
    inputs: MineMessage["inputs"];
    outputs: MineMessage["outputs"];
    satsPerVbyte: number;
    distribution?: bigint[];
    useCborNonce: boolean;
  },
  segment: { start: bigint; size: number; nonceLength: number },
  normalizedNetwork: MineMessage["network"]
): Promise<WorkerTemplate> => {
  const cached = cache.get(segment.nonceLength);
  if (cached) {
    return cached;
  }

  const template = wasm.build_mining_template(
    params.inputs,
    params.outputs,
    normalizedNetwork,
    BigInt(params.satsPerVbyte),
    segment.start,
    segment.size,
    params.distribution ?? null
  );

  const built: WorkerTemplate = {
    ...template,
    nonceLength: segment.nonceLength,
    useCborNonce: template.useCborNonce ?? params.useCborNonce,
  };

  cacheTemplate(cache, built);
  return built;
};

const runBatch = async (
  wasm: WasmExports,
  mode: WorkerMode,
  prefix: Uint8Array,
  suffix: Uint8Array,
  startNonce: bigint,
  batchSize: number,
  targetZeros: number,
  useCborNonce: boolean
): Promise<unknown> => {
  if (mode === "gpu") {
    if (!wasm.mine_batch_gpu) {
      throw new Error("GPU mining requested but mine_batch_gpu is unavailable");
    }
    return wasm.mine_batch_gpu(
      prefix,
      suffix,
      startNonce,
      batchSize,
      targetZeros,
      useCborNonce
    );
  }

  return wasm.mine_batch_wasm(
    prefix,
    suffix,
    startNonce,
    batchSize,
    targetZeros,
    useCborNonce
  );
};

const mineLoop = async (msg: MineMessage, abort: AbortController): Promise<void> => {
  let wasm: WasmExports;
  try {
    wasm = await loadWasm();
    if (currentMode === "gpu") {
      if (!wasm.mine_batch_gpu) {
        postError(
          "GPU mining requested but mine_batch_gpu is unavailable",
          ZeldMinerErrorCode.WEBGPU_NOT_AVAILABLE
        );
        return;
      }
      if (wasm.init_gpu) {
        await wasm.init_gpu();
      }
    }
  } catch (err) {
    const message = formatError(err);
    const code =
      currentMode === "gpu"
        ? ZeldMinerErrorCode.WEBGPU_NOT_AVAILABLE
        : ZeldMinerErrorCode.WORKER_ERROR;
    postError(`Failed to initialize WASM: ${message}`, code);
    return;
  }

  const templateCache: TemplateCache = new Map();
  const useCborNonce =
    msg.template.useCborNonce ?? Boolean(msg.distribution && msg.distribution.length > 0);
  cacheTemplate(templateCache, { ...msg.template, useCborNonce });

  const stride = msg.nonceStep ?? BigInt(msg.batchSize);
  const normalizedNetwork = msg.network === "signet" ? "testnet" : msg.network;
  let nextNonce = msg.startNonce;
  let hashesProcessed = 0n;
  const startedAt = performance.now();

  while (!abort.signal.aborted) {
    const iterationStart = nextNonce;
    let remaining = msg.batchSize;
    let processedInIteration = 0n;

    while (remaining > 0 && !abort.signal.aborted) {
      const segmentStart = iterationStart + processedInIteration;
      let segment: { start: bigint; size: number; nonceLength: number };

      try {
        const segments = useCborNonce
          ? splitNonceSegmentsCbor(segmentStart, remaining)
          : splitNonceSegments(segmentStart, remaining);
        segment = segments[0];
      } catch (err) {
        postError(
          `Invalid nonce range: ${formatError(err)}`,
          ZeldMinerErrorCode.INVALID_INPUT
        );
        abort.abort();
        return;
      }

      let template: WorkerTemplate;
      try {
        template = await ensureTemplateForSegment(
          templateCache,
          wasm,
          {
            inputs: msg.inputs,
            outputs: msg.outputs,
            satsPerVbyte: msg.satsPerVbyte,
            distribution: msg.distribution,
            useCborNonce,
          },
          segment,
          normalizedNetwork
        );
      } catch (err) {
        postError(
          `Failed to build mining template: ${formatError(err)}`,
          ZeldMinerErrorCode.WORKER_ERROR
        );
        abort.abort();
        return;
      }

      let output: unknown;
      const batchStartedAt = performance.now();
      try {
        output = await runBatch(
          wasm,
          currentMode,
          template.prefix,
          template.suffix,
          segment.start,
          segment.size,
          msg.targetZeros,
          useCborNonce
        );
      } catch (err) {
        const message = formatError(err);
        postError(
          `Batch mining failed: ${message}`,
          ZeldMinerErrorCode.WORKER_ERROR
        );
        abort.abort();
        return;
      }

      const batchDurationMs = performance.now() - batchStartedAt;

      if (isValidationResult(output)) {
        if (!output.ok) {
          postError(
            output.error ?? "Validation failed",
            ZeldMinerErrorCode.INVALID_INPUT
          );
          abort.abort();
          return;
        }
      } else if (isMineResult(output)) {
        const attemptsBefore = hashesProcessed + processedInIteration;
        const attemptsInSegment = BigInt(output.nonce) - segment.start + 1n;
        const totalAttempts = attemptsBefore + attemptsInSegment;
        const elapsedMs = performance.now() - startedAt;
        const hashRate =
          elapsedMs > 0
            ? safeBigIntToNumber(totalAttempts) / (elapsedMs / 1000)
            : 0;
        const lastNonce = segment.start + attemptsInSegment - 1n;

        const result: MineResult = {
          psbt: "",
          txid: output.txid,
          nonce: BigInt(output.nonce),
          attempts: totalAttempts,
          duration: elapsedMs,
          hashRate,
        };

        post({
          type: "found",
          result,
          hashesProcessed: totalAttempts,
          hashRate,
          lastNonce,
        });
        abort.abort();
        return;
      }

      hashesProcessed += BigInt(segment.size);
      processedInIteration += BigInt(segment.size);
      remaining -= segment.size;

      const hashRate =
        batchDurationMs > 0
          ? segment.size / (batchDurationMs / 1000)
          : segment.size;
      const lastNonce = segment.start + BigInt(segment.size) - 1n;

      post({ type: "progress", hashesProcessed, hashRate, lastNonce });
    }

    if (abort.signal.aborted) {
      break;
    }

    const lastNonce = iterationStart + BigInt(msg.batchSize) - 1n;
    post({ type: "batch_complete", lastNonce });

    nextNonce = iterationStart + stride;
  }
};

const startMining = (msg: MineMessage): void => {
  const abort = new AbortController();
  miningAbort?.abort();
  miningAbort = abort;

  const promise = mineLoop(msg, abort).finally(() => {
    if (miningAbort === abort) {
      miningAbort = null;
    }
    if (miningPromise === promise) {
      miningPromise = null;
    }
  });

  miningPromise = promise;
};

ctx.addEventListener("message", (event: MessageEvent<WorkerMessage>) => {
  const data = event.data;

  switch (data.type) {
    case "init":
      currentMode = data.mode;
      post({ type: "ready" });
      break;
    case "mine":
      startMining(data);
      break;
    case "stop":
      miningAbort?.abort();
      break;
    default:
      postError(
        `Unknown message type: ${(data as { type?: string }).type}`,
        ZeldMinerErrorCode.WORKER_ERROR
      );
  }
});

