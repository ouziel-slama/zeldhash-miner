import { Buffer } from "buffer";

// Ensure a global Buffer exists for dependencies that expect Node globals.
(globalThis as { Buffer?: typeof Buffer }).Buffer ??= Buffer;

