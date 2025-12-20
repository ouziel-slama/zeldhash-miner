# zeldminer (TypeScript SDK)

TypeScript SDK for Zeldhash mining in the browser. This package wraps the Rust/WASM miner, exposes a simple API for mining vanity Bitcoin txids, and supports optional WebGPU acceleration.

> Requires Node.js 20+ and an ES module-capable bundler (Vite, webpack 5, etc.).

## Installation

```bash
npm install zeldminer
```

If you are developing locally from the monorepo, run the full pipeline to regenerate WASM bindings, library bundle, and demo:

```bash
./scripts/build-all.sh
```

## Quick Start

```ts
import { ZeldMiner, ZeldMinerErrorCode } from "zeldminer";

const miner = new ZeldMiner({
  network: "mainnet",
  batchSize: 10_000,
  useWebGPU: true, // automatically falls back to CPU if unavailable
  workerThreads: 4,
  satsPerVbyte: 12,
});

miner.on("progress", ({ hashRate, hashesProcessed }) => {
  console.log("hash rate", hashRate, "hashes", hashesProcessed.toString());
});

miner.on("found", ({ psbt, nonce, txid }) => {
  console.log("nonce found", nonce.toString());
  console.log("txid", txid);
  console.log("psbt", psbt);
});

miner.on("error", (err) => {
  if (err.code === ZeldMinerErrorCode.INVALID_INPUT) {
    console.error("bad inputs", err.details);
  } else {
    console.error(err);
  }
});

await miner.mineTransaction({
  inputs: [
    {
      txid: "1f81ad6116ac6045b5bc4941afc212456770ab389c05973c088f22063a2aff37",
      vout: 0,
      scriptPubKey: "0014ea9d20bfb938b2a0d778a5d8d8bc2aaff755c395",
      amount: 100000,
    },
  ],
  outputs: [
    { address: "bc1qa2wjp0ae8ze2p4mc5hvd30p24lm4tsu479mw0r", amount: 90000, change: false },
    { address: "bc1q...change", change: true }, // amount auto-calculated
  ],
  targetZeros: 6,
  distribution: [600n, 300n, 100n], // optional ZELD distribution
});
```

## API Reference

### `new ZeldMiner(options)`

Creates a new miner instance.

**Options:**
- `network`: `"mainnet" | "testnet" | "signet" | "regtest"`
- `batchSize`: `number` — Base batch size per worker
- `useWebGPU`: `boolean` — Attempt GPU backend (falls back to CPU)
- `workerThreads`: `number` — Worker count for CPU or GPU tasks
- `satsPerVbyte`: `number` — Fee rate for PSBT construction

### Methods

| Method | Description |
|--------|-------------|
| `mineTransaction(params): Promise<MineResult>` | Mines for a nonce and returns an unsigned PSBT |
| `pause(): void` | Pause the current mining session |
| `resume(): Promise<void>` | Resume after pause |
| `stop(): void` | Abort and reject the mining promise |
| `on(event, handler)` | Subscribe to miner events |
| `off(event, handler)` | Unsubscribe from events |

### Mining Parameters

```ts
interface MineParams {
  inputs: TxInput[];          // UTXOs to spend
  outputs: TxOutput[];        // Destinations (one must be change: true)
  targetZeros: number;        // Leading zero hex digits (1–32)
  startNonce?: bigint;        // Starting point (default 0n)
  batchSize?: number;         // Override instance batch size
  distribution?: bigint[];    // Optional ZELD distribution values
  signal?: AbortSignal;       // Abort controller signal
}

interface TxInput {
  txid: string;
  vout: number;
  scriptPubKey: string;
  amount: number;
}

interface TxOutput {
  address: string;
  amount?: number;  // Required unless change: true
  change: boolean;
}
```

### Events

| Event | Payload | Description |
|-------|---------|-------------|
| `progress` | `ProgressStats` | Periodic mining progress |
| `found` | `MineResult` | Nonce found, PSBT ready |
| `error` | `ZeldMinerError` | Mining error occurred |
| `stopped` | `void` | Mining was stopped |

```ts
interface ProgressStats {
  hashesProcessed: bigint;
  hashRate: number;
  elapsedMs?: number;
  lastNonce?: bigint;
  workerId?: number;
}

interface MineResult {
  psbt: string;        // Base64-encoded unsigned PSBT
  txid: string;        // Transaction ID with leading zeros
  nonce: bigint;       // Winning nonce
  attempts: bigint;    // Total hashes computed
  duration: number;    // Elapsed milliseconds
  hashRate: number;    // Hashes per second
}
```

## ZELD Distribution Mode

When `distribution` is provided, the OP_RETURN payload becomes:

```
OP_RETURN | push | "ZELD" | CBOR([distribution..., nonce])
```

The nonce is appended as the final CBOR element. Bitcoin nodes truncate the CBOR array to match the count of spendable outputs, so the nonce is automatically ignored by downstream wallets while still affecting the txid for mining.

```ts
await miner.mineTransaction({
  // ...
  distribution: [600n, 300n, 100n], // 3 outputs → 3 values
});
```

## Error Handling

```ts
import { ZeldMinerError, ZeldMinerErrorCode } from "zeldminer";

try {
  await miner.mineTransaction(params);
} catch (err) {
  if (err instanceof ZeldMinerError) {
    switch (err.code) {
      case ZeldMinerErrorCode.INSUFFICIENT_FUNDS:
        console.error("Not enough sats");
        break;
      case ZeldMinerErrorCode.INVALID_ADDRESS:
        console.error("Bad address:", err.message);
        break;
      case ZeldMinerErrorCode.MINING_ABORTED:
        console.error("Stopped by user");
        break;
      default:
        console.error(err);
    }
  }
}
```

### Error Codes

| Code | Description |
|------|-------------|
| `INVALID_ADDRESS` | Address parsing failed |
| `UNSUPPORTED_ADDRESS_TYPE` | Only P2WPKH and P2TR supported |
| `INSUFFICIENT_FUNDS` | Inputs don't cover outputs + fees |
| `NO_CHANGE_OUTPUT` | No output marked as change |
| `MULTIPLE_CHANGE_OUTPUTS` | More than one change output |
| `INVALID_INPUT` | Bad parameter |
| `WEBGPU_NOT_AVAILABLE` | WebGPU requested but unavailable |
| `WORKER_ERROR` | Internal worker failure |
| `MINING_ABORTED` | Mining was stopped |
| `DUST_OUTPUT` | Output below dust limit (546 sats) |

## Runtime Notes

- The WASM artifacts live in `node_modules/zeldminer/wasm`. Most modern bundlers copy them automatically because the SDK loads them via `new URL("./wasm/zeldhash_miner_wasm_bg.wasm", import.meta.url)`.
- If your bundler does not copy assets automatically, copy the `wasm/` folder to your public/static assets.
- WebGPU is optional. When `useWebGPU` is `true`, the miner auto-detects support and silently falls back to CPU.

## Build & Test

```bash
# Install dependencies
npm install

# Build the package
npm run build

# Run tests
npm test
```

## Publishing

The package is configured for the public npm registry with `"publishConfig.access": "public"`.

```bash
# From the monorepo root
./scripts/build-all.sh
cd facades/typescript && npm publish
```

Release flows are documented in [docs/RELEASING.md](../../docs/RELEASING.md).

## License

MIT
