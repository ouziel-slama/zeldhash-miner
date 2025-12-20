import { beforeEach, describe, expect, it, vi } from "vitest";
import { TransactionBuilder } from "./builder";
import { ZeldMinerError } from "./errors";
import { ZeldMinerErrorCode } from "./types";

const mockTemplate = {
  prefix: new Uint8Array([1]),
  suffix: new Uint8Array([2]),
};

const mockWasm = {
  validate_address: vi.fn(() => ({ ok: true, network: "testnet" })),
  build_mining_template: vi.fn(() => mockTemplate),
  build_psbt: vi.fn(() => "psbt"),
};

vi.mock("./wasm", () => ({
  loadWasm: vi.fn(() => Promise.resolve(mockWasm)),
}));

const baseInputs = [
  {
    txid: "a".repeat(64),
    vout: 0,
    scriptPubKey: "ab".repeat(10),
    amount: 10_000,
  },
];

const validOutputs = [
  { address: "tb1qchange", change: true },
  { address: "tb1qrecipient", amount: 1_000, change: false },
];

describe("TransactionBuilder validation", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("rejects nonce ranges that cross a byte-length boundary", async () => {
    const builder = new TransactionBuilder("testnet", 1);

    const promise = builder.buildMiningTemplate({
      inputs: baseInputs,
      outputs: validOutputs,
      startNonce: 255n,
      batchSize: 2,
    });

    await expect(promise).rejects.toBeInstanceOf(ZeldMinerError);
    await expect(promise).rejects.toMatchObject({
      code: ZeldMinerErrorCode.INVALID_INPUT,
    });

    expect(mockWasm.validate_address).toHaveBeenCalled();
    expect(mockWasm.build_mining_template).not.toHaveBeenCalled();
  });

  it("requires change outputs to use non-negative amounts when provided", async () => {
    const builder = new TransactionBuilder("testnet", 1);
    const outputs = [
      { address: "tb1qchange", amount: -1, change: true },
      { address: "tb1qrecipient", amount: 1_000, change: false },
    ];

    const promise = builder.buildMiningTemplate({
      inputs: baseInputs,
      outputs,
      startNonce: 0n,
      batchSize: 1,
    });

    await expect(promise).rejects.toBeInstanceOf(ZeldMinerError);
    await expect(promise).rejects.toMatchObject({
      code: ZeldMinerErrorCode.INVALID_INPUT,
    });

    expect(mockWasm.validate_address).toHaveBeenCalled();
    expect(mockWasm.build_mining_template).not.toHaveBeenCalled();
  });

  it("propagates address validation failures with INVALID_ADDRESS code", async () => {
    mockWasm.validate_address.mockReturnValueOnce({
      ok: false,
      error: "invalid bech32 checksum",
    });

    const builder = new TransactionBuilder("testnet", 1);

    await expect(
      builder.buildMiningTemplate({
        inputs: baseInputs,
        outputs: validOutputs,
        startNonce: 0n,
        batchSize: 1,
      })
    ).rejects.toMatchObject({
      code: ZeldMinerErrorCode.INVALID_ADDRESS,
    });
  });

  it("maps wasm fee errors to INSUFFICIENT_FUNDS", async () => {
    mockWasm.build_mining_template.mockImplementationOnce(() => {
      throw new Error("insufficient funds for outputs and fee");
    });

    const builder = new TransactionBuilder("testnet", 1);

    await expect(
      builder.buildMiningTemplate({
        inputs: baseInputs,
        outputs: validOutputs,
        startNonce: 0n,
        batchSize: 1,
      })
    ).rejects.toMatchObject({
      code: ZeldMinerErrorCode.INSUFFICIENT_FUNDS,
    });
  });

  it("maps wasm fee errors to DUST_OUTPUT when change would be dust", async () => {
    mockWasm.build_mining_template.mockImplementationOnce(() => {
      throw new Error("change would be dust");
    });

    const builder = new TransactionBuilder("testnet", 1);

    await expect(
      builder.buildMiningTemplate({
        inputs: baseInputs,
        outputs: validOutputs,
        startNonce: 0n,
        batchSize: 1,
      })
    ).rejects.toMatchObject({
      code: ZeldMinerErrorCode.DUST_OUTPUT,
    });
  });

  it("passes satsPerVbyte and minimal nonce length for fee sizing", async () => {
    const builder = new TransactionBuilder("testnet", 3);
    const startNonce = 0x1ffn; // stays within 2-byte length for this batch
    const batchSize = 10;

    let capturedSats: bigint | undefined;
    let capturedStart: bigint | undefined;
    let capturedBatch: number | undefined;

    mockWasm.build_mining_template.mockImplementationOnce(
      (_inputs, _outputs, _network, satsPerVbyte, startNonceArg, batchSizeArg) => {
        capturedSats = satsPerVbyte;
        capturedStart = startNonceArg;
        capturedBatch = batchSizeArg;
        return mockTemplate;
      }
    );

    const template = await builder.buildMiningTemplate({
      inputs: baseInputs,
      outputs: validOutputs,
      startNonce,
      batchSize,
    });

    expect(capturedSats).toBe(3n);
    expect(capturedStart).toBe(startNonce);
    expect(capturedBatch).toBe(batchSize);
    expect(template.nonceLength).toBe(2);
  });
});

