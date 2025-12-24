Zeldhash Miner – Repository Layout Proposal
===========================================

Context
-------
- Goal: publish Rust crate, npm package, and (later) pip package via pyo3, with GPU-friendly mining APIs.
- Pain point: unclear structure; wasm artifacts duplicated across multiple folders; demo frontend mixed with sources.

Recommended Final Tree (with shared facades/)
---------------------------------------------
```
zeldhash-miner/
  Cargo.toml                   # workspace root
  rust-toolchain.toml
  crates/                      # core Rust building blocks
    core/                      # domain logic (PSBT, hash, fees, nonce, builder…)
    kernel/                    # low-level CPU primitives
    gpu/                       # GPU acceleration
    wasm/                      # wasm-bindgen target; depends on core/gpu
    python-core/               # pyo3 crate producing the wheel; depends on core/gpu

  facades/                     # public APIs per ecosystem (grouped together)
    rust/                      # crate published to crates.io; pub use core/gpu
      Cargo.toml
      src/lib.rs
    typescript/                # npm package (TypeScript), consumes wasm artifacts
      package.json
      src/
      wasm/                    # copied from crates/wasm/target
      tests/
    python/                    # pip packaging; consumes crates/python-core
      pyproject.toml
      README.md

  examples/
    web-demo/                  # Vite app consuming facades/ts + wasm
    cli/                       # CLI or bench examples (Rust)

  scripts/
    build-wasm.sh
    build-all.sh
    release-npm.sh
    release-pypi.sh
    release-crate.sh

  docs/
    ARCHITECTURE.md
    RELEASING.md               # release flows for crate + npm + pip

  .githooks/
  .github/                     # CI/CD (lint, test, release)
```

Analysis (what changes vs current)
----------------------------------
- Core Rust stays in `crates/` (core/kernel/gpu/wasm/python-core). No public API logic lives at workspace root.
- All public-facing SDKs are co-located in `facades/` (rust, ts, python) to make “the three APIs” discoverable together.
- Wasm artifacts are centralized: built in `crates/wasm`, then copied to `facades/ts/wasm/` and optionally to `examples/web-demo/public/wasm/`. Avoid keeping generated files under multiple folders.
- Frontend demo moves to `examples/web-demo/` to separate product code from example UX.
- Pip flow is explicit: `crates/python-core` builds the wheel; `facades/python` provides packaging metadata and docs.

Action Plan (repo to target layout)
-----------------------------------
- Workspace + crates
  - Update `Cargo.toml` workspace members to include `crates/*` and new `facades/rust` crate; keep crate names stable (`zeldhash-miner-*`) to avoid downstream API breaks.
  - Move/rename crates to match the layout: `crates/zeldhash-miner-core` -> `crates/core`, `crates/zeldhash-miner-kernel` -> `crates/kernel`, `crates/zeldhash-miner-gpu` -> `crates/gpu`, `crates/zeldhash-miner-wasm` -> `crates/wasm`. Adjust path dependencies accordingly without changing published crate names unless necessary.
  - Create `facades/rust/` and move the current orchestrator (`src/lib.rs`, package metadata) there; leave the workspace root as a pure workspace manifest.
  - Add a placeholder `crates/python-core/` (pyo3 target) and wire it into the workspace when ready; keep API surface aligned with core/gpu.

- TypeScript facade
- Move `packages/zeldhash-miner` to `facades/typescript/` keeping the npm name `zeldhash-miner`.
  - Relocate the wasm artifacts to `facades/typescript/wasm/` (copied from `crates/wasm/pkg/`) and update `wasm.ts` imports/URL resolution to the new relative path.
  - Update `tsconfig`, `vite.config`, tests, and path references to the new folder depth (e.g., fixtures/worker paths, worker entry).

- WASM build flow
  - Update `scripts/build-wasm.sh` to use `crates/wasm` as the source and to sync outputs into `facades/typescript/wasm/` and `examples/web-demo/public/wasm/`.
  - Keep the GPU toggle (`WASM_GPU`) and the type guard that checks for `mine_batch_gpu` in the bindings; ensure wasm-opt still runs if available.
  - Ensure `pkg/` is not committed; keep one canonical copy in `facades/typescript/wasm`.

- Demo / examples
  - Move `web/` to `examples/web-demo/` and adjust its dependency to point to `facades/typescript` (likely `file:../../facades/typescript`).
  - Update the demo’s wasm asset path (public/wasm) to match the new copy location and its import URLs (Vite assets, `new URL("./wasm/...", import.meta.url)`).
  - Verify `scripts/build-all.sh` builds in order: wasm -> facades/typescript -> examples/web-demo. Add `SKIP_INSTALL` behavior as before.

- Release tooling & docs
  - Fix `scripts/release-crate.sh` default path (currently points to non-existent `facades/rust`) once the move is done; add npm/pypi release scripts under `scripts/`.
  - Update `README.md`, `docs/ARCHITECTURE.md`, and `docs/RELEASING.md` (new) to reflect the new tree and flows (crate publish, npm publish, future pip).
  - Refresh CI/configs (if any) and `.githooks/` to point to the new paths for lint/test/build.

Points of attention / do-not-break
----------------------------------
- Preserve crate names and features (`cpu`, `gpu`, `rayon`, `serde`) so dependent code continues to compile; watch for path/feature assumptions in `src/lib.rs` and `crates/*`.
- Ensure the wasm artifact loading still resolves correctly in both the npm package (ESM import URL) and the demo (Vite public path); broken asset paths are the likeliest regression.
- After moving files, re-run tests/builds: `cargo test -p zeldhash-miner-core`, `cargo test -p zeldhash-miner`, `wasm-pack build crates/wasm --target web`, `npm test --prefix facades/typescript`, and Vite build for the demo.
- Audit relative imports inside `packages/zeldhash-miner/src` (workers, fixtures) after the move; adjust any hardcoded `../wasm` or `../../` paths.
- Keep the workspace root clean (no src/) once the orchestrator moves; double-check `Cargo.lock` and `package-lock.json` paths when relocating folders to avoid stale lockfiles.
