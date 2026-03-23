// Catch silently swallowed errors
process.on("unhandledRejection", (reason) => {
  console.error("[UNHANDLED REJECTION]", reason);
});
process.on("uncaughtException", (err) => {
  console.error("[UNCAUGHT EXCEPTION]", err);
});

import { loadConfig } from "./config.js";
import { initBridge, startOutboundServer } from "./bridge.js";
import { loadPlugins, startChannels, stopChannels } from "./runtime.js";
import { registry } from "./registry.js";

async function main() {
  console.log("[sidecar] starting...");

  // 1. Load configuration.
  const config = loadConfig();
  console.log(`[sidecar] eli_url=${config.eli_url} port=${config.port}`);
  console.log(`[sidecar] plugins: ${config.plugins.join(", ") || "(none)"}`);

  // 2. Initialize the bridge (sets the eli URL for inbound POSTs).
  initBridge(config);

  // 3. Load all plugins — this populates the registry.
  await loadPlugins(config);

  const channelCount = registry.channels.size;
  const toolCount = registry.tools.size;
  console.log(`[sidecar] registered: ${channelCount} channel(s), ${toolCount} tool(s)`);

  // 4. Start the outbound HTTP server (eli calls back here with responses).
  const server = await startOutboundServer(config.port);

  // 5. Start all channel gateways (begins receiving messages from platforms).
  await startChannels(config);

  console.log("[sidecar] ready");

  // Graceful shutdown on SIGINT / SIGTERM.
  const shutdown = async () => {
    console.log("\n[sidecar] shutting down...");
    server.close();
    await stopChannels(config);
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  console.error("[sidecar] fatal error:", err);
  process.exit(1);
});
