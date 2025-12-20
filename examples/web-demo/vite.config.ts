import { defineConfig } from "vite";
import { resolve } from "node:path";

export default defineConfig({
  server: {
    headers: {
      "Cross-Origin-Opener-Policy": "same-origin",
      "Cross-Origin-Embedder-Policy": "require-corp",
    },
    fs: {
      allow: [
        resolve(__dirname, ".."),
        resolve(__dirname, "../../facades/typescript"),
        resolve(__dirname, "../.."),
      ],
    },
  },
  resolve: {
    alias: {
      buffer: "buffer",
      zeldminer: resolve(__dirname, "../../facades/typescript/src"),
    },
  },
  optimizeDeps: {
    include: ["buffer", "bitcoinjs-lib"],
  },
  build: {
    target: "esnext",
  },
  worker: {
    format: "es",
  },
});

