# Releasing Zeldhash Miner

This repository ships three artifacts: the Rust crate (`zeldhash-miner`), the npm package (`zeldhash-miner`), and a future Python wheel (via `crates/python-core`). The TypeScript facade and the web demo depend on the WASM bindings built from `crates/wasm`.

## Pre-flight checklist

- Update versions where appropriate (crate `facades/rust/Cargo.toml`, npm `facades/typescript/package.json`).
- Verify the workspace is clean and CI-quality checks pass:
  - `cargo fmt --all -- --check`
  - `cargo test -p zeldhash-miner-core`
  - `cargo test -p zeldhash-miner` (add `--no-default-features --features "cpu serde"` when you need CPU-only coverage)
  - `wasm-pack build crates/wasm --target web` or `./scripts/build-wasm.sh` to refresh `facades/typescript/wasm/` and `examples/web-demo/public/wasm/`
    - GPU on by default (`WASM_GPU=1`); set `WASM_GPU=0` (or `false`) to force CPU-only when rust-gpu toolchains are unavailable.
  - `npm test --prefix facades/typescript`
  - `npm run build --prefix examples/web-demo` (smoke test the demo)
- Make sure `pkg/` outputs remain untracked; `scripts/build-wasm.sh` copies the canonical artifacts into `facades/typescript/wasm/`.

## Releasing the Rust crate (`zeldhash-miner`)

1. Bump the version in `facades/rust/Cargo.toml` (and `Cargo.lock` if you regenerate it).
2. Run tests (or set `SKIP_TESTS=1` if you intentionally skip them).
3. Publish from the repo root:
   ```bash
   ./scripts/release-crate.sh              # defaults to facades/rust
   # optionally: CRATE_DIR=path/to/crate ALLOW_DIRTY=1 ./scripts/release-crate.sh
   ```

## Releasing the npm package (`zeldhash-miner`)

1. Bump the version in `facades/typescript/package.json` and `package-lock.json`.
2. Regenerate WASM artifacts:
   ```bash
   ./scripts/build-wasm.sh   # copies outputs into facades/typescript/wasm/ and the demo
   # GPU on by default; use WASM_GPU=0 or WASM_GPU=false for CPU-only
   ```
3. Build and publish the package:
   ```bash
   npm ci --prefix facades/typescript
   npm run build --prefix facades/typescript
   (cd facades/typescript && npm publish --access public)
   ```

## Python wheel (planned)

The placeholder crate `crates/python-core` is reserved for a future pyo3-based wheel that will reuse `crates/core` and optionally `crates/gpu`. When the bindings are implemented, add a `facades/python/` packaging folder and document the release flow here (likely via maturin).

## Release order and quick pipelines

- For a full pipeline (WASM → npm → demo build), use:
  ```bash
  ./scripts/build-all.sh
  ```
- Typical order of operations: refresh WASM, publish the Rust crate, then publish the npm package once the artifacts are in place.

