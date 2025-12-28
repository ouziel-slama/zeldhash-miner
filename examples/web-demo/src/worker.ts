// Worker entry point - imports the actual worker from the zeldhash-miner package
// This file is served at /worker.js via vite.config.ts configuration
// The worker module has side effects (sets up message handlers) so we just import it
import "zeldhash-miner/worker";

