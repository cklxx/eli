/**
 * Mock channel plugin for e2e testing.
 *
 * Registers as channel "test" in the sidecar registry. outbound.sendText()
 * captures messages to an in-memory array instead of sending them anywhere.
 * The test harness reads captured responses via /test/responses.
 */

import type {
  ChannelPlugin,
  OutboundTextParams,
  OutboundResult,
} from "../src/types.js";

export interface CapturedMessage {
  to: string;
  text: string;
  accountId: string;
  timestamp: number;
}

const captured: CapturedMessage[] = [];

export function getResponses(): CapturedMessage[] {
  return [...captured];
}

export function clearResponses(): void {
  captured.length = 0;
}

export const mockPlugin: ChannelPlugin = {
  meta: {
    id: "test",
    label: "Test Channel",
    blurb: "Mock channel for integration testing",
  },
  config: {
    listAccountIds: () => ["default"],
    resolveAccount: (_cfg, _accountId) => ({}),
  },
  capabilities: {
    chatTypes: ["direct", "group"],
  },
  outbound: {
    async sendText(params: OutboundTextParams): Promise<OutboundResult> {
      captured.push({
        to: params.to,
        text: params.text,
        accountId: params.accountId,
        timestamp: Date.now(),
      });
      return { ok: true };
    },
  },
};
