import express from "express";
import type { SidecarConfig } from "./config.js";
import type {
  ChannelPlugin,
  EliChannelMessage,
  InboundEnvelope,
  SessionContext,
  ToolCallLifecycleEvent,
} from "./types.js";
import { emitPluginEvent } from "./api.js";
import { envelopeToEliMessage, parseOutboundTarget } from "./envelope.js";
import { registry } from "./registry.js";
import { endPendingTyping, sessionContexts } from "./runtime.js";

// ---------------------------------------------------------------------------
// Tool grouping — infer group from tool name prefix
// ---------------------------------------------------------------------------

/**
 * Infer a tool group from the tool name.
 * 3+ segments: first two (feishu_calendar_event → feishu-calendar)
 * 1-2 segments: first only (feishu_oauth → feishu)
 */
function inferToolGroup(name: string): string {
  const parts = name.split("_");
  if (parts.length >= 3) return `${parts[0]}-${parts[1]}`;
  return parts[0];
}

// ---------------------------------------------------------------------------
// Inbound: channel plugin message → POST to eli
// ---------------------------------------------------------------------------

let eliUrl = "";
let sidecarConfig: SidecarConfig;

export function initBridge(config: SidecarConfig) {
  eliUrl = config.eli_url;
  sidecarConfig = config;
}

function buildChannelConfig(): { channels: SidecarConfig["channels"] } {
  return { channels: sidecarConfig.channels };
}

function normalizeToolCallText(text: string | null | undefined): string | null {
  if (typeof text !== "string") return null;
  const trimmed = text.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function extractToolDescription(body: any): string | undefined {
  const description = normalizeToolCallText(body?.description);
  return description ?? undefined;
}

async function resolveToolCallText(
  channelPlugin: ChannelPlugin,
  event: ToolCallLifecycleEvent,
): Promise<string | null> {
  if (channelPlugin.lifecycle?.renderToolCallText) {
    return normalizeToolCallText(await channelPlugin.lifecycle.renderToolCallText(event));
  }
  if (event.phase !== "before") {
    return null;
  }
  return normalizeToolCallText(event.description);
}

function resolveToolNoticeTarget(
  channelPlugin: ChannelPlugin,
  sessionCtx: SessionContext,
): string {
  return (
    sessionCtx.channelTarget ||
    channelPlugin.lifecycle?.resolveOutboundTarget?.({}, sessionCtx.chatId) ||
    sessionCtx.chatId
  );
}

async function notifyToolCall(
  channelPlugin: ChannelPlugin | undefined,
  sessionCtx: SessionContext | null,
  event: ToolCallLifecycleEvent,
): Promise<void> {
  if (!sessionCtx || !channelPlugin?.outbound?.sendText) {
    return;
  }

  const text = await resolveToolCallText(channelPlugin, event);
  if (!text) {
    return;
  }

  await sendSessionNotice(channelPlugin, sessionCtx, text, `tool "${event.toolName}"`);
}

async function sendSessionNotice(
  channelPlugin: ChannelPlugin | undefined,
  sessionCtx: SessionContext | null,
  text: string,
  noticeKind = "session notice",
): Promise<void> {
  if (!sessionCtx || !channelPlugin?.outbound?.sendText) {
    return;
  }

  try {
    await channelPlugin.outbound.sendText({
      cfg: buildChannelConfig(),
      to: resolveToolNoticeTarget(channelPlugin, sessionCtx),
      text,
      accountId: sessionCtx.accountId,
    });
  } catch (err: any) {
    console.error(
      `[bridge] ${noticeKind} send failed:`,
      err?.message ?? err,
    );
  }
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

    // Auth middleware: validate ELI_SIDECAR_TOKEN when set
    const sidecarToken = process.env.ELI_SIDECAR_TOKEN;
    if (sidecarToken) {
      app.use((req, res, next) => {
        if (req.path === "/health") return next();
        const auth = req.headers.authorization;
        if (auth !== `Bearer ${sidecarToken}`) {
          res.status(401).json({ error: "unauthorized" });
          return;
        }
        next();
      });
    }

    app.post("/outbound", async (req, res) => {
      const msg = req.body as EliChannelMessage;
      const cleanupOnly = Boolean(msg.context?._eli_cleanup_only);
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

      if (!cleanupOnly && !channelPlugin.outbound?.sendText) {
        console.error(`[bridge] outbound: channel "${sourceChannel}" has no sendText`);
        res.status(501).json({ error: `channel "${sourceChannel}" cannot send text` });
        return;
      }

      try {
        // Build the cfg object in the shape OpenClaw plugins expect:
        //   { channels: { feishu: { appId, appSecret, accounts: { default: {...} } } } }
        const cfg = { channels: sidecarConfig.channels };

        // Route target: use lifecycle hook, or fall back to context fields / chatId.
        let to = chatId;
        if (!cleanupOnly) {
          if (channelPlugin.lifecycle?.resolveOutboundTarget) {
            to = channelPlugin.lifecycle.resolveOutboundTarget(msg.context ?? {}, chatId);
          } else {
            to = msg.context?.channel_target || msg.context?.feishu_to || chatId;
          }
        }

        console.log(`[bridge] outbound: channel=${sourceChannel} to=${to} accountId=${accountId} textLen=${msg.content?.length} cleanupOnly=${cleanupOnly}`);

        // Remove typing indicator if one was set for this session.
        const sessionId = msg.session_id || `${sourceChannel}:${accountId}:${chatId}`;
        // Keep cleanup off the critical path of the actual reply send. The
        // runtime queue preserves start/stop ordering for this session, so we
        // do not need to block outbound delivery on the cleanup call here.
        void endPendingTyping({ sessionId, channelPlugin });

        if (cleanupOnly) {
          res.json({ ok: true, cleanup_only: true });
          return;
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

    // Tool listing — returns tool names, descriptions, parameter schemas, and group.
    app.get("/tools", (_req, res) => {
      const tools = Array.from(registry.tools.values()).map((t) => ({
        name: t.name,
        description: t.description,
        parameters: t.parameters,
        group: t.group ?? inferToolGroup(t.name),
      }));
      res.json(tools);
    });

    // Tool execution — calls the registered tool by name.
    // Accepts { params, context?, session_id? } where context carries session info
    // for constructing channel-specific auth context.
    app.post("/tools/:name", async (req, res) => {
      const tool = registry.tools.get(req.params.name);
      if (!tool) {
        res.status(404).json({ error: `tool "${req.params.name}" not found` });
        return;
      }

      const params = req.body?.params ?? req.body ?? {};
      const id = req.body?.id ?? `call_${Date.now()}`;

      // Find session context: prefer explicit session_id, fall back to most recent.
      let sessionCtx: SessionContext | null = null;
      const requestedSession = req.body?.session_id ?? req.body?.context?.session_id;
      if (requestedSession) {
        sessionCtx = sessionContexts.get(requestedSession) ?? null;
      }
      // No fallback — require explicit session_id to prevent cross-user
      // auth leakage in multi-session scenarios.

      const channelPlugin = sessionCtx ? registry.channels.get(sessionCtx.channel) : undefined;
      const description = extractToolDescription(req.body);
      const startedAt = Date.now();

      const emitToolLifecycle = async (
        phase: "before" | "after",
        extras: Partial<ToolCallLifecycleEvent> = {},
      ) => {
        if (!sessionCtx) return;
        const event: ToolCallLifecycleEvent = {
          phase,
          toolName: tool.name,
          params,
          session: sessionCtx,
          description,
          ...extras,
        };
        await emitPluginEvent(`${phase}_tool_call`, event);
        await notifyToolCall(channelPlugin, sessionCtx, event);
      };

      await emitToolLifecycle("before");

      try {
        let result;
        if (sessionCtx) {
          // Try lifecycle hook first, then legacy fallback.
          if (channelPlugin?.lifecycle?.wrapToolExecution) {
            result = await channelPlugin.lifecycle.wrapToolExecution(sessionCtx, () =>
              tool.execute(id, params)
            );
          } else {
            // Legacy fallback: try openclaw-lark LarkTicket wrapping.
            try {
              const pluginDir = require("path").dirname(require.resolve("@larksuite/openclaw-lark"));
              const { withTicket } = require(pluginDir + "/src/core/lark-ticket.js");
              const ticket = {
                messageId: sessionCtx.messageId,
                chatId: sessionCtx.chatId,
                accountId: sessionCtx.accountId,
                senderOpenId: sessionCtx.senderId,
                chatType: sessionCtx.chatType,
                startTime: Date.now(),
              };
              result = await withTicket(ticket, () => tool.execute(id, params));
            } catch {
              result = await tool.execute(id, params);
            }
          }
        } else {
          result = await tool.execute(id, params);
        }

        await emitToolLifecycle("after", {
          durationMs: Date.now() - startedAt,
          result,
        });
        res.json(result);
      } catch (err: any) {
        const errorMessage = err?.message ?? "tool execution failed";
        await emitToolLifecycle("after", {
          durationMs: Date.now() - startedAt,
          error: errorMessage,
        });
        console.error(`[bridge] tool "${req.params.name}" error:`, err);
        res.status(500).json({
          content: [{ type: "text", text: `Error: ${errorMessage}` }],
        });
      }
    });

    app.post("/notify", async (req, res) => {
      const text = normalizeToolCallText(req.body?.text);
      if (!text) {
        res.status(400).json({ error: "missing text" });
        return;
      }

      const requestedSession = req.body?.session_id;
      if (!requestedSession) {
        res.status(400).json({ error: "missing session_id" });
        return;
      }

      const sessionCtx = sessionContexts.get(requestedSession) ?? null;
      if (!sessionCtx) {
        res.json({ ok: true, delivered: false });
        return;
      }

      const channelPlugin = registry.channels.get(sessionCtx.channel);
      await sendSessionNotice(channelPlugin, sessionCtx, text, "tool notice");
      res.json({ ok: true, delivered: true });
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
