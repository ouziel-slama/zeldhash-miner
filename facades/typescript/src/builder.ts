import type {
  MiningTemplate,
  Network,
  TxInput,
  TxOutput,
  ValidationResult,
  WasmExports,
} from "./types";
import { ZeldMinerErrorCode } from "./types";
import { createMinerError, toZeldMinerError } from "./errors";
import { loadWasm } from "./wasm";
import { cborNonceLength, nonceLength } from "./nonce";

const TXID_REGEX = /^[0-9a-fA-F]{64}$/;
const HEX_REGEX = /^[0-9a-fA-F]+$/;
const MAX_U64 = (1n << 64n) - 1n;
const MAX_U32 = 0xffff_ffff;

const dustLimitForAddress = (addressType?: "p2wpkh" | "p2tr"): number => {
  if (addressType === "p2tr") return 330;
  if (addressType === "p2wpkh") return 310;
  return 546; // conservative fallback for unexpected types
};

const formatError = (err: unknown): string =>
  err instanceof Error ? err.message : String(err);

const includesAny = (haystack: string, needles: string[]): boolean =>
  needles.some((needle) => haystack.includes(needle));

const mapWasmError = (
  err: unknown,
  context: string,
  details?: Record<string, unknown>
): never => {
  const message = formatError(err);
  const normalized = message.toLowerCase();

  if (includesAny(normalized, ["insufficient funds", "insufficient_funds"])) {
    throw createMinerError(
      ZeldMinerErrorCode.INSUFFICIENT_FUNDS,
      "Insufficient funds for outputs and fee",
      { ...details, cause: message }
    );
  }

  if (includesAny(normalized, ["change would be dust", "output amount below dust limit", "dust"])) {
    throw createMinerError(
      ZeldMinerErrorCode.DUST_OUTPUT,
      "Change would be dust",
      { ...details, cause: message }
    );
  }

  throw toZeldMinerError(err, ZeldMinerErrorCode.WORKER_ERROR, {
    ...details,
    context,
  });
};

const normalizeNetwork = (network: Network): Network =>
  network === "signet" ? "testnet" : network;

export class TransactionBuilder {
  private readonly network: Network;
  private readonly satsPerVbyte: number;

  constructor(network: Network, satsPerVbyte: number) {
    if (!Number.isFinite(satsPerVbyte) || satsPerVbyte <= 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "satsPerVbyte must be a positive number",
        { field: "satsPerVbyte" }
      );
    }
    this.network = network;
    this.satsPerVbyte = satsPerVbyte;
  }

  private async getWasm(): Promise<WasmExports> {
    return loadWasm();
  }

  private assertNonceRange(startNonce: bigint, batchSize: number, useCborNonce: boolean): number {
    if (startNonce < 0n || startNonce > MAX_U64) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "startNonce must be between 0 and 2^64 - 1",
        { startNonce }
      );
    }
    if (!Number.isInteger(batchSize) || batchSize <= 0 || batchSize > MAX_U32) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "batchSize must be a positive 32-bit integer",
        { batchSize }
      );
    }

    const lastNonce = startNonce + BigInt(batchSize - 1);
    if (lastNonce > MAX_U64) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "nonce range exceeds u64",
        { startNonce, batchSize }
      );
    }

    const lengthFn = useCborNonce ? cborNonceLength : nonceLength;
    const startLen = lengthFn(startNonce);
    const lastLen = lengthFn(lastNonce);
    if (startLen !== lastLen) {
      const boundaryLabel = useCborNonce ? "CBOR length" : "byte-length";
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        `nonce range crosses ${boundaryLabel} boundary; reduce batch size`,
        { startNonce, batchSize }
      );
    }

    return startLen;
  }

  private cloneInputs(inputs: TxInput[]): TxInput[] {
    return inputs.map((input) => ({ ...input }));
  }

  private cloneOutputs(outputs: TxOutput[]): TxOutput[] {
    return outputs.map((output) => ({ ...output }));
  }

  private validateInputs(inputs: TxInput[]): void {
    if (!Array.isArray(inputs) || inputs.length === 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "At least one input is required"
      );
    }

    inputs.forEach((input, idx) => {
      if (!TXID_REGEX.test(input.txid)) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          `inputs[${idx}].txid must be a 64-character hex`,
          { index: idx }
        );
      }
      if (!Number.isInteger(input.vout) || input.vout < 0) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          `inputs[${idx}].vout must be a non-negative integer`,
          { index: idx }
        );
      }
      if (!Number.isInteger(input.amount) || input.amount <= 0) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          `inputs[${idx}].amount must be a positive integer`,
          { index: idx }
        );
      }
      if (
        typeof input.scriptPubKey !== "string" ||
        input.scriptPubKey.length === 0 ||
        input.scriptPubKey.length % 2 !== 0 ||
        !HEX_REGEX.test(input.scriptPubKey)
      ) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          `inputs[${idx}].scriptPubKey must be valid hex`,
          { index: idx }
        );
      }
    });
  }

  private validateAddressResult(
    validation: ValidationResult,
    idx: number,
    network: Network
  ): void {
    if (!validation.ok) {
      const reason = validation.error ?? "invalid address";
      const normalized = reason.toLowerCase();
      const code = normalized.includes("unsupported")
        ? ZeldMinerErrorCode.UNSUPPORTED_ADDRESS_TYPE
        : ZeldMinerErrorCode.INVALID_ADDRESS;

      throw createMinerError(
        code,
        `outputs[${idx}].address is invalid (${reason})`,
        { index: idx }
      );
    }

    if (
      validation.network &&
      validation.network !== network &&
      !(validation.network === "testnet" && network === "signet")
    ) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_ADDRESS,
        `outputs[${idx}].address network mismatch`,
        { index: idx }
      );
    }

    if (
      validation.addressType &&
      validation.addressType !== "p2tr" &&
      validation.addressType !== "p2wpkh"
    ) {
      throw createMinerError(
        ZeldMinerErrorCode.UNSUPPORTED_ADDRESS_TYPE,
        `outputs[${idx}].address uses an unsupported type`,
        { index: idx, addressType: validation.addressType }
      );
    }
  }

  private async validateOutputs(outputs: TxOutput[]): Promise<void> {
    if (!Array.isArray(outputs) || outputs.length === 0) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "At least one output is required"
      );
    }

    const changeCount = outputs.filter((o) => o.change).length;
    if (changeCount > 1) {
      throw createMinerError(
        ZeldMinerErrorCode.MULTIPLE_CHANGE_OUTPUTS,
        "At most one change output is allowed",
        { changeCount }
      );
    }

    const wasm = await this.getWasm();
    const net = normalizeNetwork(this.network);

    outputs.forEach((output, idx) => {
      const validation = wasm.validate_address(output.address, net);
      this.validateAddressResult(validation, idx, net);
      const dustLimit = dustLimitForAddress(validation.addressType);

      if (output.change) {
        if (
          output.amount !== undefined &&
          (!Number.isInteger(output.amount) || output.amount < 0)
        ) {
          throw createMinerError(
            ZeldMinerErrorCode.INVALID_INPUT,
            `outputs[${idx}].amount must be a non-negative integer when provided`,
            { index: idx }
          );
        }
      } else {
        if (
          !Number.isInteger(output.amount) ||
          (output.amount as number) < dustLimit
        ) {
          throw createMinerError(
            ZeldMinerErrorCode.DUST_OUTPUT,
            `outputs[${idx}].amount must be at least ${dustLimit} sats`,
            { index: idx, addressType: validation.addressType }
          );
        }
      }
    });
  }

  private validateDistribution(
    outputs: TxOutput[],
    distribution?: bigint[]
  ): bigint[] | undefined {
    if (distribution === undefined) return undefined;
    if (!Array.isArray(distribution)) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "distribution must be an array of bigint values"
      );
    }

    if (distribution.length !== outputs.length) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "distribution length must match number of outputs",
        { expected: outputs.length, actual: distribution.length }
      );
    }

    return distribution.map((value, idx) => {
      if (typeof value !== "bigint") {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          "distribution values must be bigint",
          { index: idx }
        );
      }
      if (value < 0n) {
        throw createMinerError(
          ZeldMinerErrorCode.INVALID_INPUT,
          "distribution values must be non-negative",
          { index: idx }
        );
      }
      return value;
    });
  }

  async buildMiningTemplate(params: {
    inputs: TxInput[];
    outputs: TxOutput[];
    startNonce: bigint;
    batchSize: number;
    distribution?: bigint[];
  }): Promise<MiningTemplate & { nonceLength: number }> {
    const { inputs, outputs, startNonce, batchSize, distribution } = params;
    this.validateInputs(inputs);
    await this.validateOutputs(outputs);
    const validatedDistribution = this.validateDistribution(outputs, distribution);
    const useCborNonce = Array.isArray(validatedDistribution);
    const nonceLength = this.assertNonceRange(startNonce, batchSize, useCborNonce);

    const wasm = await this.getWasm();
    try {
      const template = wasm.build_mining_template(
        this.cloneInputs(inputs),
        this.cloneOutputs(outputs),
        normalizeNetwork(this.network),
        BigInt(this.satsPerVbyte),
        startNonce,
        batchSize,
        validatedDistribution ?? null
      );

      if (
        !template ||
        !(template.prefix instanceof Uint8Array) ||
        !(template.suffix instanceof Uint8Array)
      ) {
        throw createMinerError(
          ZeldMinerErrorCode.WORKER_ERROR,
          "WASM returned an invalid mining template"
        );
      }

      const templateUsesCbor = template.useCborNonce ?? useCborNonce;
      return { ...template, nonceLength, useCborNonce: templateUsesCbor };
    } catch (err) {
      throw mapWasmError(err, "build_mining_template", {
        startNonce,
        batchSize,
        distribution: validatedDistribution,
      });
    }
  }

  async buildPsbt(params: {
    inputs: TxInput[];
    outputs: TxOutput[];
    nonce: bigint;
    distribution?: bigint[];
  }): Promise<string> {
    const { inputs, outputs, nonce, distribution } = params;
    this.validateInputs(inputs);
    await this.validateOutputs(outputs);
    const validatedDistribution = this.validateDistribution(outputs, distribution);

    if (nonce < 0n || nonce > MAX_U64) {
      throw createMinerError(
        ZeldMinerErrorCode.INVALID_INPUT,
        "nonce must be between 0 and 2^64 - 1",
        { nonce }
      );
    }

    const wasm = await this.getWasm();
    try {
      return wasm.build_psbt(
        this.cloneInputs(inputs),
        this.cloneOutputs(outputs),
        normalizeNetwork(this.network),
        BigInt(this.satsPerVbyte),
        nonce,
        validatedDistribution ?? null
      );
    } catch (err) {
      throw mapWasmError(err, "build_psbt", { nonce, distribution: validatedDistribution });
    }
  }
}

