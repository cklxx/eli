/**
 * Integration test: full inbound/outbound bridge cycle.
 *
 * Spins up a mock eli server + the sidecar outbound server + a mock channel
 * plugin, then verifies end-to-end message flow in both directions.
 */

import { describe, it, expect, beforeAll, afterAll, beforeEach } from "bun:test";
import express from "express";
import type { Server } from "node:http";
import { registry } from "../src/registry.js";
import { initBridge, sendToEli, startOutboundServer } from "../src/bridge.js";
import { envelopeToEliMessage, parseOutboundTarget } from "../src/envelope.js";
import { beginPendingTyping, pendingTyping, sessionContexts } from "../src/runtime.js";
import type { ChannelPlugin, InboundEnvelope, EliChannelMessage } from "../src/types.js";

const MOCK_ELI_PORT = 13100;
const SIDECAR_PORT = 13101;

let mockEliServer: Server;
let sidecarServer: Server | undefined;
let capturedInbound: any[] = [];
let sentMessages: Array<{ text: string; chatId: string; accountId: string }> = [];
let cleanupCalls: Array<{ typingState: any; accountId: string }> = [];

// Mock channel plugin with outbound.sendText
const mockChannel: ChannelPlugin = {
  meta: { id: "mock-channel", label: "Mock Channel" },
  config: {
    listAccountIds: () => ["default"],
    resolveAccount: () => ({}),
  },
  capabilities: { chatTypes: ["direct"] },
  outbound: {
    deliveryMode: "direct",
    sendText: async ({ text, to, accountId }: any) => {
      sentMessages.push({ text, chatId: to, accountId });
      return { ok: true };
    },
  },
  lifecycle: {
    onOutboundReply: async ({ typingState, accountId }: any) => {
      cleanupCalls.push({ typingState, accountId });
    },
  },
};

beforeAll(async () => {
  capturedInbound = [];
  sentMessages = [];
  cleanupCalls = [];

  // Mock eli server — captures inbound POSTs.
  await new Promise<void>((resolve) => {
    const app = express();
    app.use(express.json());
    app.post("/inbound", (req, res) => {
      capturedInbound.push(req.body);
      res.status(200).json({ ok: true });
    });
    mockEliServer = app.listen(MOCK_ELI_PORT, () => resolve());
  });

  initBridge({
    eli_url: `http://127.0.0.1:${MOCK_ELI_PORT}`,
    port: SIDECAR_PORT,
    plugins: [],
    channels: {},
  });

  registry.channels.clear();
  registry.registerChannel(mockChannel);

  sidecarServer = await startOutboundServer(SIDECAR_PORT);
});

beforeEach(() => {
  capturedInbound = [];
  sentMessages = [];
  cleanupCalls = [];
  pendingTyping.clear();
  sessionContexts.clear();
  registry.tools.clear();
  mockChannel.lifecycle = {
    onOutboundReply: async ({ typingState, accountId }: any) => {
      cleanupCalls.push({ typingState, accountId });
    },
  };
  registry.channels.clear();
  registry.registerChannel(mockChannel);
});

afterAll(() => {
  mockEliServer?.close();
  sidecarServer?.close();
});

// ---------------------------------------------------------------------------
// Inbound: sidecar → eli
// ---------------------------------------------------------------------------

describe("inbound: sendToEli", () => {
  it("POSTs normalized ChannelMessage to eli /inbound", async () => {
    capturedInbound = [];

    await sendToEli({
      channel: "mock-channel",
      accountId: "default",
      senderId: "sender_1",
      senderName: "Bob",
      chatType: "direct",
      text: "Hello from mock",
    });

    expect(capturedInbound).toHaveLength(1);
    const msg = capturedInbound[0];
    expect(msg.session_id).toBe("mock-channel:default:sender_1");
    expect(msg.channel).toBe("webhook");
    expect(msg.content).toBe("Hello from mock");
    expect(msg.chat_id).toBe("sender_1");
    expect(msg.is_active).toBe(true);
    expect(msg.context.source_channel).toBe("mock-channel");
    expect(msg.context.sender_name).toBe("Bob");
  });
});

// ---------------------------------------------------------------------------
// Outbound: eli → sidecar → channel plugin
// ---------------------------------------------------------------------------

describe("outbound: eli callback", () => {
  it("routes response to the correct channel plugin's sendText", async () => {
    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/outbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_1",
        channel: "webhook",
        content: "Hello from eli",
        chat_id: "user_1",
        context: {
          source_channel: "mock-channel",
          account_id: "default",
          chat_type: "direct",
          sender_id: "user_1",
        },
        output_channel: "webhook",
      }),
    });

    expect(resp.status).toBe(200);
    expect(sentMessages).toHaveLength(1);
    expect(sentMessages[0].text).toBe("Hello from eli");
    expect(sentMessages[0].chatId).toBe("user_1");
    expect(sentMessages[0].accountId).toBe("default");
  });

  it("runs cleanup-only outbound without sending text", async () => {
    pendingTyping.set("mock-channel:default:user_cleanup", {
      typingState: { reaction: "thinking" },
      cfg: { channels: {} },
      accountId: "default",
    });

    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/outbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_cleanup",
        channel: "webhook",
        content: "",
        chat_id: "user_cleanup",
        context: {
          source_channel: "mock-channel",
          account_id: "default",
          chat_type: "direct",
          _eli_cleanup_only: true,
        },
        output_channel: "webhook",
      }),
    });

    const body = (await resp.json()) as any;
    expect(resp.status).toBe(200);
    expect(body.ok).toBe(true);
    expect(body.cleanup_only).toBe(true);
    expect(sentMessages).toHaveLength(0);
    expect(cleanupCalls).toHaveLength(1);
    expect(cleanupCalls[0].typingState).toEqual({ reaction: "thinking" });
    expect(cleanupCalls[0].accountId).toBe("default");
    expect(pendingTyping.has("mock-channel:default:user_cleanup")).toBe(false);
  });

  it("queues typing cleanup when outbound arrives before typing setup completes", async () => {
    let resolveTypingState: ((value: any) => void) | undefined;

    mockChannel.lifecycle = {
      onInboundMessage: async () => {
        return await new Promise((resolve) => {
          resolveTypingState = resolve;
        });
      },
      onOutboundReply: async ({ typingState, accountId }: any) => {
        cleanupCalls.push({ typingState, accountId });
      },
    };
    registry.channels.clear();
    registry.registerChannel(mockChannel);

    void beginPendingTyping({
      channelPlugin: mockChannel,
      cfg: { channels: {} },
      messageId: "msg_race",
      accountId: "default",
      sessionId: "mock-channel:default:user_race",
    });

    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/outbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_race",
        channel: "webhook",
        content: "Hello from eli",
        chat_id: "user_race",
        context: {
          source_channel: "mock-channel",
          account_id: "default",
          chat_type: "direct",
          sender_id: "user_race",
        },
        output_channel: "webhook",
      }),
    });

    expect(resp.status).toBe(200);
    expect(sentMessages).toHaveLength(1);
    expect(cleanupCalls).toHaveLength(0);
    expect(pendingTyping.has("mock-channel:default:user_race")).toBe(false);

    resolveTypingState?.({ reaction: "thinking" });
    await new Promise((resolve) => setTimeout(resolve, 10));

    expect(cleanupCalls).toHaveLength(1);
    expect(cleanupCalls[0].typingState).toEqual({ reaction: "thinking" });
    expect(cleanupCalls[0].accountId).toBe("default");
    expect(pendingTyping.has("mock-channel:default:user_race")).toBe(false);
  });

  it("returns 404 for unknown source_channel", async () => {
    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/outbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "x",
        channel: "webhook",
        content: "Hello",
        chat_id: "u",
        context: { source_channel: "nonexistent" },
        output_channel: "webhook",
      }),
    });
    expect(resp.status).toBe(404);
  });

  it("returns 400 when source_channel missing from context", async () => {
    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/outbound`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "x",
        channel: "webhook",
        content: "Hello",
        chat_id: "u",
        context: {},
        output_channel: "webhook",
      }),
    });
    expect(resp.status).toBe(400);
  });
});

// ---------------------------------------------------------------------------
// Tool execution notices
// ---------------------------------------------------------------------------

describe("tool execution", () => {
  it("sends top-level description back to the active channel before executing", async () => {
    registry.registerTool({
      name: "bash",
      description: "Run shell",
      parameters: {},
      execute: async () => ({ content: [{ type: "text", text: "ok" }] }),
    });

    sessionContexts.set("mock-channel:default:user_1", {
      channel: "mock-channel",
      messageId: "msg_1",
      chatId: "user_1",
      channelTarget: "route:user_1",
      accountId: "default",
      senderId: "user_1",
      chatType: "direct",
      cfg: {},
    });

    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/tools/bash`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_1",
        description: "查看当前工作目录",
        params: {
          cmd: "pwd",
        },
      }),
    });

    expect(resp.status).toBe(200);
    expect(sentMessages).toHaveLength(1);
    expect(sentMessages[0]).toEqual({
      text: "查看当前工作目录",
      chatId: "route:user_1",
      accountId: "default",
    });
  });

  it("lets channels override tool notice rendering", async () => {
    mockChannel.lifecycle = {
      onOutboundReply: async ({ typingState, accountId }: any) => {
        cleanupCalls.push({ typingState, accountId });
      },
      renderToolCallText: async (event) =>
        event.phase === "after" ? `完成 ${event.toolName}` : null,
    };
    registry.channels.clear();
    registry.registerChannel(mockChannel);

    registry.registerTool({
      name: "custom_tool",
      description: "Custom tool",
      parameters: {},
      execute: async () => ({ content: [{ type: "text", text: "done" }] }),
    });

    sessionContexts.set("mock-channel:default:user_2", {
      channel: "mock-channel",
      messageId: "msg_2",
      chatId: "user_2",
      accountId: "default",
      senderId: "user_2",
      chatType: "direct",
      cfg: {},
    });

    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/tools/custom_tool`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_2",
        params: { value: 1 },
      }),
    });

    expect(resp.status).toBe(200);
    expect(sentMessages).toHaveLength(1);
    expect(sentMessages[0]).toEqual({
      text: "完成 custom_tool",
      chatId: "user_2",
      accountId: "default",
    });
  });

  it("sends direct notices for builtin tools without clearing typing state", async () => {
    pendingTyping.set("mock-channel:default:user_3", {
      typingState: { reaction: "thinking" },
      cfg: { channels: {} },
      accountId: "default",
    });
    sessionContexts.set("mock-channel:default:user_3", {
      channel: "mock-channel",
      messageId: "msg_3",
      chatId: "user_3",
      channelTarget: "route:user_3",
      accountId: "default",
      senderId: "user_3",
      chatType: "direct",
      cfg: {},
    });

    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/notify`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({
        session_id: "mock-channel:default:user_3",
        text: "正在读取文件",
      }),
    });

    const body = (await resp.json()) as any;
    expect(resp.status).toBe(200);
    expect(body).toEqual({ ok: true, delivered: true });
    expect(sentMessages).toHaveLength(1);
    expect(sentMessages[0]).toEqual({
      text: "正在读取文件",
      chatId: "route:user_3",
      accountId: "default",
    });
    expect(cleanupCalls).toHaveLength(0);
    expect(pendingTyping.has("mock-channel:default:user_3")).toBe(true);
  });
});

// ---------------------------------------------------------------------------
// Envelope conversion
// ---------------------------------------------------------------------------

describe("envelope conversion", () => {
  it("maps InboundEnvelope to EliChannelMessage with correct session_id", () => {
    const msg = envelopeToEliMessage({
      channel: "lark",
      accountId: "bot1",
      senderId: "user_456",
      senderName: "Alice",
      chatType: "group",
      chatId: "group_789",
      groupLabel: "Team",
      text: "Hi team",
    });

    expect(msg.session_id).toBe("lark:bot1:group_789");
    expect(msg.channel).toBe("webhook");
    expect(msg.chat_id).toBe("group_789");
    expect(msg.content).toBe("Hi team");
    expect(msg.context.source_channel).toBe("lark");
    expect(msg.context.chat_type).toBe("group");
    expect(msg.context.group_label).toBe("Team");
  });

  it("parseOutboundTarget extracts routing fields", () => {
    const target = parseOutboundTarget({
      session_id: "lark:default:u1",
      channel: "webhook",
      content: "resp",
      chat_id: "u1",
      is_active: false,
      context: { source_channel: "lark", account_id: "default", chat_type: "direct" },
      output_channel: "webhook",
    });

    expect(target.sourceChannel).toBe("lark");
    expect(target.accountId).toBe("default");
    expect(target.chatId).toBe("u1");
    expect(target.chatType).toBe("direct");
  });
});

// ---------------------------------------------------------------------------
// Health check
// ---------------------------------------------------------------------------

describe("health check", () => {
  it("returns registered channels and tools", async () => {
    const resp = await fetch(`http://127.0.0.1:${SIDECAR_PORT}/health`);
    const body = (await resp.json()) as any;

    expect(resp.status).toBe(200);
    expect(body.status).toBe("ok");
    expect(body.channels).toContain("mock-channel");
  });
});
