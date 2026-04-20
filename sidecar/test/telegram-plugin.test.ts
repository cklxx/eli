import { afterEach, describe, expect, it } from "bun:test";
import { parseIpFamily, resolveApiTimeoutMs, resolveIpFamily } from "../plugins/telegram.ts";

afterEach(() => {
  delete process.env.ELI_TELEGRAM_IP_FAMILY;
  delete process.env.SIDECAR_TELEGRAM_IP_FAMILY;
});

describe("telegram transport defaults", () => {
  it("defaults to ipv4", () => {
    expect(parseIpFamily(undefined)).toBe(4);
    expect(resolveIpFamily({})).toBe(4);
  });

  it("respects explicit ipv6 config", () => {
    expect(resolveIpFamily({ channels: { telegram: { ip_family: 6 } } })).toBe(6);
  });

  it("reads ip family from env", () => {
    process.env.ELI_TELEGRAM_IP_FAMILY = "6";
    expect(resolveIpFamily({})).toBe(6);
  });

  it("extends getUpdates timeout beyond long poll wait", () => {
    expect(resolveApiTimeoutMs("getUpdates", { timeout: 30 })).toBe(60_000);
    expect(resolveApiTimeoutMs("getUpdates", { timeout: 90 })).toBe(95_000);
  });
});
