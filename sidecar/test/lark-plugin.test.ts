/**
 * Real plugin test: loads @larksuite/openclaw-lark and verifies it registers
 * correctly through our SidecarPluginApi.
 *
 * This is the most important test — it validates that a real OpenClaw plugin
 * can be loaded by the sidecar and its register() call chain works end-to-end.
 */

import { describe, it, expect, beforeEach } from "bun:test";
import { registry } from "../src/registry.js";
import { SidecarPluginApi } from "../src/api.js";
import type { SidecarConfig } from "../src/config.js";

const testConfig: SidecarConfig = {
  eli_url: "http://127.0.0.1:3100",
  port: 3101,
  plugins: ["@larksuite/openclaw-lark"],
  channels: {
    "openclaw-lark": {
      accounts: {
        default: {
          appId: "cli_test_fake",
          appSecret: "test_fake_secret",
        },
      },
    },
  },
};

describe("@larksuite/openclaw-lark plugin", () => {
  beforeEach(() => {
    registry.channels.clear();
    registry.tools.clear();
    registry.hooks.length = 0;
  });

  it("loads and has expected default export shape", async () => {
    const mod = await import("@larksuite/openclaw-lark");
    const plugin = mod.default;

    expect(plugin.id).toBe("openclaw-lark");
    expect(plugin.name).toBe("Feishu");
    expect(typeof plugin.register).toBe("function");
  });

  it("register() populates channel registry", async () => {
    const mod = await import("@larksuite/openclaw-lark");
    const plugin = mod.default;
    const api = new SidecarPluginApi(plugin.id, testConfig);

    plugin.register(api);

    // Should have registered at least the lark/feishu channel.
    expect(registry.channels.size).toBeGreaterThanOrEqual(1);

    // Find the feishu channel — could be "openclaw-lark", "lark", or "feishu".
    const channelIds = Array.from(registry.channels.keys());
    const hasLarkChannel = channelIds.some(
      (id) => id.includes("lark") || id.includes("feishu"),
    );
    expect(hasLarkChannel).toBe(true);

    // The channel should have the expected adapters.
    const larkChannel = registry.channels.get(channelIds.find(
      (id) => id.includes("lark") || id.includes("feishu"),
    )!);
    expect(larkChannel).toBeDefined();
    expect(larkChannel!.meta).toBeDefined();
    expect(larkChannel!.meta.id).toBeTruthy();
    expect(larkChannel!.config).toBeDefined();
    expect(typeof larkChannel!.config.listAccountIds).toBe("function");
    expect(typeof larkChannel!.config.resolveAccount).toBe("function");
    expect(larkChannel!.capabilities).toBeDefined();
  });

  it("register() populates tool registry with lark tools", async () => {
    const mod = await import("@larksuite/openclaw-lark");
    const plugin = mod.default;
    const api = new SidecarPluginApi(plugin.id, testConfig);

    plugin.register(api);

    // Lark plugin registers doc/wiki/drive/bitable/task/calendar tools.
    const toolNames = Array.from(registry.tools.keys());
    expect(toolNames.length).toBeGreaterThan(0);

    // Log what was registered for visibility.
    console.log(`  registered ${registry.channels.size} channel(s): ${Array.from(registry.channels.keys()).join(", ")}`);
    console.log(`  registered ${registry.tools.size} tool(s): ${toolNames.slice(0, 10).join(", ")}${toolNames.length > 10 ? "..." : ""}`);
    console.log(`  registered ${registry.hooks.length} hook(s)`);
  });

  it("channel outbound adapter has sendText", async () => {
    const mod = await import("@larksuite/openclaw-lark");
    const plugin = mod.default;
    const api = new SidecarPluginApi(plugin.id, testConfig);

    plugin.register(api);

    const channelIds = Array.from(registry.channels.keys());
    const larkChannel = registry.channels.get(
      channelIds.find((id) => id.includes("lark") || id.includes("feishu"))!,
    );

    expect(larkChannel!.outbound).toBeDefined();
    expect(typeof larkChannel!.outbound!.sendText).toBe("function");
  });

  it("channel gateway adapter has startAccount", async () => {
    const mod = await import("@larksuite/openclaw-lark");
    const plugin = mod.default;
    const api = new SidecarPluginApi(plugin.id, testConfig);

    plugin.register(api);

    const channelIds = Array.from(registry.channels.keys());
    const larkChannel = registry.channels.get(
      channelIds.find((id) => id.includes("lark") || id.includes("feishu"))!,
    );

    expect(larkChannel!.gateway).toBeDefined();
    const startFn = larkChannel!.gateway!.startAccount ?? larkChannel!.gateway!.start;
    expect(typeof startFn).toBe("function");
  });
});

describe("auto-discovery", () => {
  it("discoverPlugins finds @larksuite/openclaw-lark in node_modules", async () => {
    // Import loadConfig and check that auto-discovery works.
    // We test by loading config without a config file — it should discover the lark plugin.
    const { loadConfig } = await import("../src/config.js");

    // Save and clear env to avoid interference.
    const saved = process.env.SIDECAR_ELI_URL;
    delete process.env.SIDECAR_ELI_URL;

    const config = loadConfig("/tmp/nonexistent-sidecar.json");

    if (saved) process.env.SIDECAR_ELI_URL = saved;

    expect(config.plugins.length).toBeGreaterThanOrEqual(1);
    expect(config.plugins).toContain("@larksuite/openclaw-lark");
  });
});
