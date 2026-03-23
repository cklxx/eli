import express from "express";
import type { SidecarConfig } from "./config.js";
import type { EliChannelMessage, InboundEnvelope } from "./types.js";
import { envelopeToEliMessage, parseOutboundTarget } from "./envelope.js";
import { registry } from "./registry.js";
import { pendingTyping, lastSessionContext } from "./runtime.js";

// ---------------------------------------------------------------------------
// Inbound: channel plugin message → POST to eli
// ---------------------------------------------------------------------------

let eliUrl = "";
let sidecarConfig: SidecarConfig;

export function initBridge(config: SidecarConfig) {
  eliUrl = config.eli_url;
  sidecarConfig = config;
}

/**
 * Called by channel gateways when they receive a message.
 * Normalizes the envelope and POSTs it to eli's webhook channel.
 */
export async function sendToEli(envelope: InboundEnvelope): Promise<void> {
  const msg = envelopeToEliMessage(envelope);
  const url = `${eliUrl}/inbound`;

  try {
    const resp = await fetch(url, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(msg),
    });
    if (!resp.ok) {
      const body = await resp.text();
      console.error(`[bridge] POST ${url} failed: ${resp.status} ${body}`);
    }
  } catch (err) {
    console.error(`[bridge] POST ${url} error:`, err);
  }
}

// ---------------------------------------------------------------------------
// Outbound: eli callback → route to channel plugin
// ---------------------------------------------------------------------------

/**
 * Start the outbound HTTP server that eli calls back to with responses.
 */
export function startOutboundServer(port: number): Promise<import("node:http").Server> {
  return new Promise((resolve) => {
    const app = express();
    app.use(express.json());

    app.post("/outbound", async (req, res) => {
      const msg = req.body as EliChannelMessage;
      let { sourceChannel, accountId, chatId, chatType } = parseOutboundTarget(msg);

      // Fallback: extract source_channel from session_id (format: "channel:account:chatId")
      if (!sourceChannel && msg.session_id) {
        const parts = msg.session_id.split(":");
        if (parts.length >= 2) {
          sourceChannel = parts[0];
          accountId = accountId || parts[1];
          chatId = chatId || parts.slice(2).join(":");
        }
      }

      if (!sourceChannel) {
        console.error("[bridge] outbound: cannot determine source_channel from context or session_id");
        res.status(400).json({ error: "missing source_channel" });
        return;
      }

      const channelPlugin = registry.channels.get(sourceChannel);
      if (!channelPlugin) {
        console.error(`[bridge] outbound: unknown channel "${sourceChannel}"`);
        res.status(404).json({ error: `channel "${sourceChannel}" not found` });
        return;
      }

      if (!channelPlugin.outbound?.sendText) {
        console.error(`[bridge] outbound: channel "${sourceChannel}" has no sendText`);
        res.status(501).json({ error: `channel "${sourceChannel}" cannot send text` });
        return;
      }

      try {
        // Build the cfg object in the shape OpenClaw plugins expect:
        //   { channels: { feishu: { appId, appSecret, accounts: { default: {...} } } } }
        const cfg = { channels: sidecarConfig.channels };

        // Route target: prefer the feishu_to value captured at inbound time
        // (e.g. "user:ou_xxx" or "oc_xxx"), fall back to chatId.
        const to = msg.context?.feishu_to || chatId;

        console.log(`[bridge] outbound: channel=${sourceChannel} to=${to} accountId=${accountId} textLen=${msg.content?.length}`);

        // Remove typing indicator if one was set for this session.
        const sessionId = msg.session_id || `${sourceChannel}:${accountId}:${chatId}`;
        const typing = pendingTyping.get(sessionId);
        if (typing) {
          pendingTyping.delete(sessionId);
          try {
            const { removeTypingIndicator } = require(
              require("path").dirname(require.resolve("@larksuite/openclaw-lark"))
                + "/src/messaging/outbound/typing.js"
            );
            await removeTypingIndicator({ cfg: typing.cfg, state: typing.typingState, accountId: typing.accountId });
          } catch (e: any) {
            console.log(`[bridge] typing indicator removal failed: ${e.message}`);
          }
        }

        // OpenClaw outbound adapters use { cfg, to, text, accountId, replyToId, threadId }
        const result = await channelPlugin.outbound.sendText({
          cfg,
          to,
          text: msg.content,
          accountId,
        });

        res.json(result);
      } catch (err: any) {
        console.error(`[bridge] outbound sendText error:`, err);
        res.status(500).json({ error: err.message ?? "sendText failed" });
      }
    });

    // Tool listing — returns tool names, descriptions, and parameter schemas.
    app.get("/tools", (_req, res) => {
      const tools = Array.from(registry.tools.values()).map((t) => ({
        name: t.name,
        description: t.description,
        parameters: t.parameters,
      }));
      res.json(tools);
    });

    // Tool execution — calls the registered tool by name.
    // Accepts { params, context? } where context carries session info
    // for constructing a LarkTicket (needed by auto-auth).
    app.post("/tools/:name", async (req, res) => {
      const tool = registry.tools.get(req.params.name);
      if (!tool) {
        res.status(404).json({ error: `tool "${req.params.name}" not found` });
        return;
      }

      try {
        const params = req.body?.params ?? req.body ?? {};
        const id = req.body?.id ?? `call_${Date.now()}`;

        // Wrap in LarkTicket context from last inbound message so tools
        // can resolve user identity for OAuth auto-auth.
        let result;
        if (lastSessionContext) {
          const pluginDir = require("path").dirname(require.resolve("@larksuite/openclaw-lark"));
          const { withTicket } = require(pluginDir + "/src/core/lark-ticket.js");
          const ticket = {
            messageId: lastSessionContext.messageId,
            chatId: lastSessionContext.chatId,
            accountId: lastSessionContext.accountId,
            senderOpenId: lastSessionContext.senderOpenId,
            chatType: lastSessionContext.chatType,
            startTime: Date.now(),
          };
          result = await withTicket(ticket, () => tool.execute(id, params));
        } else {
          result = await tool.execute(id, params);
        }

        res.json(result);
      } catch (err: any) {
        console.error(`[bridge] tool "${req.params.name}" error:`, err);
        res.status(500).json({
          content: [{ type: "text", text: `Error: ${err.message ?? "tool execution failed"}` }],
        });
      }
    });

    // Health check.
    app.get("/health", (_req, res) => {
      const channels = Array.from(registry.channels.keys());
      const tools = Array.from(registry.tools.keys());
      res.json({ status: "ok", channels, tools });
    });

    const server = app.listen(port, () => {
      console.log(`[bridge] outbound server listening on :${port}`);
      resolve(server);
    });
  });
}
