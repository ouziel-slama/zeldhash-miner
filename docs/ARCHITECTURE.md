# Zeldhash Miner — Architecture Documentation

## Table of Contents

1. [Overview](#overview)
2. [Design Philosophy](#design-philosophy)
3. [Workspace Structure](#workspace-structure)
4. [Core Components](#core-components)
5. [Mining Algorithm](#mining-algorithm)
6. [Transaction Construction](#transaction-construction)
7. [GPU Acceleration](#gpu-acceleration)
8. [Data Flow](#data-flow)
9. [Error Handling](#error-handling)
10. [Build System](#build-system)
11. [Testing Strategy](#testing-strategy)
12. [Performance & Security](#performance--security)

---

## Overview

Zeldhash Miner is a **Bitcoin vanity transaction miner** that searches for transaction IDs (txids) with a target number of leading zero hex digits. The project provides native Rust and WebAssembly implementations with optional WebGPU acceleration.

The miner constructs a Bitcoin transaction with an `OP_RETURN` output containing a nonce. By iterating nonce values and computing the double-SHA256 hash, it finds a transaction whose txid meets the target difficulty.

### Key Features

- **Multi-platform**: Native Rust, WebAssembly, and browser environments
- **GPU acceleration**: WebGPU compute shaders for parallel hash computation
- **CPU parallelization**: Multi-threaded with Rayon (native) or Web Workers (browser)
- **no_std core**: Works in constrained environments
- **SegWit support**: P2WPKH and P2TR (Taproot) addresses
- **PSBT generation**: Produces unsigned PSBTs for wallet integration

---

## Design Philosophy

### Layered Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    Applications / Demos                     │
├─────────────────────────────────────────────────────────────┤
│   zeldhash-miner (TypeScript SDK)    │    zeldhash-miner (Rust)  │
├─────────────────────────────────────────────────────────────┤
│              zeldhash-miner-wasm (WASM bindings)            │
├─────────────────────────────────────────────────────────────┤
│   zeldhash-miner-gpu   │   —                                │
├─────────────────────────────────────────────────────────────┤
│                   zeldhash-miner-core                       │
│              (no_std, pure algorithms & types)              │
└─────────────────────────────────────────────────────────────┘
```

### Core Principles

1. **no_std compatibility**: Core uses only `alloc` for heap allocations
2. **Feature-gated complexity**: GPU and serde are opt-in
3. **Deterministic outputs**: Same inputs → same transaction structure
4. **Minimal nonce encoding**: Shortest possible byte representation
5. **Fallback resilience**: GPU failures gracefully fall back to CPU

---

## Workspace Structure

```
zeldhash-miner/
├── Cargo.toml                 # Workspace root (no package)
├── crates/
│   ├── core/                  # Core algorithms (no_std)
│   ├── gpu/                   # WebGPU backend
│   ├── (kernel removed)       # legacy GPU compute kernel (removed)
│   ├── wasm/                  # WASM bindings
│   └── python-core/           # Placeholder for pyo3 wheel
├── facades/
│   ├── rust/                  # Crates.io-facing orchestrator
│   └── typescript/            # npm SDK + WASM artifacts
├── examples/
│   └── web-demo/              # Vite demo consuming the TS facade
├── scripts/                   # Build automation
└── docs/                      # Documentation
```

---

## Core Components

### zeldhash-miner-core

Foundational crate with pure algorithms. `no_std` compatible with `alloc`.

| Module    | Purpose                                           |
|-----------|---------------------------------------------------|
| `address` | Bech32/Bech32m parsing (P2WPKH, P2TR)            |
| `hash`    | Double-SHA256 and target checking                 |
| `tx`      | Transaction serialization (legacy for txid)       |
| `psbt`    | PSBT construction with WITNESS_UTXO               |
| `fees`    | Virtual size and fee estimation                   |
| `nonce`   | Minimal big-endian nonce encoding                 |
| `miner`   | Mining loop, batch processing, templates          |

**Key Types:**

```rust
pub struct TxInput {
    pub txid: [u8; 32],
    pub vout: u32,
    pub script_pubkey: Vec<u8>,
    pub amount: u64,
    pub sequence: u32,
}

pub struct MiningTemplate {
    pub prefix: Vec<u8>,   // Serialized tx before nonce
    pub suffix: Vec<u8>,   // Serialized tx after nonce
    pub nonce_len: u8,
}

pub struct TransactionPlan {
    pub inputs: Vec<TxInput>,
    pub outputs: Vec<TxOutput>,       // in caller order, excludes OP_RETURN
    // If the change amount would be dust, the change output is omitted and the
    // sats flow to fees. Transactions may also be built with no change output.
    pub change_index: Option<usize>,  // index of the change output (if present)
    pub op_return_script: Vec<u8>,
    pub op_return_size: usize,       // Payload len (without OP_RETURN/pushdata)
    pub distribution: Option<Vec<u64>>, // ZELD distribution values (if any)
}
```

**Hash Target Verification** (txid is byte-reversed):

```rust
pub fn hash_meets_target(hash: &[u8; 32], target_zeros: u8) -> bool {
    let full_zero_bytes = (target_zeros / 2) as usize;
    if hash.iter().rev().take(full_zero_bytes).any(|b| *b != 0) {
        return false;
    }
    if target_zeros % 2 == 1 {
        let byte = hash[hash.len() - full_zero_bytes - 1];
        return byte >> 4 == 0;
    }
    true
}
```

### zeldhash-miner-gpu

WebGPU backend for parallel hash computation with WGSL shaders.

**Buffer Layout:**

| Binding | Name    | Type            | Purpose                     |
|---------|---------|-----------------|-----------------------------| 
| 0       | PREFIX  | storage<read>   | Tx prefix bytes             |
| 1       | SUFFIX  | storage<read>   | Tx suffix bytes             |
| 2       | PARAMS  | uniform         | Mining parameters           |
| 3       | RESULTS | storage<rw>     | Found nonces (atomic)       |

**Shader Core:**

```wgsl
@compute @workgroup_size(256, 1, 1)
fn main(@builtin(global_invocation_id) gid: vec3<u32>) {
    let idx = gid.x;
    if idx >= PARAMS.batch_size { return; }
    
    // Compute nonce, build message, double SHA256
    let hash = double_sha(nonce, msg_len);
    
    if meets_target(hash, PARAMS.target_zeros) {
        let pos = atomicAdd(&RESULTS.found_count, 1u);
        if pos < MAX_RESULTS { store_result(pos, nonce, hash); }
    }
}
```

**Batch Size Defaults:**

| Device Type    | Default Batch |
|----------------|---------------|
| DiscreteGpu    | 1,000,000     |
| IntegratedGpu  | 100,000       |
| Cpu            | 25,000        |

### zeldhash-miner-wasm

WASM bindings via `wasm-bindgen`:

| Function                | Purpose                            |
|-------------------------|------------------------------------|
| `validate_address`      | Check address validity             |
| `build_psbt`            | Create PSBT with nonce             |
| `build_mining_template` | Generate prefix/suffix             |
| `mine_batch_wasm`       | CPU mining                         |
| `mine_batch_gpu`        | GPU mining (feature-gated)         |
| `calibrate_batch_size`  | Auto-tune GPU batch size           |

### zeldhash-miner (Orchestrator)

Top-level Rust crate with control signals:

```rust
pub struct ZeldMiner {
    opts: ZeldMinerOptions,
    control: MiningControl,
    gpu_ctx: Option<GpuContext>,
}

impl ZeldMiner {
    pub fn mine_transaction(&self, params, on_progress, on_found) -> Result<MineResult>;
    pub fn stop(&self);
    pub fn pause(&self);
    pub fn resume(&self);
}
```

**Parallel CPU Mining** with Rayon:

```rust
fn mine_segment_cpu_parallel(...) -> Result<ControlledMineResult> {
    let subs = split_segment_for_workers(segment, workers)?;
    let found = Arc::new(AtomicBool::new(false));
    
    subs.into_par_iter().try_for_each(|sub| {
        mine_segment_cpu_controlled(..., Some(found.as_ref()))?;
        Ok(())
    })?;
}
```

### zeldhash-miner (TypeScript SDK)

Browser-facing API with Web Workers:

```typescript
class ZeldMiner {
    async mineTransaction(params: MineParams): Promise<MineResult>;
    stop(): void;
    pause(): void;
    async resume(): Promise<void>;
    on(event: 'progress' | 'found' | 'error' | 'stopped', handler): void;
}

class MiningCoordinator {
    private workers: WorkerState[];
    async start(params): Promise<void>;
}
```

**Stride-based Distribution:**

```
Workers: 4, BatchSize: 10,000
Worker 0: [0..10k),  [40k..50k), [80k..90k), ...
Worker 1: [10k..20k), [50k..60k), ...
Worker 2: [20k..30k), [60k..70k), ...
Worker 3: [30k..40k), [70k..80k), ...
Stride = BatchSize × Workers = 40,000
```

---

## Mining Algorithm

### Process

1. **Plan Transaction**: Validate inputs/outputs, calculate fees, compute change
2. **Build Template**: Serialize tx, split at OP_RETURN → `prefix + [nonce slot] + suffix`  
   - Slot is raw nonce bytes for legacy mode, or CBOR-encoded nonce appended to `ZELD || CBOR([distribution..., nonce])` when a distribution is provided.
3. **Mining Loop**: For each nonce:
   - Encode nonce as minimal big-endian bytes (legacy) or CBOR unsigned integer (distribution mode)
   - Concatenate: `prefix || encoded_nonce || suffix`
   - Compute `SHA256(SHA256(data))`
   - Check leading zeros
4. **Build PSBT**: Embed winning nonce and the full OP_RETURN payload (legacy or ZELD+CBOR), create unsigned PSBT

### Nonce Encoding

Two encoding modes are supported:

- **Legacy raw nonce (no distribution):** minimal big-endian encoding

| Nonce   | Bytes | Encoding    |
|---------|-------|-------------|
| 0       | 1     | `00`        |
| 255     | 1     | `ff`        |
| 256     | 2     | `01 00`     |
| 65,536  | 3     | `01 00 00`  |

**Segment Splitting:** Ranges crossing byte boundaries are auto-split:

```rust
split_nonce_segments(240, 32) → [
    { start: 240, size: 16, nonce_len: 1 },  // 240..255
    { start: 256, size: 16, nonce_len: 2 },  // 256..271
]
```

- **Distribution mode (ZELD + CBOR):** nonce is CBOR-encoded as an unsigned integer; lengths follow RFC 8949:

| Nonce           | CBOR length |
|-----------------|-------------|
| 0–23            | 1 byte      |
| 24–255          | 2 bytes     |
| 256–65,535      | 3 bytes     |
| 65,536–2^32−1   | 5 bytes     |
| ≥2^32           | 9 bytes     |

`split_nonce_segments_cbor` uses these lengths to avoid crossing CBOR byte-width boundaries.

---

## Transaction Construction

### Structure

```
Version (4B)        : 0x02000000
Input Count         : varint
Inputs[]            : serialized inputs
Output Count        : varint
Outputs[]           :
  ├─ User outputs   : destination payments
  ├─ OP_RETURN      : 0 sats, contains nonce
  └─ Change         : remaining funds - fee
Locktime (4B)       : 0x00000000
```

### OP_RETURN Format

- **Legacy nonce-only:** `OP_RETURN (0x6a) | PUSH_OPCODE | NONCE_BYTES`  
  Examples: `6a 01 00` (nonce 0), `6a 01 ff` (nonce 255), `6a 02 01 00` (nonce 256)

- **ZELD distribution mode:** `OP_RETURN | push | "ZELD" | CBOR([distribution..., nonce])`  
  - The nonce is the final CBOR element.  
  - Bitcoin nodes truncate the CBOR array to the count of spendable outputs, so the nonce is ignored by recipients while still affecting the txid for mining.  
  - Example for distribution `[600, 300, 100]` and nonce `42`:  
    `6a 0f 5a454c44 84 190258 19012c 1864 182a`

### Fee Calculation

```
vsize = (base_weight + witness_weight + 3) / 4

P2WPKH witness: ~108 bytes (1+1+72+1+33)
P2TR witness:   ~66 bytes  (1+1+64)
```

---

## GPU Acceleration

### Pipeline

```
Host (CPU)                              GPU
┌─────────────────┐                    ┌────────────────────┐
│ Upload buffers  │───────────────────►│ Compute Shader     │
│ • prefix[]      │                    │ (256 threads/group)│
│ • suffix[]      │                    │ • Build message    │
│ • params        │                    │ • Double SHA256    │
│                 │◄───────────────────│ • Check target     │
│ Read results    │                    │ • Store matches    │
└─────────────────┘                    └────────────────────┘
```

### Buffer Structures

```rust
#[repr(C)]
struct MiningParams {
    start_nonce_lo: u32,
    start_nonce_hi: u32,
    batch_size: u32,
    target_zeros: u32,
    prefix_len: u32,
    suffix_len: u32,
    nonce_len: u32,
    _padding: [u32; 5],
}

#[repr(C)]
struct ResultBuffer {
    found_count: u32,  // atomic
    _padding: [u32; 3],
    results: [ResultEntry; 8],
}
```

---

## Data Flow

### Native Rust

```
User → ZeldMiner::mine_transaction()
  ├─► parse_inputs/outputs
  ├─► split_nonce_segments() or split_nonce_segments_cbor() depending on distribution
  └─► for segment:
        ├─► plan_transaction()
        ├─► build_mining_template()
        ├─► [GPU] dispatch_mining_batch()
        └─► [CPU] mine_segment_cpu_parallel() → Rayon
```

### Browser/WASM

```
JavaScript → ZeldMiner.mineTransaction()
  ├─► selectBackend() → 'cpu' | 'gpu'
  ├─► TransactionBuilder.buildMiningTemplate(distribution?) → WASM (sets useCborNonce when distribution is present)
  └─► MiningCoordinator.start()
        ├─► spawnWorkers() → Web Workers
        └─► workers.postMessage({ type: 'mine' })
              └─► Worker: mineLoop()
                    ├─► wasm.mine_batch_gpu()
                    └─► wasm.mine_batch_wasm()
```

---

## Error Handling

### Error Codes

```typescript
enum ZeldMinerErrorCode {
    INVALID_ADDRESS,
    UNSUPPORTED_ADDRESS_TYPE,
    INSUFFICIENT_FUNDS,
    MULTIPLE_CHANGE_OUTPUTS,
    INVALID_INPUT,
    WEBGPU_NOT_AVAILABLE,
    WORKER_ERROR,
    MINING_ABORTED,
    DUST_OUTPUT,
}
```

### Rust Hierarchy

```rust
enum ZeldError {
    Address(AddressError),
    Fee(FeeError),
    Tx(TxError),
    Psbt(PsbtError),
}
```

---

## Build System

### Toolchain

```toml
# rust-toolchain.toml
channel = "1.82.0"
components = ["rust-src", "rustc-dev", "llvm-tools-preview", "rustfmt", "clippy"]
```

### Feature Matrix

| Crate              | Default           | Optional       |
|--------------------|-------------------|----------------|
| zeldhash-miner     | cpu, gpu, rayon   | serde          |
| zeldhash-miner-gpu | gpu (wgsl)        | —                               |
| zeldhash-miner-wasm| cpu, gpu          | —              |

### Commands

```bash
# Full build
./scripts/build-all.sh

# WASM only
./scripts/build-wasm.sh

# Tests
cargo test -p zeldhash-miner-core
cargo test -p zeldhash-miner
```

---

## Testing Strategy

### Categories

1. **Unit Tests**: Hash, nonce encoding, fees, address parsing
2. **Integration**: Full mining, PSBT generation, GPU/CPU equivalence
3. **Cross-Validation**: Compare with `bitcoin` crate

### Key Tests

```rust
#[test]
fn gpu_matches_cpu_when_available() {
    let cpu = cpu_mine(&batch);
    let gpu = dispatch_mining_batch(&ctx, &batch).await;
    assert_eq!(cpu, gpu);
}

#[test]
fn parses_psbt_with_bitcoin_crate() {
    let psbt = bitcoin::psbt::Psbt::deserialize(&psbt_bytes);
    assert!(psbt.is_ok());
}
```

---

## Performance & Security

### Hash Rates

| Backend            | Typical Rate     |
|--------------------|------------------|
| CPU (single core)  | 200-500 KH/s     |
| CPU (8 cores)      | 1-3 MH/s         |
| Integrated GPU     | 5-20 MH/s        |
| Discrete GPU       | 50-200+ MH/s     |

### Optimizations

- Template caching across same-length nonce batches
- GPU buffer pooling
- Fixed 256-thread workgroups
- Non-overlapping worker ranges
- Atomic early termination flag

### Security

- **No private keys**: Produces unsigned PSBTs only
- **Input validation**: Addresses, amounts, script lengths
- **Deterministic**: Reproducible outputs
- **Sandboxed**: GPU shaders run in browser sandbox

### Constants

| Constant           | Value     | Purpose                  |
|--------------------|-----------|--------------------------|
| DUST_LIMIT         | 310/330 sats | Minimum output (P2WPKH / P2TR) |
| MAX_TARGET_ZEROS   | 32        | Maximum difficulty       |
| WORKGROUP_SIZE     | 256       | GPU threads/group        |
| MAX_RESULTS        | 8         | GPU result capacity      |
| DEFAULT_SEQUENCE   | 0xFFFFFFFD| RBF-enabled              |

---

*Generated from zeldhash-miner source code analysis.*
