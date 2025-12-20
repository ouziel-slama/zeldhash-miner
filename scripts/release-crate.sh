#!/usr/bin/env bash
set -euo pipefail

# Publish the Rust crate to crates.io.
# By default targets the facade crate at facades/rust; override with CRATE_DIR=path/to/crate.

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_DIR="${CRATE_DIR:-facades/rust}"
CRATE_PATH="${ROOT_DIR}/${CRATE_DIR}"

if [ ! -f "${CRATE_PATH}/Cargo.toml" ]; then
  echo "error: Cargo.toml not found at ${CRATE_PATH} (override with CRATE_DIR=...)" >&2
  exit 1
fi

pushd "${CRATE_PATH}" >/dev/null

if [ "${SKIP_TESTS:-0}" -ne 1 ]; then
  echo "Running tests..."
  cargo test --locked
else
  echo "Skipping tests (SKIP_TESTS=1)."
fi

PUBLISH_ARGS=("--locked")
if [ "${ALLOW_DIRTY:-0}" -eq 1 ]; then
  PUBLISH_ARGS+=("--allow-dirty")
fi

echo "Publishing crate from ${CRATE_PATH} ..."
cargo publish "${PUBLISH_ARGS[@]}"

popd >/dev/null

