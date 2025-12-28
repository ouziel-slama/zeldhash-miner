# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.2.2] - 2025-12-28

### Fixed

#### TypeScript SDK
- **Fixed WASM loading in Vite dev mode**: Added automatic bootstrap that sets `globalThis.__ZELDMINER_WASM_BASE__` to `/wasm/` on the application's origin, preventing 404 errors when loading WASM from `node_modules`. Both the main bundle and worker now resolve WASM paths relative to the host application instead of `import.meta.url`

---

## [0.2.1] - 2025-12-28

### Fixed

#### TypeScript SDK
- Fixed Vite `base` config to use relative paths (`"./"`) so worker and script URLs stay relative in the published npm build

---

## [0.2.0] - 2025-12-27

### Changed

#### Output Ordering (Breaking)
- **Stable output order**: Non-OP_RETURN outputs now preserve the exact order provided by the caller, including the change output at its specified position
- **OP_RETURN always last**: The OP_RETURN output is always appended as the final output in the transaction
- **`TransactionPlan` refactored**: Replaced `user_outputs` + `change_output` with a unified `outputs` array and `change_index` field

**Migration**: If you relied on the previous ordering (user outputs → OP_RETURN → change), update your code to expect the new order (caller-specified outputs → OP_RETURN). The change output is now at the index you specified, not always last.

### Fixed

#### WASM Module
- Fixed deprecated `init()` call syntax in TypeScript SDK — now passes `{ module_or_path: url }` object instead of URL directly
- Build script now strips wasm-bindgen deprecation warnings from generated JS glue code

#### Release Scripts
- Fixed `release-crate.sh` array parsing with proper `read -ra` syntax
- Added crate existence check to skip already published versions on crates.io
- Fixed `tomllib.load()` to use binary file mode (Python 3.11+ compliance)
- Fixed `release-npm.sh` Node.js heredoc syntax for cross-platform compatibility
- Added `User-Agent` header to crates.io API requests (required by their rate limiting)

#### CI/CD
- Release workflow now triggers on GitHub release publish events
- Added `environment: default` for proper secrets access

### Documentation

- Updated README with stable output ordering behavior
- Updated `ARCHITECTURE.md` with new `TransactionPlan` structure

---

## [0.1.0] - 2024-12-25

### Added

#### Core Mining Engine
- Bitcoin vanity transaction miner that finds txids with leading zero hex digits
- Double-SHA256 hash computation with configurable difficulty target (0-32 leading zeros)
- Nonce iteration via `OP_RETURN` output modification
- Mining template system with prefix/suffix splitting for efficient batch processing
- Minimal big-endian nonce encoding with automatic segment splitting at byte boundaries

#### Transaction Construction
- SegWit support for P2WPKH and P2TR (Taproot) addresses
- Bech32/Bech32m address parsing and validation
- PSBT generation with `WITNESS_UTXO` for wallet signing
- Virtual size (vsize) and fee estimation with configurable sats/vbyte
- Dust limit enforcement (310 sats for P2WPKH, 330 sats for P2TR)
- RBF-enabled transactions (default sequence `0xFFFFFFFD`)

#### ZELD Distribution
- Optional CBOR-encoded distribution array in `OP_RETURN`
- Format: `OP_RETURN | "ZELD" | CBOR([distribution..., nonce])`
- CBOR nonce encoding following RFC 8949

#### Multi-Platform Support
- **Rust SDK** (`zeldhash-miner` crate) with CPU and optional GPU backends
- **TypeScript SDK** (`zeldhash-miner` npm package) with WebAssembly bindings
- `no_std` compatible core library (`zeldhash-miner-core`) using only `alloc`

#### CPU Parallelization
- Multi-threaded mining with Rayon (Rust native)
- Web Workers support for browser environments (TypeScript SDK)
- Stride-based work distribution across workers
- Atomic early termination when a match is found

#### GPU Acceleration
- WebGPU compute shaders (WGSL) for parallel hash computation
- 256-thread workgroups with automatic batch size tuning
- Buffer pooling and template caching for performance
- Graceful fallback to CPU when GPU is unavailable

#### Control Flow
- Pause/resume/stop controls for mining operations
- Progress callbacks with hash rate and elapsed time
- Event emitter pattern in TypeScript SDK (`on('progress')`, `on('found')`, etc.)

#### Developer Experience
- Comprehensive error handling with typed error codes
- Optional `serde` support for Rust types
- Full API documentation in SDK READMEs
- Web demo application (Vite-based)

### Architecture

```
zeldhash-miner/
├── crates/
│   ├── core/          # no_std algorithms (hash, tx, psbt, fees, nonce)
│   ├── gpu/           # WebGPU backend with WGSL shaders
│   ├── wasm/          # wasm-bindgen bindings
│   └── python-core/   # Placeholder for future pyo3 wheel
├── facades/
│   ├── rust/          # crates.io SDK
│   └── typescript/    # npm SDK + WASM artifacts
└── examples/
    └── web-demo/      # Browser demo application
```

### Performance

| Backend           | Typical Hash Rate |
|-------------------|-------------------|
| CPU (single core) | 200-500 KH/s      |
| CPU (8 cores)     | 1-3 MH/s          |
| Integrated GPU    | 5-20 MH/s         |
| Discrete GPU      | 50-200+ MH/s      |

### Security

- Produces unsigned PSBTs only — no private keys handled
- Input validation for addresses, amounts, and script lengths
- Deterministic outputs for reproducible builds
- GPU shaders run in browser sandbox

---

[0.2.1]: https://github.com/zeldhash/zeldhash-miner/releases/tag/v0.2.1
[0.2.0]: https://github.com/zeldhash/zeldhash-miner/releases/tag/v0.2.0
[0.1.0]: https://github.com/zeldhash/zeldhash-miner/releases/tag/v0.1.0

