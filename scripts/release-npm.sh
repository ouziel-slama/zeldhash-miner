#!/usr/bin/env bash
set -euo pipefail

# Publish the npm package from facades/typescript with optional checks.
# Environment toggles:
#   PKG_DIR      - package directory relative to repo root (default: facades/typescript)
#   SKIP_TESTS   - set to 1 to skip npm test/typecheck (default: 0)
#   SKIP_BUILD   - set to 1 to skip npm run build (default: 0)
#   NPM_TAG      - dist-tag to publish under (default: latest)

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

PKG_DIR="${PKG_DIR:-facades/typescript}"
SKIP_TESTS="${SKIP_TESTS:-0}"
SKIP_BUILD="${SKIP_BUILD:-0}"
NPM_TAG="${NPM_TAG:-latest}"

PKG_PATH="${ROOT_DIR}/${PKG_DIR}"
TMP_NPMRC=""

log() {
  printf "==> %s\n" "$*"
}

read_name_version() {
  local manifest="$1/package.json"
  if [[ ! -f "${manifest}" ]]; then
    echo "error: package.json not found at ${manifest}" >&2
    exit 1
  fi
  node -e "
const fs = require('fs');
const pkg = JSON.parse(fs.readFileSync('${manifest}', 'utf8'));
console.log(pkg.name + ' ' + pkg.version);
"
}

if [[ ! -d "${PKG_PATH}" ]]; then
  echo "error: package directory not found: ${PKG_PATH}" >&2
  exit 1
fi

read -r PKG_NAME PKG_VERSION < <(read_name_version "${PKG_PATH}")

cleanup() {
  if [[ -n "${TMP_NPMRC}" && -f "${TMP_NPMRC}" ]]; then
    rm -f "${TMP_NPMRC}"
  fi
}
trap cleanup EXIT

if [[ -n "${NPM_TOKEN:-}" ]]; then
  TMP_NPMRC="$(mktemp)"
  printf "//registry.npmjs.org/:_authToken=%s\nregistry=https://registry.npmjs.org/\nalways-auth=true\n" "${NPM_TOKEN}" > "${TMP_NPMRC}"
  export NPM_CONFIG_USERCONFIG="${TMP_NPMRC}"
  log "Configured npm auth via NPM_TOKEN"
else
  log "warning: NPM_TOKEN is not set; npm publish may fail"
fi

log "Publishing ${PKG_NAME} v${PKG_VERSION} from ${PKG_PATH}"

log "Building WASM artifacts"
"${ROOT_DIR}/scripts/build-wasm.sh"

log "Installing dependencies"
npm ci --prefix "${PKG_PATH}"

if [[ "${SKIP_TESTS}" != "1" ]]; then
  log "Running typecheck and tests"
  npm run typecheck --prefix "${PKG_PATH}"
  npm test --prefix "${PKG_PATH}"
else
  log "Skipping tests (SKIP_TESTS=${SKIP_TESTS})"
fi

if [[ "${SKIP_BUILD}" != "1" ]]; then
  log "Building package"
  npm run build --prefix "${PKG_PATH}"
else
  log "Skipping build (SKIP_BUILD=${SKIP_BUILD})"
fi

log "Publishing to npm (tag: ${NPM_TAG})"
(cd "${PKG_PATH}" && npm publish --access public --tag "${NPM_TAG}")


