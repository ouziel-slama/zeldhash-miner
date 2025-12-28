# Integrating zeldhash-miner in Web Applications

This document explains how to integrate the `zeldhash-miner` package in various web frameworks. The package contains WebAssembly (WASM) files and a Web Worker that must be served as static assets.

## The Challenge

Unlike regular npm packages, `zeldhash-miner` includes:

- **WASM files** (`zeldhash_miner_wasm.js`, `zeldhash_miner_wasm_bg.wasm`) - Binary code compiled from Rust
- **Web Worker** (`worker.js`) - Runs mining in a separate thread

These files cannot be bundled into your JavaScript bundle. They must be served as **static files** accessible via HTTP requests (e.g., `fetch('/wasm/zeldhash_miner_wasm.js')`).

## Quick Start: CDN (Recommended)

The simplest solution is to use a CDN. No configuration required:

```typescript
// Set this BEFORE importing zeldhash-miner
globalThis.__ZELDMINER_WASM_BASE__ = 'https://cdn.jsdelivr.net/npm/zeldhash-miner@0.1.0/wasm/';

import { ZeldMiner } from 'zeldhash-miner';
```

**Pros:**
- Works with any framework (React, Next.js, Vue, Svelte, vanilla JS)
- No build configuration needed
- Global CDN caching for better performance

**Cons:**
- Requires internet connection
- Depends on CDN availability

---

## Self-Hosting: Framework-Specific Setup

If you prefer to host the WASM files yourself, follow the instructions for your framework.

### Vite (React, Vue, Svelte, vanilla)

**Option A: Manual copy to public folder**

1. Copy the assets after install:

```bash
# Create public/wasm directory
mkdir -p public/wasm

# Copy WASM files
cp node_modules/zeldhash-miner/wasm/zeldhash_miner_wasm.js public/wasm/
cp node_modules/zeldhash-miner/wasm/zeldhash_miner_wasm_bg.wasm public/wasm/

# Copy worker
cp node_modules/zeldhash-miner/dist/worker.js public/
```

2. Add to your `package.json` scripts:

```json
{
  "scripts": {
    "postinstall": "mkdir -p public/wasm && cp node_modules/zeldhash-miner/wasm/* public/wasm/ && cp node_modules/zeldhash-miner/dist/worker.js public/"
  }
}
```

**Option B: Use vite-plugin-static-copy**

```bash
npm install -D vite-plugin-static-copy
```

```typescript
// vite.config.ts
import { defineConfig } from 'vite';
import { viteStaticCopy } from 'vite-plugin-static-copy';

export default defineConfig({
  plugins: [
    viteStaticCopy({
      targets: [
        {
          src: 'node_modules/zeldhash-miner/wasm/*',
          dest: 'wasm'
        },
        {
          src: 'node_modules/zeldhash-miner/dist/worker.js',
          dest: '.'
        }
      ]
    })
  ]
});
```

---

### Next.js (App Router & Pages Router)

Next.js uses Webpack, not Vite. You need to copy assets to the `public/` folder.

**Option A: postinstall script**

```json
{
  "scripts": {
    "postinstall": "mkdir -p public/wasm && cp node_modules/zeldhash-miner/wasm/* public/wasm/ && cp node_modules/zeldhash-miner/dist/worker.js public/"
  }
}
```

**Option B: Use copy-webpack-plugin**

```bash
npm install -D copy-webpack-plugin
```

```javascript
// next.config.js
const CopyPlugin = require('copy-webpack-plugin');
const path = require('path');

module.exports = {
  webpack: (config, { isServer }) => {
    if (!isServer) {
      config.plugins.push(
        new CopyPlugin({
          patterns: [
            {
              from: path.join(__dirname, 'node_modules/zeldhash-miner/wasm'),
              to: path.join(__dirname, 'public/wasm'),
            },
            {
              from: path.join(__dirname, 'node_modules/zeldhash-miner/dist/worker.js'),
              to: path.join(__dirname, 'public/worker.js'),
            },
          ],
        })
      );
    }
    return config;
  },
};
```

**Important for Next.js:** You may also need to configure headers for WASM files:

```javascript
// next.config.js
module.exports = {
  async headers() {
    return [
      {
        source: '/wasm/:path*',
        headers: [
          { key: 'Cross-Origin-Opener-Policy', value: 'same-origin' },
          { key: 'Cross-Origin-Embedder-Policy', value: 'require-corp' },
        ],
      },
    ];
  },
};
```

---

### Create React App (CRA)

CRA doesn't allow Webpack configuration without ejecting. Use the postinstall approach:

```json
{
  "scripts": {
    "postinstall": "mkdir -p public/wasm && cp node_modules/zeldhash-miner/wasm/* public/wasm/ && cp node_modules/zeldhash-miner/dist/worker.js public/"
  }
}
```

Or use `react-app-rewired` with `copy-webpack-plugin`.

---

### Vanilla HTML/JS

Simply copy the files to your server's static directory and include them:

```html
<script type="module">
  // Set WASM base before importing
  globalThis.__ZELDMINER_WASM_BASE__ = '/wasm/';
</script>
<script type="module" src="your-app.js"></script>
```

---

## Configuration Options

### WASM Base URL

The package **automatically bootstraps** `globalThis.__ZELDMINER_WASM_BASE__` to `/wasm/` relative to your application's origin. This means WASM files are fetched from `https://your-app.com/wasm/` rather than from inside `node_modules/`, which resolves 404 errors in Vite dev mode.

The resolution order is:

1. `globalThis.__ZELDMINER_WASM_BASE__` - Automatically set to `/wasm/` on your app's origin (or you can override it before import)
2. `import.meta.env.VITE_ZELDMINER_WASM_BASE` - Vite environment variable
3. `import.meta.env.BASE_URL + 'wasm/'` - Vite base URL + wasm/
4. `/wasm/` - Default fallback

**Note:** Since the bootstrap runs automatically, you only need to copy the WASM/worker files to your `public/` folder. No additional configuration is required unless you want a custom path.

**Example: Custom path**

```typescript
// Set before any zeldhash-miner imports
globalThis.__ZELDMINER_WASM_BASE__ = '/assets/zeldhash-miner/wasm/';
```

**Example: Vite environment variable**

```bash
# .env
VITE_ZELDMINER_WASM_BASE=/custom/wasm/path/
```

---

## Troubleshooting

### Error: "Failed to load resource: 404"

The WASM files are not being served. Check that:

1. Files exist in your public folder (`public/wasm/`)
2. Your dev server is running and serving the public folder
3. The WASM base URL is correctly configured

### Error: "mine_batch_gpu is unavailable"

GPU bindings are missing. Rebuild the WASM bundle with `./scripts/build-wasm.sh` to ensure GPU features are compiled, or reinstall the npm package to get the GPU-enabled artifacts.

### Error: "outputs[0].amount must be at least 546 sats"

You're using an outdated WASM build. The current version enforces dust per address type (310 sats for P2WPKH, 330 sats for P2TR). Rebuild or update the package.

### Cross-Origin Issues

Web Workers and WASM require proper CORS headers. Ensure your server sends:

```
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

For Vite dev server, this is usually automatic. For production, configure your server or CDN.

---

## File Structure Reference

After setup, your project should have:

```
your-project/
├── public/
│   ├── wasm/
│   │   ├── zeldhash_miner_wasm.js      # WASM JS bindings
│   │   └── zeldhash_miner_wasm_bg.wasm # WASM binary
│   └── worker.js                        # Mining Web Worker
├── src/
│   └── ...
└── package.json
```

These files will be served at:
- `http://localhost:3000/wasm/zeldhash_miner_wasm.js`
- `http://localhost:3000/wasm/zeldhash_miner_wasm_bg.wasm`
- `http://localhost:3000/worker.js`

---

## Future Improvements

We're working on:

1. **Official Vite plugin** (`zeldhash-miner/vite-plugin`) - Zero-config setup for Vite projects
2. **Official Next.js plugin** (`zeldhash-miner/next-plugin`) - Zero-config setup for Next.js
3. **Default CDN** - WASM served from CDN by default, no configuration needed

Contributions welcome!

