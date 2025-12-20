#!/bin/bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

log() {
  printf "==> %s\n" "$1"
}

install_and_build() {
  local package_dir="$1"
  local label="$2"

  if [ "${SKIP_INSTALL:-0}" != "1" ]; then
    log "Installing dependencies for ${label}"
    npm ci --prefix "${package_dir}"
  else
    log "Skipping dependency install for ${label}"
  fi

  log "Building ${label}"
  npm run build --prefix "${package_dir}"
}

log "Building WASM bindings"
"${ROOT_DIR}/scripts/build-wasm.sh"

install_and_build "${ROOT_DIR}/facades/typescript" "zeldminer TypeScript facade"
install_and_build "${ROOT_DIR}/examples/web-demo" "demo web app"

log "Build pipeline completed successfully"

