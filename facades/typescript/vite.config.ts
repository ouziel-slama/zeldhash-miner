import { defineConfig } from "vite";
import dts from "vite-plugin-dts";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

export default defineConfig({
  // Use relative paths so worker/script URLs stay relative in the published build.
  base: "./",
  plugins: [
    dts({
      rollupTypes: true,
      insertTypesEntry: true,
      outDir: "dist",
    }),
  ],
  build: {
    lib: {
      entry: {
        index: resolve(__dirname, "src/index.ts"),
        worker: resolve(__dirname, "src/worker.ts"),
      },
      name: "ZeldMiner",
      formats: ["es"],
      fileName: (_, entryName) => `${entryName}.js`,
    },
    assetsDir: ".",
    target: "es2022",
    sourcemap: true,
    rollupOptions: {
      // Do not bundle peer deps; let the consumer manage them.
      external: ["bitcoinjs-lib"],
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name].[ext]",
      },
    },
  },
  worker: {
    format: "es",
    rollupOptions: {
      output: {
        entryFileNames: "worker.js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name].[ext]",
      },
    },
  },
});

