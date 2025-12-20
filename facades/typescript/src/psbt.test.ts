import { beforeEach, describe, expect, it, vi } from "vitest";
import { networks, payments, Psbt } from "bitcoinjs-lib";
import { Buffer } from "buffer";
import { TransactionBuilder } from "./builder";
import { MAINNET_REAL_CASE } from "./__fixtures__/mainnetRealCase";

const mockWasm = {
  validate_address: vi.fn(() => ({ ok: true, network: "testnet" })),
  build_mining_template: vi.fn(() => ({
    prefix: new Uint8Array([1]),
    suffix: new Uint8Array([2]),
  })),
  build_psbt: vi.fn(() => ""),
};

vi.mock("./wasm", () => ({
  loadWasm: vi.fn(() => Promise.resolve(mockWasm)),
}));

const baseInput = {
  txid: "a".repeat(64),
  vout: 0,
  scriptPubKey: "",
  amount: 10_000,
};

describe("PSBT parsing", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("returns a PSBT parseable by bitcoinjs-lib", async () => {
    const changePayment = payments.p2wpkh({
      pubkey: Buffer.alloc(33, 2),
      network: networks.testnet,
    });
    const destPayment = payments.p2wpkh({
      pubkey: Buffer.alloc(33, 3),
      network: networks.testnet,
    });

    const psbtBuilder = new Psbt({ network: networks.testnet });
    const inputScript = changePayment.output as Buffer;
    const changeAddress = changePayment.address as string;
    const destAddress = destPayment.address as string;

    psbtBuilder.addInput({
      hash: Buffer.alloc(32).toString("hex"),
      index: 0,
      witnessUtxo: { script: inputScript, value: baseInput.amount },
    });
    psbtBuilder.addOutput({ address: destAddress, value: 8_000 });
    psbtBuilder.addOutput({ address: changeAddress, value: 1_000 });

    const base64Psbt = psbtBuilder.toBase64();
    mockWasm.build_psbt.mockReturnValueOnce(base64Psbt);

    const builder = new TransactionBuilder("testnet", 1);
    const result = await builder.buildPsbt({
      inputs: [
        {
          ...baseInput,
          scriptPubKey: inputScript.toString("hex"),
        },
      ],
      outputs: [
        { address: changeAddress, change: true },
        { address: destAddress, amount: 1_000, change: false },
      ],
      nonce: 0n,
    });

    const parsed = Psbt.fromBase64(result);
    expect(parsed.txInputs).toHaveLength(1);
    expect(parsed.txOutputs).toHaveLength(2);
    expect(mockWasm.build_psbt).toHaveBeenCalled();
  });

  it("keeps the real mainnet PSBT fixture parseable and consistent", () => {
    const psbt = Psbt.fromBase64(MAINNET_REAL_CASE.expectedPsbt, {
      network: networks.bitcoin,
    });

    const expectedLittleEndian = Buffer.from(MAINNET_REAL_CASE.inputs[0].txid, "hex")
      .reverse()
      .toString("hex");
    expect(psbt.txInputs[0]?.hash.toString("hex")).toBe(expectedLittleEndian);

    const changeOutput = psbt.txOutputs.find(
      (out) => out.address === MAINNET_REAL_CASE.outputs[0].address
    );
    expect(changeOutput?.value).toBeGreaterThan(0);

    const opReturn = psbt.txOutputs.find((out) => !out.address);
    expect(opReturn?.script?.slice(2).toString("hex")).toBe(MAINNET_REAL_CASE.opReturnHex);

    const nonceBytes = opReturn?.script?.slice(2) ?? Buffer.alloc(0);
    const nonceLe = nonceBytes.readUIntLE(0, nonceBytes.length);
    expect(BigInt(nonceLe)).toBe(MAINNET_REAL_CASE.nonce);

    const txid = psbt.__CACHE?.__TX?.getId?.();
    expect(txid).toBe(MAINNET_REAL_CASE.expectedTxid);
  });
});

