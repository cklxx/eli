/**
 * Test-mode sidecar entry point.
 *
 * Registers a mock "test" channel plugin and mounts /test/* endpoints,
 * then starts the sidecar normally. Usage:
 *
 *   bun sidecar/test/start-test.ts
 *
 * Requires eli gateway running on :3100.
 */

import { logger } from "../src/log.js";
import { loadConfig } from "../src/config.js";
import { initBridge, startOutboundServer } from "../src/bridge.js";
import { loadPlugins, startChannels } from "../src/runtime.js";
import { registry } from "../src/registry.js";
import { mockPlugin } from "./mock-channel-plugin.js";
import { mountTestEndpoints } from "./test-endpoints.js";
import express from "express";

const log = logger("test-sidecar");

async function main() {
  const config = loadConfig();

  // Override: point to local gateway
  config.eli_url = process.env.SIDECAR_ELI_URL ?? "http://127.0.0.1:3100";
  config.port = Number(process.env.SIDECAR_PORT ?? 3101);

  log.info("test mode", { eli_url: config.eli_url, port: config.port });

  initBridge(config);

  // Register mock channel BEFORE loading real plugins
  registry.channels.set("test", mockPlugin);
  log.info("registered mock channel: test");

  // Load real plugins too (if any configured)
  await loadPlugins(config);

  log.info("channels", { list: Array.from(registry.channels.keys()) });
  log.info("tools", { count: registry.tools.size });

  // Start the normal outbound server.
  const server = await startOutboundServer(config.port);

  // Mount test endpoints on the same server via a separate express app
  // Actually, startOutboundServer creates its own express app.
  // We need a different approach: mount test endpoints on a separate port.
  const TEST_PORT = Number(process.env.TEST_PORT ?? config.port + 10); // 3111
  const testApp = express();
  testApp.use(express.json());
  mountTestEndpoints(testApp);

  testApp.listen(TEST_PORT, () => {
    log.info("test endpoints listening", { port: TEST_PORT });
  });

  // Start real channel gateways (if any)
  await startChannels(config);

  log.info("test sidecar ready", {
    outbound_port: config.port,
    test_port: TEST_PORT,
    channels: Array.from(registry.channels.keys()),
  });

  // Graceful shutdown
  const shutdown = () => {
    log.info("shutting down");
    server.close();
    process.exit(0);
  };
  process.on("SIGINT", shutdown);
  process.on("SIGTERM", shutdown);
}

main().catch((err) => {
  log.error("fatal", { err: String(err) });
  process.exit(1);
});
