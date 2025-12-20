# Zeldhash Miner — Web Demo

Interactive browser demo showcasing the [zeldminer](../../facades/typescript/README.md) TypeScript SDK. Mine Bitcoin vanity transactions with leading-zero txids directly in your browser.

## Features

- Real-time hash rate visualization
- WebGPU acceleration with automatic CPU fallback
- UTXO and output management UI
- PSBT export (copy/download)
- Optional ZELD distribution support

## Running the Demo

### From the Monorepo Root

```bash
# Build everything (WASM, TypeScript SDK, demo)
./scripts/build-all.sh

# Start the dev server
cd examples/web-demo
npm run dev
```

### Standalone

```bash
cd examples/web-demo

# Install dependencies
npm install

# Start the dev server
npm run dev
```

The demo will be available at `http://localhost:5173`.

> **Note:** The demo requires COOP/COEP headers for SharedArrayBuffer (needed for Web Workers). The Vite config already sets these headers.

## Building for Production

```bash
npm run build
```

The built files will be in `dist/`.

## Usage

1. **Configure Network & Fees**: Select the Bitcoin network and set the fee rate (sats/vB)
2. **Add UTXOs**: Enter the transaction inputs you want to spend
3. **Add Outputs**: Define destination addresses (mark exactly one as "change")
4. **Set Target**: Choose the number of leading zeros for the txid
5. **Start Mining**: Click "Start mining" and wait for a valid nonce
6. **Export PSBT**: Copy or download the unsigned PSBT for signing

## Technical Details

- Built with Vite + TypeScript
- Consumes `zeldminer` from `../../facades/typescript`
- Uses `bitcoinjs-lib` for PSBT parsing
- WebGPU batch size: 65,536 hashes
- CPU batch size: 256 hashes

## License

MIT

