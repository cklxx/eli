// Catch silently swallowed errors
import { logger } from "./log.js";
const log = logger("sidecar");

process.on("unhandledRejection", (reason) => {
  log.error("unhandled rejection", { reason: String(reason) });
});
process.on("uncaughtException", (err) => {
  log.error("uncaught exception", { err: String(err) });
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
export { startMcpServer } from "./mcp.js";
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
 * Load plugins and start an MCP server (stdio).
 *
 * This is the recommended entry point for external agents like Claude Code
 * or Cursor. No HTTP server is started — communication is over stdio.
 *
 * ```ts
 * import { createMcpSidecar } from "eli-sidecar";
 * await createMcpSidecar();
 * ```
 */
export async function createMcpSidecar(
  configOrPath?: Partial<SidecarConfig> | string,
): Promise<void> {
  let config: SidecarConfig;

  if (typeof configOrPath === "string" || configOrPath === undefined) {
    config = loadConfig(configOrPath);
  } else {
    const base = loadConfig();
    config = { ...base, ...configOrPath };
  }

  initBridge(config);
  await loadPlugins(config);

  const mcpLog = logger("mcp");
  mcpLog.info("loaded", { channels: registry.channels.size, tools: registry.tools.size });

  const { startMcpServer: startMcp } = await import("./mcp.js");
  await startMcp({ transport: "stdio", config });
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

  log.info("start", { eli_url: config.eli_url, port: config.port });
  log.info("plugins", { list: config.plugins.join(", ") || "(auto-discover)" });

  initBridge(config);
  await loadPlugins(config);

  log.info("registered", { channels: registry.channels.size, tools: registry.tools.size });

  const server = await startOutboundServer(config.port);
  await startChannels(config);

  log.info("ready");

  return {
    config,
    server,
    async callTool(name: string, params: Record<string, any> = {}) {
      const tool = registry.tools.get(name);
      if (!tool) throw new Error(`unknown tool: ${name}`);
      return tool.execute(`call_${Date.now()}`, params);
    },
    async stop() {
      log.info("shutting down");
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
  const mcpFlag = process.argv.includes("--mcp");

  if (mcpFlag) {
    // MCP mode: stdio transport, no HTTP server.
    createMcpSidecar().catch((err) => {
      logger("mcp").error("fatal", { err: String(err) });
      process.exit(1);
    });
  } else {
    // Normal mode: HTTP bridge server.
    createSidecar()
      .then((sidecar) => {
        const shutdown = async () => {
          await sidecar.stop();
          process.exit(0);
        };
        process.on("SIGINT", shutdown);
        process.on("SIGTERM", shutdown);

        // Detect parent death: eli pipes stdin to us. When the parent
        // process dies (killed, crashed), the pipe closes and we get 'end'.
        process.stdin.resume();
        process.stdin.on("end", () => {
          log.info("parent stdin closed, shutting down");
          shutdown();
        });
      })
      .catch((err) => {
        log.error("fatal", { err: String(err) });
        process.exit(1);
      });
  }
}
