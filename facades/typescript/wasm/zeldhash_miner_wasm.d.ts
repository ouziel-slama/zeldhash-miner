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
  readonly mine_batch_wasm: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => number;
  readonly mine_range_wasm: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => number;
  readonly validate_address: (a: number, b: number, c: number, d: number) => number;
  readonly build_psbt: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: bigint, h: number) => void;
  readonly build_mining_template: (a: number, b: number, c: number, d: number, e: number, f: bigint, g: bigint, h: number, i: number) => void;
  readonly compute_txid: (a: number, b: number, c: number) => void;
  readonly init_gpu: () => number;
  readonly mine_batch_gpu: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => number;
  readonly mine_range_gpu: (a: number, b: number, c: number, d: number, e: bigint, f: number, g: number, h: number) => number;
  readonly calibrate_batch_size: () => number;
  readonly init_panic_hook: () => void;
  readonly wasm_bindgen__convert__closures_____invoke__h876fe25d0032356a: (a: number, b: number, c: number) => void;
  readonly wasm_bindgen__closure__destroy__h921362508ff5fc1b: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures_____invoke__h6a599d60b2f583dd: (a: number, b: number, c: number) => void;
  readonly wasm_bindgen__closure__destroy__h793fac1df6ff4559: (a: number, b: number) => void;
  readonly wasm_bindgen__convert__closures_____invoke__h7ea8db5a81f38b49: (a: number, b: number, c: number, d: number) => void;
  readonly __wbindgen_exn_store: (a: number) => void;
  readonly __wbindgen_malloc: (a: number, b: number) => number;
  readonly __wbindgen_realloc: (a: number, b: number, c: number, d: number) => number;
  readonly __wbindgen_free: (a: number, b: number, c: number) => void;
  readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
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
