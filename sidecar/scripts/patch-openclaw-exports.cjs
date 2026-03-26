/**
 * Patch openclaw package.json exports to add missing subpaths
 * that @openclaw/discord (and other newer plugins) depend on.
 *
 * openclaw ships the files but forgot to list them in the exports map.
 * This runs as postinstall to ensure plugins can resolve their imports.
 */
const fs = require("fs");
const path = require("path");

const pkgPath = path.join(__dirname, "..", "node_modules", "openclaw", "package.json");
if (!fs.existsSync(pkgPath)) process.exit(0);

const pkg = JSON.parse(fs.readFileSync(pkgPath, "utf-8"));
if (!pkg.exports) process.exit(0);

const missing = ["./plugin-sdk/compat"];
let patched = false;

for (const sub of missing) {
  if (pkg.exports[sub]) continue;
  const jsFile = path.join(__dirname, "..", "node_modules", "openclaw", "dist", sub.replace("./", "") + ".js");
  if (!fs.existsSync(jsFile)) continue;
  pkg.exports[sub] = {
    types: `./dist/${sub.replace("./", "")}.d.ts`,
    default: `./dist/${sub.replace("./", "")}.js`,
  };
  patched = true;
}

if (patched) {
  fs.writeFileSync(pkgPath, JSON.stringify(pkg, null, 2) + "\n");
  console.log("patched openclaw exports (added plugin-sdk/compat)");
}
