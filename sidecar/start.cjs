#!/usr/bin/env node
/**
 * Sidecar entry point — uses jiti (same as openclaw) to load everything.
 * This ensures CJS singletons work correctly and openclaw/plugin-sdk resolves.
 *
 * In dev: loads src/index.ts directly via jiti.
 * Published: loads dist/index.js (src/ is not included in the package).
 */
const path = require("path");
const fs = require("fs");
const { createJiti } = require("jiti");
const jiti = createJiti(__filename, { interopDefault: true, tryNative: true });

const srcEntry = path.join(__dirname, "src", "index.ts");
if (fs.existsSync(srcEntry)) {
  jiti("./src/index.ts");
} else {
  jiti("./dist/index.js");
}
