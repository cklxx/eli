#!/usr/bin/env node
/**
 * Sidecar entry point — uses jiti (same as openclaw) to load everything.
 * This ensures CJS singletons work correctly and openclaw/plugin-sdk resolves.
 */
const { createJiti } = require("jiti");
const jiti = createJiti(__filename, { interopDefault: true, tryNative: true });
jiti("./src/index.ts");
