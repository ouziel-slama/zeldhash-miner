# Zeldhash Miner

[![Tests](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/tests.yml/badge.svg)](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/tests.yml)
[![Clippy](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/clippy.yml/badge.svg)](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/clippy.yml)
[![Fmt](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/fmt.yml/badge.svg)](https://github.com/ouziel-slama/zeldhash-miner/actions/workflows/fmt.yml)
[![codecov](https://codecov.io/gh/ouziel-slama/zeldhash-miner/graph/badge.svg?token=EBST68VDXC)](https://codecov.io/gh/ouziel-slama/zeldhash-miner)
[![Crates.io](https://img.shields.io/crates/v/zeldhash-miner.svg)](https://crates.io/crates/zeldhash-miner)
[![npm](https://img.shields.io/npm/v/zeldhash-miner.svg)](https://www.npmjs.com/package/zeldhash-miner)


**Bitcoin vanity transaction miner** that finds transaction IDs (txids) with leading zero hex digits. The miner builds unsigned PSBTs containing a nonce in an `OP_RETURN` output and iterates until the resulting txid meets the target difficulty.

## Features

- **Multi-platform**: Native Rust and WebAssembly for browser environments
- **GPU acceleration**: WebGPU compute shaders for parallel hash computation
- **CPU parallelization**: Multi-threaded with Rayon (Rust) or Web Workers (browser)
- **SegWit support**: P2WPKH and P2TR (Taproot) addresses
- **PSBT generation**: Produces unsigned PSBTs ready for wallet signing
- **ZELD distribution**: Optional CBOR-encoded distribution in OP_RETURN
- **Stable output ordering**: Non-OP_RETURN outputs stay in the order provided
  (change included); the OP_RETURN is always appended last.

## SDKs

Zeldhash Miner provides two similar SDKs with matching APIs:

| SDK | Package | Documentation |
|-----|---------|---------------|
| **Rust** | [`zeldhash-miner`](https://crates.io/crates/zeldhash-miner) | [facades/rust/README.md](facades/rust/README.md) |
| **TypeScript** | [`zeldhash-miner`](https://www.npmjs.com/package/zeldhash-miner) | [facades/typescript/README.md](facades/typescript/README.md) |

Both SDKs expose the same core functionality:
- `ZeldMiner` orchestrator with `mine_transaction()` / `mineTransaction()`
- GPU/CPU backend selection with automatic fallback
- Progress callbacks and pause/stop controls
- Identical transaction input/output structures

## Repository Structure

```
zeldhash-miner/
├── crates/                    # Core Rust building blocks
│   ├── core/                  # Domain logic (no_std, pure algorithms)
│   ├── gpu/                   # WebGPU backend
│   ├── wasm/                  # wasm-bindgen bindings
│   └── python-core/           # Placeholder for pyo3 wheel
├── facades/
│   ├── rust/                  # Crates.io SDK (zeldhash-miner)
│   └── typescript/            # npm SDK (zeldhash-miner) + WASM artifacts
├── examples/
│   └── web-demo/              # Vite demo consuming the TypeScript SDK
├── scripts/                   # Build & release automation
└── docs/                      # Architecture & release documentation
```

## Quick Start

### Rust

```rust
use zeldhash_miner::{MineParams, NetworkOption, TxInputDesc, TxOutputDesc, ZeldMiner, ZeldMinerOptions};

let miner = ZeldMiner::new(ZeldMinerOptions {
    network: NetworkOption::Mainnet,
    batch_size: 10_000,
    use_gpu: true,
    worker_threads: 1,
    sats_per_vbyte: 15,
})?;

let result = miner.mine_transaction(
    MineParams {
        inputs: vec![/* ... */],
        outputs: vec![/* ... */],
        target_zeros: 2,
        start_nonce: None,
        batch_size: None,
        distribution: None,
    },
    None,
    None,
)?;

println!("Found nonce {}", result.nonce);
```

→ See [facades/rust/README.md](facades/rust/README.md) for the full API reference.

### TypeScript

```ts
import { ZeldMiner } from "zeldhash-miner";

const miner = new ZeldMiner({
  network: "mainnet",
  batchSize: 10_000,
  useWebGPU: true,
  workerThreads: 4,
  satsPerVbyte: 12,
});

miner.on("found", ({ psbt, nonce }) => {
  console.log("nonce found", nonce.toString());
});

await miner.mineTransaction({
  inputs: [/* ... */],
  outputs: [/* ... */],
  targetZeros: 6,
  distribution: [600n, 300n, 100n], // optional ZELD distribution
});
```

→ See [facades/typescript/README.md](facades/typescript/README.md) for the full API reference.

## Building from Source

```bash
# Full build (WASM + TypeScript SDK + demo)
./scripts/build-all.sh

# WASM bindings only
./scripts/build-wasm.sh

# Run tests
cargo test -p zeldhash-miner-core
cargo test -p zeldhash-miner
npm test --prefix facades/typescript
```

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — Detailed design and component breakdown
- [Releasing](docs/RELEASING.md) — Release flows for crates.io and npm

## License

MIT
