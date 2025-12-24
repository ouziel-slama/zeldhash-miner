import { beforeEach, describe, expect, it, vi, afterEach } from "vitest";
import type { MineResult, TxInput, TxOutput } from "./types";
import { ZeldMiner } from "./index";
import { MAINNET_REAL_CASE } from "./__fixtures__/mainnetRealCase";

const buildMiningTemplate = vi.fn<
  [
    {
      inputs: TxInput[];
      outputs: TxOutput[];
      startNonce: bigint;
      batchSize: number;
    }
  ],
  Promise<{ prefix: Uint8Array; suffix: Uint8Array; nonceLength: number }>
>();

const buildPsbt = vi.fn<
  [{ inputs: TxInput[]; outputs: TxOutput[]; nonce: bigint }],
  Promise<string>
>();

const coordinatorStart = vi.fn<
  [
    {
      inputs: TxInput[];
      outputs: TxOutput[];
      network: string;
      satsPerVbyte: number;
      template: { prefix: Uint8Array; suffix: Uint8Array; nonceLength: number };
      targetZeros: number;
      startNonce?: bigint;
    }
  ],
  Promise<void>
>();

let foundResult: MineResult | null = null;

vi.mock("./builder", () => {
  return {
    TransactionBuilder: class {
      constructor(public readonly network: string, public readonly satsPerVbyte: number) {}
      buildMiningTemplate = buildMiningTemplate;
      buildPsbt = buildPsbt;
    },
  };
});

vi.mock("./coordinator", () => {
  return {
    MiningCoordinator: class {
      private readonly listeners: Record<string, Set<(payload?: any) => void>> = {
        progress: new Set(),
        found: new Set(),
        error: new Set(),
        stopped: new Set(),
      };

      constructor(public readonly options: { mode: string; batchSize: number; workerThreads: number }) {}

      on(event: string, handler: (payload: any) => void): void {
        this.listeners[event]?.add(handler);
      }

      off(event: string, handler: (payload: any) => void): void {
        this.listeners[event]?.delete(handler);
      }

      async start(params: Parameters<typeof coordinatorStart>[0]): Promise<void> {
        await coordinatorStart(params);
        this.listeners.progress.forEach((cb) =>
          cb({
            hashesProcessed: 0n,
            hashRate: 0,
            elapsedMs: 0,
          })
        );

        const baseResult: MineResult = {
          psbt: "",
          txid: "txid",
          nonce: params.startNonce ?? 0n,
          attempts: 1n,
          duration: 1,
          hashRate: 1,
        };

        const result: MineResult = foundResult ? { ...foundResult } : baseResult;
        this.listeners.found.forEach((cb) => cb(result));
      }

      stop(): void {
        this.listeners.stopped.forEach((cb) => cb());
      }

      pause(): void {}
      resume(): Promise<void> {
        return Promise.resolve();
      }
    },
  };
});

describe("ZeldMiner nonce segmentation", () => {
  const inputs: TxInput[] = [
    { txid: "0".repeat(64), vout: 0, scriptPubKey: "00", amount: 10_000 },
  ];
  const outputs: TxOutput[] = [
    { address: "change", change: true },
    { address: "dest", amount: 1_000, change: false },
  ];

  beforeEach(() => {
    vi.clearAllMocks();
    buildMiningTemplate.mockResolvedValue({
      prefix: new Uint8Array([1]),
      suffix: new Uint8Array([2]),
      nonceLength: 1,
    });
    buildPsbt.mockResolvedValue("psbt");
    foundResult = null;
    vi.unstubAllGlobals();
  });

  afterEach(() => {
    foundResult = null;
    vi.unstubAllGlobals();
  });

  it("builds the initial template using the first nonce segment size", async () => {
    const miner = new ZeldMiner({
      network: "testnet",
      batchSize: 70_000,
      useWebGPU: false,
      workerThreads: 2,
      satsPerVbyte: 1,
    });

    const result = await miner.mineTransaction({
      inputs,
      outputs,
      targetZeros: 1,
      startNonce: 0n,
      batchSize: 70_000,
    });

    expect(buildMiningTemplate).toHaveBeenCalledWith(
      expect.objectContaining({
        startNonce: 0n,
        // First segment: 0x00..0xff inclusive
        batchSize: 256,
      })
    );

    expect(coordinatorStart).toHaveBeenCalled();
    expect(result.nonce).toBe(0n);
    expect(result.psbt).toBe("psbt");
  });

  const runRealCase = async (useWebGPU: boolean, gpuAvailable: boolean) => {
    const requestAdapter = vi.fn().mockResolvedValue(gpuAvailable ? {} : null);
    vi.stubGlobal("navigator", { gpu: { requestAdapter } });

    buildMiningTemplate.mockResolvedValue({
      prefix: new Uint8Array([1]),
      suffix: new Uint8Array([2]),
      nonceLength: 3,
    });
    buildPsbt.mockResolvedValue(MAINNET_REAL_CASE.expectedPsbt);
    foundResult = {
      psbt: "",
      txid: MAINNET_REAL_CASE.expectedTxid,
      nonce: MAINNET_REAL_CASE.nonce,
      attempts: 1n,
      duration: 1,
      hashRate: 1,
    };

    const miner = new ZeldMiner({
      network: MAINNET_REAL_CASE.network,
      batchSize: 10_000,
      useWebGPU,
      workerThreads: 2,
      satsPerVbyte: MAINNET_REAL_CASE.satsPerVbyte,
    });

    const result = await miner.mineTransaction({
      inputs: MAINNET_REAL_CASE.inputs,
      outputs: MAINNET_REAL_CASE.outputs,
      targetZeros: MAINNET_REAL_CASE.targetZeros,
      startNonce: MAINNET_REAL_CASE.nonce,
      batchSize: 10_000,
    });

    expect(result.txid).toBe(MAINNET_REAL_CASE.expectedTxid);
    expect(result.psbt).toBe(MAINNET_REAL_CASE.expectedPsbt);
    expect(result.nonce).toBe(MAINNET_REAL_CASE.nonce);

    expect(buildPsbt).toHaveBeenCalledWith({
      inputs: MAINNET_REAL_CASE.inputs,
      outputs: MAINNET_REAL_CASE.outputs,
      nonce: MAINNET_REAL_CASE.nonce,
    });
    expect(buildMiningTemplate).toHaveBeenCalledWith(
      expect.objectContaining({
        inputs: MAINNET_REAL_CASE.inputs,
        outputs: MAINNET_REAL_CASE.outputs,
        startNonce: MAINNET_REAL_CASE.nonce,
        batchSize: 10_000,
      })
    );
    expect(coordinatorStart).toHaveBeenCalledWith(
      expect.objectContaining({
        network: MAINNET_REAL_CASE.network,
        targetZeros: MAINNET_REAL_CASE.targetZeros,
      })
    );
  };

  it("processes the real mainnet vector on CPU fallback", async () => {
    await runRealCase(false, false);
  });

  it("processes the real mainnet vector when GPU is available", async () => {
    await runRealCase(true, true);
  });
});

