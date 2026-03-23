// Catch silently swallowed errors
process.on("unhandledRejection", (reason) => {
  console.error("[UNHANDLED REJECTION]", reason);
});
process.on("uncaughtException", (err) => {
  console.error("[UNCAUGHT EXCEPTION]", err);
});

import { loadConfig, type SidecarConfig } from "./config.js";
import { initBridge, startOutboundServer } from "./bridge.js";
import { loadPlugins, startChannels, stopChannels, cleanupInstalledSkills } from "./runtime.js";
import { registry } from "./registry.js";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

export { loadConfig, type SidecarConfig } from "./config.js";
export { registry } from "./registry.js";
export type { ChannelAccountConfig } from "./config.js";
export type {
  ToolDefinition,
  ToolResult,
  ChannelPlugin,
  ChannelLifecycleHooks,
  SessionContext,
  InboundEnvelope,
  EliChannelMessage,
} from "./types.js";

export interface Sidecar {
  config: SidecarConfig;
  server: import("node:http").Server;
  /** Execute a tool by name. */
  callTool(name: string, params?: Record<string, any>): Promise<any>;
  stop(): Promise<void>;
}

/**
 * Create and start a sidecar instance.
 *
 * ```ts
 * import { createSidecar } from "eli-sidecar";
 *
 * const sidecar = await createSidecar();
 * // or with explicit config:
 * const sidecar = await createSidecar({
 *   eli_url: "http://localhost:3100",
 *   port: 3101,
 *   plugins: ["@larksuite/openclaw-lark"],
 *   channels: { feishu: { appId: "...", appSecret: "...", accounts: { default: { ... } } } },
 * });
 * ```
 */
export async function createSidecar(
  configOrPath?: Partial<SidecarConfig> | string,
): Promise<Sidecar> {
  let config: SidecarConfig;

  if (typeof configOrPath === "string" || configOrPath === undefined) {
    config = loadConfig(configOrPath);
  } else {
    // Merge with defaults.
    const base = loadConfig();
    config = { ...base, ...configOrPath };
  }

  console.log(`[sidecar] eli_url=${config.eli_url} port=${config.port}`);
  console.log(
    `[sidecar] plugins: ${config.plugins.join(", ") || "(auto-discover)"}`,
  );

  initBridge(config);
  await loadPlugins(config);

  console.log(
    `[sidecar] registered: ${registry.channels.size} channel(s), ${registry.tools.size} tool(s)`,
  );

  const server = await startOutboundServer(config.port);
  await startChannels(config);

  console.log("[sidecar] ready");

  return {
    config,
    server,
    async callTool(name: string, params: Record<string, any> = {}) {
      const tool = registry.tools.get(name);
      if (!tool) throw new Error(`unknown tool: ${name}`);
      return tool.execute(`call_${Date.now()}`, params);
    },
    async stop() {
      console.log("[sidecar] shutting down...");
      cleanupInstalledSkills();
      server.close();
      await stopChannels(config);
    },
  };
}

// ---------------------------------------------------------------------------
// CLI entry point — runs when executed directly (via start.cjs)
// ---------------------------------------------------------------------------

const isMainModule =
  typeof require !== "undefined" && require.main === module;
const isCLI =
  isMainModule || process.argv[1]?.endsWith("start.cjs") || false;

if (isCLI) {
  createSidecar()
    .then((sidecar) => {
      const shutdown = async () => {
        await sidecar.stop();
        process.exit(0);
      };
      process.on("SIGINT", shutdown);
      process.on("SIGTERM", shutdown);
    })
    .catch((err) => {
      console.error("[sidecar] fatal error:", err);
      process.exit(1);
    });
}
