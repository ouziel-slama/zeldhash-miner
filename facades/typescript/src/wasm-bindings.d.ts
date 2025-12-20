declare module "../wasm/zeldhash_miner_wasm.js" {
  export type InitInput =
    | RequestInfo
    | URL
    | Response
    | BufferSource
    | WebAssembly.Module;

  export interface InitOutput {
    readonly memory: WebAssembly.Memory;
  }

  const init: (moduleOrPath?: InitInput | Promise<InitInput>) => Promise<InitOutput>;
  export default init;

  export function init_panic_hook(): void;
  export function mine_batch_wasm(
    txPrefix: Uint8Array,
    txSuffix: Uint8Array,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    useCborNonce?: boolean
  ): unknown;
  export function mine_range_wasm(
    inputs: unknown,
    outputs: unknown,
    network: string,
    satsPerVbyte: bigint,
    range: { startNonce: bigint; batchSize: number },
    targetZeros: number,
    distribution?: bigint[] | null
  ): unknown;
  export function mine_batch_gpu(
    txPrefix: Uint8Array,
    txSuffix: Uint8Array,
    startNonce: bigint,
    batchSize: number,
    targetZeros: number,
    useCborNonce?: boolean
  ): Promise<unknown>;
  export function mine_range_gpu(
    inputs: unknown,
    outputs: unknown,
    network: string,
    satsPerVbyte: bigint,
    range: { startNonce: bigint; batchSize: number },
    targetZeros: number,
    distribution?: bigint[] | null
  ): Promise<unknown>;
  export function validate_address(
    addr: string,
    network: string
  ): unknown;
  export function build_psbt(
    inputs: unknown,
    outputs: unknown,
    network: string,
    satsPerVbyte: bigint,
    nonce: bigint,
    distribution?: bigint[] | null
  ): string;
  export function build_mining_template(
    inputs: unknown,
    outputs: unknown,
    network: string,
    satsPerVbyte: bigint,
    startNonce: bigint,
    batchSize: number,
    distribution?: bigint[] | null
  ): unknown;
  export function compute_txid(txBytes: Uint8Array): string;
  export function init_gpu(): Promise<unknown>;
  export function calibrate_batch_size(): Promise<number>;
}

