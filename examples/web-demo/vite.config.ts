import { defineConfig, type Plugin } from "vite";
import { resolve } from "node:path";

/**
 * Plugin to serve the worker module at /worker.js during development.
 * The MiningCoordinator expects to load the worker from this path.
 */
function workerRedirectPlugin(): Plugin {
  return {
    name: "worker-redirect",
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        if (req.url === "/worker.js") {
          // Redirect to the worker entry point that Vite can transform
          req.url = "/src/worker.ts";
        }
        next();
      });
    },
  };
}

export default defineConfig({
  plugins: [workerRedirectPlugin()],
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
      "zeldhash-miner": resolve(__dirname, "../../facades/typescript/src"),
    },
  },
  optimizeDeps: {
    include: ["buffer", "bitcoinjs-lib"],
  },
  build: {
    target: "esnext",
    rollupOptions: {
      input: {
        main: resolve(__dirname, "index.html"),
        worker: resolve(__dirname, "src/worker.ts"),
      },
      output: {
        entryFileNames: (chunkInfo) => {
          // Output worker.js at root level for the coordinator to find it
          if (chunkInfo.name === "worker") {
            return "worker.js";
          }
          return "assets/[name]-[hash].js";
        },
      },
    },
  },
  worker: {
    format: "es",
  },
});

