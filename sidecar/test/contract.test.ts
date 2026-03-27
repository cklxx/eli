import { describe, expect, it } from "bun:test";
import { readFileSync } from "node:fs";
import { join } from "node:path";

import {
  ELI_BRIDGE_CONTRACT_VERSION,
  outboundMediaItems,
  resolveBridgeContractVersion,
} from "../src/contract.js";
import type { EliChannelMessage } from "../src/types.js";

interface ToolRequestFixture {
  contract_version: string;
  params: Record<string, unknown>;
  description?: string;
  session_id?: string;
}

interface NoticeRequestFixture {
  contract_version: string;
  session_id: string;
  text: string;
}

describe("sidecar contract fixtures", () => {
  it("loads channel-message fixture and prefers top-level media", () => {
    const message = readFixture<EliChannelMessage>("channel-message.json");

    expect(resolveBridgeContractVersion(message.contract_version)).toBe(ELI_BRIDGE_CONTRACT_VERSION);
    expect(message.context.source_channel).toBe("mock-channel");
    expect(outboundMediaItems(message)[0]?.path).toBe("/tmp/fixture.png");
  });

  it("loads tool-request fixture", () => {
    const request = readFixture<ToolRequestFixture>("tool-request.json");

    expect(resolveBridgeContractVersion(request.contract_version)).toBe(ELI_BRIDGE_CONTRACT_VERSION);
    expect(request.description).toBe("同步飞书日程");
    expect(request.params.title).toBe("Weekly sync");
  });

  it("loads notice-request fixture", () => {
    const request = readFixture<NoticeRequestFixture>("notice-request.json");

    expect(resolveBridgeContractVersion(request.contract_version)).toBe(ELI_BRIDGE_CONTRACT_VERSION);
    expect(request.session_id).toBe("mock-channel:default:user_1");
    expect(request.text).toContain("飞书");
  });

  it("loads committed schema bundle", () => {
    const schema = readFixture<Record<string, unknown>>("eli-sidecar.schema.json");

    expect(schema.contract_version).toBe(ELI_BRIDGE_CONTRACT_VERSION);
    expect(schema.$id).toBe("https://eliagent.github.io/contracts/eli-sidecar-v1.schema.json");
  });
});

function readFixture<T>(name: string): T {
  const path = join(import.meta.dir, "..", "contracts", "v1", name);
  return JSON.parse(readFileSync(path, "utf8")) as T;
}
