#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="${ROOT_DIR}/crates/wasm"
PKG_DIR="${CRATE_DIR}/pkg"
OUT_DIR="${ROOT_DIR}/facades/typescript/wasm"
WEB_WASM_DIR="${ROOT_DIR}/examples/web-demo/public/wasm"

mkdir -p "${OUT_DIR}"
mkdir -p "${WEB_WASM_DIR}"

# wasm-pack defaults to a release build; run from the crate directory so it
# picks up the correct manifest and avoid cargo receiving unstable flags.
export RUSTFLAGS=--cfg=web_sys_unstable_apis

# Default to GPU-enabled builds so the WebGPU demo ships the right bindings.
# Accept truthy/falsey inputs (1/0, true/false, yes/no) to avoid CI mismatches.
normalize_bool() {
  local raw="${1:-}"
  # Bash on macOS (v3) lacks `${var,,}`; use tr for portability.
  local lowered
  lowered="$(printf '%s' "${raw}" | tr '[:upper:]' '[:lower:]')"
  case "${lowered}" in
    1|true|yes|on) echo 1 ;;
    0|false|no|off) echo 0 ;;
    *) echo 1 ;; # default to GPU-enabled
  esac
}

WASM_GPU="$(normalize_bool "${WASM_GPU:-1}")"
WASM_PACK_ARGS=(--target web)

if [[ "${WASM_GPU}" == "1" ]]; then
  # GPU builds rely on rust-gpu nightly toolchains; opt-in only.
  WASM_PACK_ARGS+=(--features gpu)
else
  # Keep the default CPU-only path compatible with the stable toolchain.
  WASM_PACK_ARGS+=(-- --no-default-features --features cpu)
fi

pushd "${CRATE_DIR}" >/dev/null
wasm-pack build "${WASM_PACK_ARGS[@]}"
popd >/dev/null

# Sync the generated pkg into the destinations the JS packages expect.
# Exclude .gitignore so npm pack includes the wasm artifacts.
rsync -a --delete --exclude='.gitignore' "${PKG_DIR}/" "${OUT_DIR}/"

TYPES_FILE="${OUT_DIR}/zeldhash_miner_wasm.d.ts"
if [ ! -f "${TYPES_FILE}" ]; then
  echo "TypeScript declarations are missing in ${OUT_DIR}" >&2
  exit 1
fi

if [[ "${WASM_GPU}" == "1" ]]; then
  if ! grep -q "mine_batch_gpu" "${TYPES_FILE}"; then
    echo "GPU build requested (WASM_GPU=1) but mine_batch_gpu is missing in the WASM bindings. Did the gpu feature compile?" >&2
    exit 1
  fi
fi

# Optional size optimization if wasm-opt is available.
if command -v wasm-opt >/dev/null 2>&1; then
  TMP_WASM="${OUT_DIR}/zeldhash_miner_wasm_bg.opt.wasm"
  wasm-opt -Oz -o "${TMP_WASM}" "${OUT_DIR}/zeldhash_miner_wasm_bg.wasm"
  mv "${TMP_WASM}" "${OUT_DIR}/zeldhash_miner_wasm_bg.wasm"
fi

cp "${OUT_DIR}/zeldhash_miner_wasm.js" \
  "${OUT_DIR}/zeldhash_miner_wasm_bg.wasm" \
  "${WEB_WASM_DIR}/"

