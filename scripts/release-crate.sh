#!/usr/bin/env bash
set -euo pipefail

# Publish all Rust crates to crates.io in dependency order, waiting for each
# crate version to become available before moving to the next. Defaults to the
# workspace chain: core -> gpu -> wasm -> facade.
#
# Environment toggles:
#   CRATE_DIRS   - space-separated list of crate directories (relative to repo root)
#   SKIP_TESTS   - set to 1 to skip per-crate tests (defaults to 0, except wasm)
#   ALLOW_DIRTY  - set to 1 to pass --allow-dirty to cargo publish
#   MAX_WAIT_ATTEMPTS - how many times to poll crates.io (default 30)
#   WAIT_SLEEP_SECS   - delay between polls in seconds (default 10)

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"

DEFAULT_CRATE_DIRS=(
  "crates/core"
  "crates/gpu"
  "crates/wasm"
  "facades/rust"
)

readarray -t CRATE_DIRS_ARRAY <<<"${CRATE_DIRS:-${DEFAULT_CRATE_DIRS[*]}}"

ALLOW_DIRTY="${ALLOW_DIRTY:-0}"
SKIP_TESTS="${SKIP_TESTS:-0}"
MAX_WAIT_ATTEMPTS="${MAX_WAIT_ATTEMPTS:-30}"
WAIT_SLEEP_SECS="${WAIT_SLEEP_SECS:-10}"

PUBLISH_ARGS=("--locked")
if [[ "${ALLOW_DIRTY}" == "1" ]]; then
  PUBLISH_ARGS+=("--allow-dirty")
fi

log() {
  printf "==> %s\n" "$*"
}

read_name_version() {
  local manifest="$1/Cargo.toml"
  if [[ ! -f "${manifest}" ]]; then
    echo "error: Cargo.toml not found at ${manifest}" >&2
    exit 1
  fi
  python - "$manifest" <<'PY'
import sys, tomllib
path = sys.argv[1]
data = tomllib.loads(open(path, "rb").read())
pkg = data.get("package") or {}
print(f"{pkg.get('name')} {pkg.get('version')}")
PY
}

wait_for_crate() {
  local crate="$1"
  local version="$2"
  local attempt=1
  local url="https://crates.io/api/v1/crates/${crate}/${version}"

  while (( attempt <= MAX_WAIT_ATTEMPTS )); do
    if curl -sSf -o /dev/null "${url}"; then
      log "Crate ${crate} v${version} is available on crates.io"
      return 0
    fi
    log "Waiting for ${crate} v${version} to appear (${attempt}/${MAX_WAIT_ATTEMPTS})..."
    sleep "${WAIT_SLEEP_SECS}"
    ((attempt++))
  done

  echo "error: timed out waiting for ${crate} v${version} on crates.io" >&2
  return 1
}

# Ensure formatting matches repository expectations before publishing.
log "Running cargo fmt --check"
(cd "${ROOT_DIR}" && cargo fmt --all -- --check)

for dir in "${CRATE_DIRS_ARRAY[@]}"; do
  CRATE_PATH="${ROOT_DIR}/${dir}"
  read -r CRATE_NAME CRATE_VERSION < <(read_name_version "${CRATE_PATH}")

  log "Publishing ${CRATE_NAME} v${CRATE_VERSION} from ${CRATE_PATH}"

  if [[ "${SKIP_TESTS}" != "1" && "${CRATE_NAME}" != "zeldhash-miner-wasm" ]]; then
    (cd "${ROOT_DIR}" && cargo test -p "${CRATE_NAME}" --locked)
  else
    log "Skipping tests for ${CRATE_NAME} (SKIP_TESTS=${SKIP_TESTS})"
  fi

  (cd "${CRATE_PATH}" && cargo publish "${PUBLISH_ARGS[@]}")
  wait_for_crate "${CRATE_NAME}" "${CRATE_VERSION}"
done

