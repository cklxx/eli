/**
 * Config loading edge case tests.
 */

import { describe, it, expect, beforeEach, afterEach } from "bun:test";
import { writeFileSync, mkdirSync, rmSync } from "node:fs";
import { join, resolve } from "node:path";
import { loadConfig } from "../src/config.js";

const TEST_DIR = resolve(import.meta.dir, "../.test-config-tmp");
const TEST_CONFIG = join(TEST_DIR, "sidecar.json");

beforeEach(() => {
  mkdirSync(TEST_DIR, { recursive: true });
  // Clear env vars that could interfere.
  delete process.env.SIDECAR_ELI_URL;
  delete process.env.SIDECAR_PORT;
  delete process.env.SIDECAR_DISCORD_TOKEN;
  delete process.env.SIDECAR_ALLOW_REMOTE_CHANNEL_SHUTDOWN;
});

afterEach(() => {
  rmSync(TEST_DIR, { recursive: true, force: true });
  delete process.env.SIDECAR_ELI_URL;
  delete process.env.SIDECAR_PORT;
  delete process.env.SIDECAR_DISCORD_TOKEN;
  delete process.env.SIDECAR_ALLOW_REMOTE_CHANNEL_SHUTDOWN;
});

describe("loadConfig", () => {
  it("returns defaults when config file does not exist", () => {
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.eli_url).toBe("http://127.0.0.1:3100");
    expect(config.port).toBe(3101);
    // plugins may be auto-discovered from node_modules.
    expect(Array.isArray(config.plugins)).toBe(true);
    expect(config.allow_remote_shutdown).toBe(false);
  });

  it("loads and merges valid JSON config file", () => {
    writeFileSync(TEST_CONFIG, JSON.stringify({
      eli_url: "http://custom:9000",
      port: 4000,
      plugins: ["my-plugin"],
    }));
    const config = loadConfig(TEST_CONFIG);
    expect(config.eli_url).toBe("http://custom:9000");
    expect(config.port).toBe(4000);
    expect(config.plugins).toEqual(["my-plugin"]);
  });

  it("falls back to defaults on malformed JSON", () => {
    writeFileSync(TEST_CONFIG, "{ invalid json }}");
    const config = loadConfig(TEST_CONFIG);
    // Should not throw, should use defaults.
    expect(config.eli_url).toBe("http://127.0.0.1:3100");
    expect(config.port).toBe(3101);
  });

  it("SIDECAR_PORT=abc falls back to file/default port", () => {
    process.env.SIDECAR_PORT = "abc";
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.port).toBe(3101);
  });

  it("SIDECAR_PORT=0 is treated as valid", () => {
    process.env.SIDECAR_PORT = "0";
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.port).toBe(0);
  });

  it("SIDECAR_ELI_URL overrides file config", () => {
    writeFileSync(TEST_CONFIG, JSON.stringify({ eli_url: "http://from-file:3100" }));
    process.env.SIDECAR_ELI_URL = "http://from-env:3100";
    const config = loadConfig(TEST_CONFIG);
    expect(config.eli_url).toBe("http://from-env:3100");
  });

  it("env var channel config creates account structure", () => {
    process.env.SIDECAR_DISCORD_TOKEN = "my-token";
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.channels.discord).toBeDefined();
    expect(config.channels.discord.token).toBe("my-token");
    expect(config.channels.discord.accounts.default.token).toBe("my-token");
  });

  it("skips eli and port prefixed env vars as channel config", () => {
    process.env.SIDECAR_ELI_URL = "http://test:3100";
    process.env.SIDECAR_PORT = "5000";
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.channels.eli).toBeUndefined();
    expect(config.channels.port).toBeUndefined();
  });

  it("allow_remote_shutdown reads from env", () => {
    process.env.SIDECAR_ALLOW_REMOTE_CHANNEL_SHUTDOWN = "1";
    const config = loadConfig(join(TEST_DIR, "nonexistent.json"));
    expect(config.allow_remote_shutdown).toBe(true);
  });
});
