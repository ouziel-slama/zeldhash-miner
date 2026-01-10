# zeldhash-miner (Rust SDK)

Native Rust orchestrator for Zeldhash mining. This crate provides a high-level API to mine Bitcoin transactions with vanity txids (leading zero hex digits).

Learn more at https://zeldhash.com.

The crate ships with CPU + WebGPU support enabled by default. The runtime `use_gpu` option decides whether to attempt the GPU path, with automatic fallback to CPU if GPU initialization fails.

## Installation

```toml
[dependencies]
zeldhash-miner = "0.3"
```

Or with specific features:

```toml
[dependencies]
zeldhash-miner = { version = "0.3", default-features = false, features = ["cpu", "serde"] }
```

## Quick Start

```rust
use zeldhash_miner::{
    MineParams, NetworkOption, TxInputDesc, TxOutputDesc, ZeldMiner, ZeldMinerOptions,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let miner = ZeldMiner::new(ZeldMinerOptions {
        network: NetworkOption::Mainnet,
        batch_size: 10_000,
        use_gpu: true, // set false to force CPU even when GPU is compiled in
        worker_threads: 4,
        sats_per_vbyte: 15,
    })?;

    let inputs = vec![TxInputDesc {
        txid: "1f81ad6116ac6045b5bc4941afc212456770ab389c05973c088f22063a2aff37".to_string(),
        vout: 0,
        script_pubkey: "0014ea9d20bfb938b2a0d778a5d8d8bc2aaff755c395".to_string(),
        amount: 120_000,
        sequence: None,
    }];

    let outputs = vec![
        TxOutputDesc {
            address: "bc1qa2wjp0ae8ze2p4mc5hvd30p24lm4tsu479mw0r".to_string(),
            amount: Some(60_000),
            change: false,
        },
        TxOutputDesc {
            address: "bc1q...change".to_string(),
            amount: None, // auto-calculated
            change: true,
        },
    ];

    let result = miner.mine_transaction(
        MineParams {
            inputs,
            outputs,
            target_zeros: 2,
            start_nonce: None,
            batch_size: None,
            distribution: None, // or Some(vec![600, 300, 100]) for ZELD mode
        },
        Some(|stats| {
            println!(
                "Progress: {} hashes, {:.2} H/s",
                stats.hashes_processed, stats.hash_rate
            );
        }),
        Some(|result| {
            println!("Found nonce {} → txid {}", result.nonce, result.txid);
        }),
    )?;

    println!("PSBT: {}", result.psbt);
    Ok(())
}
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `cpu`   | ✓       | CPU mining path |
| `gpu`   | ✓       | WebGPU backend (runtime fallback to CPU if unavailable) |
| `rayon` | ✓       | Parallelizes CPU batches across threads |
| `serde` | ✓       | Serialization for options, params, results, and errors |

### Feature Combinations

```bash
# Default: CPU + GPU + Rayon + Serde
cargo build -p zeldhash-miner

# CPU-only (skip GPU dependencies, faster compile)
cargo build -p zeldhash-miner --no-default-features --features "cpu serde"

# CPU + Rayon (multi-threaded CPU, no GPU)
cargo build -p zeldhash-miner --no-default-features --features "cpu rayon serde"
```

## API Reference

### `ZeldMiner::new(options) -> Result<ZeldMiner>`

Creates a new miner instance. If `use_gpu` is true but GPU initialization fails, the miner falls back to CPU silently.

**Options:**
- `network`: `NetworkOption::Mainnet | Testnet | Signet | Regtest`
- `batch_size`: Base batch size per iteration
- `use_gpu`: Attempt GPU backend
- `worker_threads`: Thread count for CPU mining (Rayon)
- `sats_per_vbyte`: Fee rate for PSBT construction

### `miner.mine_transaction(params, on_progress, on_found) -> Result<MineResult>`

Mines for a nonce producing the requested leading zeros and returns an unsigned PSBT.

**Params:**
- `inputs: Vec<TxInputDesc>` — UTXOs to spend
- `outputs: Vec<TxOutputDesc>` — Destinations (mark exactly one as `change: true`)
- `target_zeros: u8` — Leading zero hex digits (0–32)
- `start_nonce: Option<u64>` — Starting point (default 0)
- `batch_size: Option<u32>` — Override instance batch size
- `distribution: Option<Vec<u64>>` — ZELD distribution values (enables CBOR nonce encoding)

**Returns:**
- `psbt: String` — Base64-encoded unsigned PSBT
- `txid: String` — Transaction ID with leading zeros
- `nonce: u64` — Winning nonce
- `attempts: u128` — Total hashes computed
- `duration_ms: u128` — Elapsed time
- `hash_rate: f64` — Hashes per second

### Control Methods

```rust
miner.pause();   // Pause the current mining run
miner.resume();  // Resume after pause
miner.stop();    // Abort and reject the mining promise
```

> Each `ZeldMiner` instance coordinates a single mining run at a time. Create separate instances for concurrent searches.

## ZELD Distribution Mode

When `distribution` is provided, the OP_RETURN payload becomes:

```
OP_RETURN | push | "ZELD" | CBOR([distribution..., nonce])
```

The nonce is appended as the final CBOR element. Bitcoin nodes truncate the CBOR array to match the count of spendable outputs, so the nonce is automatically ignored by downstream wallets while still affecting the txid for mining.

```rust
let params = MineParams {
    // ...
    distribution: Some(vec![600, 300, 100]), // 3 outputs → 3 values
    // ...
};
```

## Error Handling

```rust
use zeldhash_miner::{ZeldMinerError, ZeldMinerErrorCode};

match miner.mine_transaction(params, None, None) {
    Ok(result) => println!("Success: {}", result.txid),
    Err(err) => match err.code {
        ZeldMinerErrorCode::InsufficientFunds => println!("Not enough sats"),
        ZeldMinerErrorCode::InvalidAddress => println!("Bad address: {}", err.message),
        ZeldMinerErrorCode::MiningAborted => println!("Stopped by user"),
        _ => println!("Error: {:?}", err),
    }
}
```

### Error Codes

| Code | Description |
|------|-------------|
| `InvalidAddress` | Address parsing failed |
| `UnsupportedAddressType` | Only P2WPKH and P2TR supported |
| `InsufficientFunds` | Inputs don't cover outputs + fees |
| `MultipleChangeOutputs` | More than one change output |
| `InvalidInput` | Bad parameter (txid, batch_size, etc.) |
| `WorkerError` | Internal GPU/worker failure |
| `MiningAborted` | Mining was stopped |
| `NoMatchingNonce` | Exhausted nonce range without finding match |
| `DustOutput` | Output below dust limit (310 sats P2WPKH / 330 sats P2TR) |

Notes:
- A change output is optional. If provided but the computed change would be below the dust limit, the change is omitted and the extra sats are counted as fees.
- You can also build transactions with no change output (e.g., sweeping a wallet).

## Build & Test

```bash
# Run tests with default features
cargo test -p zeldhash-miner

# CPU-only tests
cargo test -p zeldhash-miner --no-default-features --features "cpu serde"

# Check no_std compatibility of core
cargo check -p zeldhash-miner-core --no-default-features
```

## Notes

- GPU support is compiled by default; setting `use_gpu` to `false` keeps you on CPU.
- When `use_gpu` is `true`, the miner falls back to CPU if the WebGPU context cannot be created.
- The core crate (`zeldhash-miner-core`) is `no_std` compatible with `alloc`.
- Release flows are documented in [docs/RELEASING.md](../../docs/RELEASING.md).

## License

MIT

