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

# Always ship GPU-enabled WASM bindings.
WASM_PACK_ARGS=(--target web --features gpu)

pushd "${CRATE_DIR}" >/dev/null
wasm-pack build "${WASM_PACK_ARGS[@]}"
popd >/dev/null

strip_init_warnings() {
  local js_file="$1"
  if [ ! -f "${js_file}" ]; then
    return
  fi

  python3 - "$js_file" <<'PYCODE'
from pathlib import Path
import re
import sys

path = Path(sys.argv[1])
src = path.read_text()

replacements = [
    (
        re.compile(
            r"if \(typeof module !== 'undefined'\) {\s*"
            r"if \(Object.getPrototypeOf\(module\) === Object\.prototype\) {\s*\({module} = module\)\s*}"
            r"\s*else {\s*console\.warn\('using deprecated parameters for `initSync\(\)`; pass a single object instead'\)\s*}"
            r"\s*}",
            re.MULTILINE,
        ),
        "if (typeof module !== 'undefined' && Object.getPrototypeOf(module) === Object.prototype) {\\n        ({module} = module)\\n    }",
    ),
    (
        re.compile(
            r"if \(typeof module_or_path !== 'undefined'\) {\s*"
            r"if \(Object.getPrototypeOf\(module_or_path\) === Object\.prototype\) {\s*\({module_or_path} = module_or_path\)\s*}"
            r"\s*else {\s*console\.warn\('using deprecated parameters for the initialization function; pass a single object instead'\)\s*}"
            r"\s*}",
            re.MULTILINE,
        ),
        "if (typeof module_or_path !== 'undefined' && Object.getPrototypeOf(module_or_path) === Object.prototype) {\\n        ({module_or_path} = module_or_path)\\n    }",
    ),
]

updated = src
for pattern, replacement in replacements:
    updated = pattern.sub(replacement, updated)

if updated != src:
    path.write_text(updated)
else:
    print(f"Warning: no deprecation warnings removed in {path}", file=sys.stderr)
PYCODE
}

strip_init_warnings "${PKG_DIR}/zeldhash_miner_wasm.js"

# Sync the generated pkg into the destinations the JS packages expect.
# Exclude .gitignore so npm pack includes the wasm artifacts.
rsync -a --delete --exclude='.gitignore' "${PKG_DIR}/" "${OUT_DIR}/"

TYPES_FILE="${OUT_DIR}/zeldhash_miner_wasm.d.ts"
if [ ! -f "${TYPES_FILE}" ]; then
  echo "TypeScript declarations are missing in ${OUT_DIR}" >&2
  exit 1
fi

if ! grep -q "mine_batch_gpu" "${TYPES_FILE}"; then
  echo "GPU build is expected but mine_batch_gpu is missing in the WASM bindings. Did the gpu feature compile?" >&2
  exit 1
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

