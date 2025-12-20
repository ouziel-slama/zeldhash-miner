import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { MiningCoordinator } from "./coordinator";
import type {
  MineResult,
  TxInput,
  TxOutput,
  WorkerMessage,
  WorkerResponse,
} from "./types";
import { ZeldMinerErrorCode } from "./types";
import { ZeldMinerError } from "./errors";

type Listener = (event: MessageEvent<WorkerResponse>) => void;

let workerBehavior: "found" | "error" = "found";
const createdWorkers: FakeWorker[] = [];
let foundDispatched = false;
const template = {
  prefix: new Uint8Array([1, 2]),
  suffix: new Uint8Array([3, 4]),
  nonceLength: 1,
};
const inputs: TxInput[] = [
  { txid: "0".repeat(64), vout: 0, scriptPubKey: "00", amount: 1 },
];
const outputs: TxOutput[] = [
  { address: "change", change: true },
  { address: "dest", amount: 1, change: false },
];

class FakeWorker {
  listeners: Set<Listener> = new Set();
  stopped = false;
  terminated = false;

  constructor(_: URL, __: { name?: string }) {
    createdWorkers.push(this);
  }

  addEventListener(event: string, listener: Listener): void {
    if (event === "message") {
      this.listeners.add(listener);
    }
  }

  removeEventListener(_: string, listener: Listener): void {
    this.listeners.delete(listener);
  }

  postMessage(msg: WorkerMessage): void {
    if (msg.type === "init") {
      queueMicrotask(() => this.dispatch({ type: "ready" }));
      return;
    }

    if (msg.type === "mine") {
      queueMicrotask(() => {
        if (workerBehavior === "error") {
          this.dispatch({
            type: "error",
            message: "simulated worker error",
            code: ZeldMinerErrorCode.WORKER_ERROR,
          });
          return;
        }

        const lastNonce = msg.startNonce + BigInt(msg.batchSize) - 1n;
        this.dispatch({
          type: "progress",
          hashesProcessed: BigInt(msg.batchSize),
          hashRate: 1000,
          lastNonce,
        });
        this.dispatch({ type: "batch_complete", lastNonce });
        if (!foundDispatched) {
          foundDispatched = true;
          this.dispatch({
            type: "found",
            result: {
              psbt: "",
              txid: "00".repeat(32),
              nonce: msg.startNonce,
              attempts: BigInt(msg.batchSize),
              duration: 1,
              hashRate: 1000,
            },
            hashesProcessed: BigInt(msg.batchSize),
            hashRate: 1000,
            lastNonce,
          });
        }
      });
      return;
    }

    if (msg.type === "stop") {
      this.stopped = true;
    }
  }

  terminate(): void {
    this.terminated = true;
  }

  private dispatch(data: WorkerResponse): void {
    const event = { data } as MessageEvent<WorkerResponse>;
    this.listeners.forEach((listener) => listener(event));
  }
}

describe("MiningCoordinator worker protocol", () => {
  beforeEach(() => {
    workerBehavior = "found";
    foundDispatched = false;
    createdWorkers.length = 0;
    vi.stubGlobal("Worker", FakeWorker as unknown as typeof Worker);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("emits progress and found results, then stops workers", async () => {
    const coordinator = new MiningCoordinator({
      mode: "cpu",
      batchSize: 2,
      workerThreads: 2,
    });

    const results: MineResult[] = [];
    const progressEvents: number[] = [];
    const foundPromise = new Promise<void>((resolve) => {
      coordinator.on("found", (result) => {
        results.push(result);
        expect(result.txid).toBeDefined();
        resolve();
      });
    });
    coordinator.on("progress", (evt) => progressEvents.push(Number(evt.hashesProcessed)));

    await coordinator.start({
      inputs,
      outputs,
      network: "testnet",
      satsPerVbyte: 1,
      template,
      targetZeros: 2,
      startNonce: 0n,
    });
    await foundPromise;

    expect(progressEvents.length).toBeGreaterThan(0);
    // In the FakeWorker, Worker 0's microtask completes (progress → found) before
    // Worker 1's microtask runs, so only Worker 0's hashes are counted at termination.
    expect(results[0]?.attempts).toBe(2n);
    expect(createdWorkers.every((w) => w.stopped)).toBe(true);
    expect(createdWorkers.every((w) => w.terminated)).toBe(true);
  });

  it("propagates worker errors as ZeldMinerError", async () => {
    workerBehavior = "error";
    const coordinator = new MiningCoordinator({
      mode: "cpu",
      batchSize: 1,
      workerThreads: 1,
    });

    const errorPromise = new Promise<ZeldMinerError>((resolve) => {
      coordinator.on("error", (err) => resolve(err));
    });

    await coordinator.start({
      inputs,
      outputs,
      network: "testnet",
      satsPerVbyte: 1,
      template,
      targetZeros: 2,
      startNonce: 0n,
    });

    const error = await errorPromise;
    expect(error).toBeInstanceOf(ZeldMinerError);
    expect(error.code).toBe(ZeldMinerErrorCode.WORKER_ERROR);
    expect(createdWorkers.every((w) => w.terminated)).toBe(true);
  });
});

