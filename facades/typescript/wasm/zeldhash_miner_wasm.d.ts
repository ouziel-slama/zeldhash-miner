/* tslint:disable */
/* eslint-disable */

export function build_mining_template(inputs: any, outputs: any, network: string, sats_per_vbyte: bigint, start_nonce: bigint, batch_size: number, distribution: any): any;

export function build_psbt(inputs: any, outputs: any, network: string, sats_per_vbyte: bigint, nonce: bigint, distribution: any): string;

export function calibrate_batch_size(): Promise<number>;

export function compute_txid(tx_bytes: Uint8Array): string;

export function init_gpu(): Promise<any>;

export function init_panic_hook(): void;

/**
 * Dispatch a GPU batch and return the match with the smallest nonce (OP_RETURN value).
 */
export function mine_batch_gpu(tx_prefix: Uint8Array, tx_suffix: Uint8Array, start_nonce: bigint, batch_size: number, target_zeros: number, use_cbor_nonce: boolean): Promise<any>;

export function mine_batch_wasm(tx_prefix: Uint8Array, tx_suffix: Uint8Array, start_nonce: bigint, batch_size: number, target_zeros: number, use_cbor_nonce: boolean): any;

export function mine_range_gpu(inputs: any, outputs: any, network: string, sats_per_vbyte: bigint, range: any, target_zeros: number, distribution: any): Promise<any>;

/**
 * Mine across a nonce range, automatically splitting at byte-length boundaries.
 */
export function mine_range_wasm(inputs: any, outputs: any, network: string, sats_per_vbyte: bigint, range: any, target_zeros: number, distribution: any): any;

export function validate_address(addr: string, network: string): any;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
  readonly memory: WebAssembly.Memory;
  readonly mine_batch_wasm: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => any;
  readonly mine_range_wasm: (a: any, b: any, c: number, d: number, e: bigint, f: any, g: number, h: any) => any;
  readonly validate_address: (a: number, b: number, c: number, d: number) => any;
  readonly build_psbt: (a: any, b: any, c: number, d: number, e: bigint, f: bigint, g: any) => [number, number, number, number];
  readonly build_mining_template: (a: any, b: any, c: number, d: number, e: bigint, f: bigint, g: number, h: any) => [number, number, number];
  readonly compute_txid: (a: number, b: number) => [number, number];
  readonly init_gpu: () => any;
  readonly mine_batch_gpu: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => any;
  readonly mine_range_gpu: (a: any, b: any, c: number, d: number, e: bigint, f: any, g: number, h: any) => any;
  readonly calibrate_batch_size: () => any;
  readonly init_panic_hook: () => void;
  readonly wasm_bindgen__convert__closures_____invoke__h46584d0b4cdbb530: (a: number, b: number, c: any) => void;
  readonly wasm_bindgen__closure__destroy__h6696f91b6795d985: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures_____invoke__h7dea4b7f169146c3: (a: number, b: number, c: any) => void;
  readonly wasm_bindgen__closure__destroy__h226a531c6ac0c8ef: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures_____invoke__h998b7a7fb3eb0867: (a: number, b: number, c: any, d: any) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __externref_table_alloc: () => number;
  readonly __wbindgen_externrefs: WebAssembly.Table;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __externref_table_dealloc: (a: number) => void;
  readonly __wbindgen_start: () => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
* Instantiates the given `module`, which can either be bytes or
* a precompiled `WebAssembly.Module`.
*
* @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
*
* @returns {InitOutput}
*/
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
* If `module_or_path` is {RequestInfo} or {URL}, makes a request and
* for everything else, calls `WebAssembly.instantiate` directly.
*
* @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
*
* @returns {Promise<InitOutput>}
*/
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
