import type { WasmExports } from "./types";

let wasmModule: WasmExports | null = null;
let wasmInitPromise: Promise<WasmExports> | null = null;

// Chrome/Dawn can reject requestDevice when optional limits (e.g.
// maxInterStageShaderComponents) are present but unsupported. Strip any
// limit keys the adapter doesn't expose to keep WebGPU initialization
// portable across browser versions.
let webGpuLimitShimInstalled = false;
const installWebGpuLimitShim = (): void => {
  if (webGpuLimitShimInstalled) return;
  webGpuLimitShimInstalled = true;

  const adapterProto = (globalThis as typeof globalThis & { GPUAdapter?: { prototype?: unknown } })
    .GPUAdapter?.prototype as GPUAdapter | undefined;
  const requestDevice = adapterProto?.requestDevice;
  if (!adapterProto || typeof requestDevice !== "function") return;

  adapterProto.requestDevice = function patchedRequestDevice(
    this: GPUAdapter,
    descriptor?: GPUDeviceDescriptor
  ): Promise<GPUDevice> {
    if (descriptor?.requiredLimits && typeof this.limits === "object") {
      const limits = descriptor.requiredLimits as Record<string, unknown>;
      const supported = this.limits as unknown as Record<string, unknown>;
      for (const key of Object.keys(limits)) {
        if (!(key in supported) || supported[key] === undefined) {
          delete limits[key];
        }
      }
    }
    return requestDevice.call(this, descriptor);
  };
};

const ensureTrailingSlash = (value: string): string =>
  value.endsWith("/") ? value : `${value}/`;

const toAbsoluteBase = (base: string): string => {
  const trimmed = base.trim();
  if (!trimmed) return trimmed;
  // Use window origin when available so Vite treats it as an external URL and does not try to transform public assets.
  if (typeof window !== "undefined" && typeof window.location?.origin === "string") {
    return ensureTrailingSlash(new URL(trimmed, window.location.origin).href);
  }
  return ensureTrailingSlash(new URL(trimmed, import.meta.url).href);
};

const resolveWasmBase = (): string => {
  const globalBase = (globalThis as { __ZELDMINER_WASM_BASE__?: unknown })
    .__ZELDMINER_WASM_BASE__;
  if (typeof globalBase === "string" && globalBase.trim()) {
    return toAbsoluteBase(globalBase);
  }

  const envBase = (import.meta as ImportMeta & { env?: Record<string, unknown> })
    .env?.VITE_ZELDMINER_WASM_BASE;
  if (typeof envBase === "string" && envBase.trim()) {
    return toAbsoluteBase(envBase);
  }

  const viteBase = (import.meta as ImportMeta & { env?: Record<string, unknown> }).env?.BASE_URL;
  if (typeof viteBase === "string" && viteBase.trim()) {
    return toAbsoluteBase(`${ensureTrailingSlash(viteBase.trim())}wasm/`);
  }

  return toAbsoluteBase("/wasm/");
};

const WASM_BASE_URL = resolveWasmBase();
const WASM_JS_PATH = `${WASM_BASE_URL}zeldhash_miner_wasm.js`;
const WASM_BINARY_PATH = `${WASM_BASE_URL}zeldhash_miner_wasm_bg.wasm`;

const formatError = (err: unknown): string =>
  err instanceof Error ? err.message : String(err);

const loadModule = async (): Promise<WasmExports> => {
  installWebGpuLimitShim();

  let bindings: unknown;
  try {
    bindings = await import(/* @vite-ignore */ WASM_JS_PATH);
  } catch (err) {
    throw new Error(
      `Failed to import WASM bundle (${WASM_JS_PATH}). ` +
        `Did you run ./scripts/build-wasm.sh? (${formatError(err)})`
    );
  }

  const init = (bindings as { default?: unknown }).default;
  if (typeof init !== "function") {
    throw new Error("WASM init function is missing from the bundle.");
  }

  try {
    const wasmUrl = new URL(WASM_BINARY_PATH, import.meta.url);
    await init({ module_or_path: wasmUrl });
  } catch (err) {
    throw new Error(
      `Failed to initialize WASM bundle: ${formatError(err)}`
    );
  }

  const typedBindings = bindings as WasmExports;
  try {
    typedBindings.init_panic_hook?.();
  } catch {
    /* ignore optional panic hook failures */
  }

  return typedBindings;
};

export const loadWasm = async (): Promise<WasmExports> => {
  if (wasmModule) return wasmModule;
  if (!wasmInitPromise) {
    wasmInitPromise = loadModule()
      .then((mod) => {
        wasmModule = mod;
        return mod;
      })
      .catch((err) => {
        wasmInitPromise = null;
        throw err;
      });
  }
  return wasmInitPromise;
};

export const getWasm = (): WasmExports | null => wasmModule;

export const resetWasm = (): void => {
  wasmModule = null;
  wasmInitPromise = null;
};

