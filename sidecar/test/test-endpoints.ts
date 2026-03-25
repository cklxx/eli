/**
 * Test-only HTTP endpoints mounted on the sidecar express app.
 *
 * POST /test/inbound   — accepts InboundEnvelope, feeds it through sendToEli()
 * GET  /test/responses  — returns captured outbound responses from mock plugin
 * POST /test/clear      — clears captured responses
 * GET  /test/health     — returns health + channel/tool counts
 */

import type { Express } from "express";
import type { InboundEnvelope } from "../src/types.js";
import { sendToEli } from "../src/bridge.js";
import { registry } from "../src/registry.js";
import { getResponses, clearResponses } from "./mock-channel-plugin.js";

export function mountTestEndpoints(app: Express): void {
  // Accept an InboundEnvelope and feed it through the normal sidecar→eli path.
  app.post("/test/inbound", async (req, res) => {
    const envelope = req.body as InboundEnvelope;

    // Default test channel if not specified
    if (!envelope.channel) {
      envelope.channel = "test";
    }
    if (!envelope.accountId) {
      envelope.accountId = "default";
    }
    if (!envelope.senderId) {
      envelope.senderId = "test_user";
    }
    if (!envelope.chatType) {
      envelope.chatType = "direct";
    }

    try {
      await sendToEli(envelope);
      res.json({ ok: true, envelope });
    } catch (err: any) {
      res.status(500).json({ ok: false, error: err?.message ?? String(err) });
    }
  });

  // Return all captured outbound responses from the mock plugin.
  app.get("/test/responses", (_req, res) => {
    res.json({ responses: getResponses() });
  });

  // Clear captured responses.
  app.post("/test/clear", (_req, res) => {
    clearResponses();
    res.json({ ok: true });
  });

  // Health check with channel/tool counts.
  app.get("/test/health", async (_req, res) => {
    const channels = Array.from(registry.channels.keys());
    const tools = Array.from(registry.tools.keys());

    // Check if eli gateway is reachable
    let gatewayOk = false;
    try {
      const r = await fetch(`http://127.0.0.1:${process.env.ELI_WEBHOOK_PORT ?? 3100}/inbound`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ session_id: "health", channel: "test", content: "", chat_id: "health", is_active: false, kind: "normal", context: {}, output_channel: "" }),
      });
      gatewayOk = r.status === 200;
    } catch {
      gatewayOk = false;
    }

    res.json({
      sidecar: true,
      gateway: gatewayOk,
      channels,
      tools: tools.length,
    });
  });
}
