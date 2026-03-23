import { readFileSync, existsSync } from "node:fs";
import { resolve, join } from "node:path";

export interface SidecarConfig {
  /** URL of eli's webhook channel (default: http://127.0.0.1:3100). */
  eli_url: string;
  /** Port for the sidecar's outbound server (default: 3101). */
  port: number;
  /** List of plugin npm package names or local paths. */
  plugins: string[];
  /** Per-channel account configuration, keyed by channel id. */
  channels: Record<string, ChannelAccountConfig>;
}

export interface ChannelAccountConfig {
  accounts: Record<string, Record<string, any>>;
  [key: string]: any;
}

const DEFAULTS: SidecarConfig = {
  eli_url: "http://127.0.0.1:3100",
  port: 3101,
  plugins: [],
  channels: {},
};

/**
 * Auto-discover OpenClaw plugins from installed npm dependencies.
 * Scans node_modules for packages that have an `openclaw` field in their
 * package.json (the standard OpenClaw plugin marker).
 */
function discoverPlugins(): string[] {
  const discovered: string[] = [];
  const nodeModules = resolve(process.cwd(), "node_modules");

  if (!existsSync(nodeModules)) return discovered;

  // Scan top-level and scoped (@org/) packages.
  const { readdirSync } = require("node:fs") as typeof import("node:fs");

  for (const entry of readdirSync(nodeModules, { withFileTypes: true })) {
    if (!entry.isDirectory() && !entry.isSymbolicLink()) continue;

    if (entry.name.startsWith("@")) {
      // Scoped package — scan one level deeper.
      const scopeDir = join(nodeModules, entry.name);
      for (const sub of readdirSync(scopeDir, { withFileTypes: true })) {
        if (!sub.isDirectory() && !sub.isSymbolicLink()) continue;
        const pkgName = `${entry.name}/${sub.name}`;
        if (isOpenClawPlugin(join(scopeDir, sub.name))) {
          discovered.push(pkgName);
        }
      }
    } else if (entry.name !== ".package-lock.json") {
      if (isOpenClawPlugin(join(nodeModules, entry.name))) {
        discovered.push(entry.name);
      }
    }
  }

  return discovered;
}

/** Check if a directory contains a package.json with an `openclaw` field. */
function isOpenClawPlugin(pkgDir: string): boolean {
  const pkgJsonPath = join(pkgDir, "package.json");
  if (!existsSync(pkgJsonPath)) return false;

  try {
    const raw = readFileSync(pkgJsonPath, "utf-8");
    const pkg = JSON.parse(raw);
    return pkg.openclaw != null;
  } catch {
    return false;
  }
}

export function loadConfig(path?: string): SidecarConfig {
  const configPath = path ?? resolve(process.cwd(), "sidecar.json");

  let fileConfig: Partial<SidecarConfig> = {};
  if (existsSync(configPath)) {
    const raw = readFileSync(configPath, "utf-8");
    fileConfig = JSON.parse(raw);
    console.log(`[config] loaded ${configPath}`);
  } else {
    console.log(`[config] no config file at ${configPath}, using auto-discovery`);
  }

  // Env overrides.
  const eli_url = process.env.SIDECAR_ELI_URL ?? fileConfig.eli_url ?? DEFAULTS.eli_url;
  const port = process.env.SIDECAR_PORT
    ? parseInt(process.env.SIDECAR_PORT, 10)
    : fileConfig.port ?? DEFAULTS.port;
  const channels = fileConfig.channels ?? DEFAULTS.channels;

  // Plugin resolution: explicit list > auto-discovery from node_modules.
  let plugins = fileConfig.plugins ?? [];
  if (plugins.length === 0) {
    plugins = discoverPlugins();
    if (plugins.length > 0) {
      console.log(`[config] auto-discovered plugins: ${plugins.join(", ")}`);
    }
  }

  // Channel config can also come from env vars:
  //   SIDECAR_LARK_APP_ID, SIDECAR_LARK_APP_SECRET → channels.lark.accounts.default
  for (const [key, value] of Object.entries(process.env)) {
    const match = key.match(/^SIDECAR_([A-Z]+)_(.+)$/);
    if (!match) continue;
    const channelId = match[1].toLowerCase();
    const configKey = match[2].toLowerCase();

    // Skip our own top-level env vars.
    if (channelId === "eli" || channelId === "port") continue;

    if (!channels[channelId]) {
      channels[channelId] = { accounts: {} };
    }
    if (!channels[channelId].accounts.default) {
      channels[channelId].accounts.default = {};
    }
    channels[channelId].accounts.default[configKey] = value;
  }

  return { eli_url, port, plugins, channels };
}
