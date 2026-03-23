import { SidecarPluginApi } from "./api.js";
import { registry } from "./registry.js";
import { sendToEli } from "./bridge.js";
import type { SidecarConfig } from "./config.js";
import type { InboundEnvelope, OpenClawPluginDefinition } from "./types.js";

// ---------------------------------------------------------------------------
// Typing indicator state — keyed by session so outbound can clean up
// ---------------------------------------------------------------------------

/** Pending typing indicator states, keyed by session_id. */
export const pendingTyping = new Map<string, { typingState: any; cfg: any; accountId: string }>();

/** Last inbound session context — used by tool execution to set LarkTicket. */
export let lastSessionContext: {
  senderOpenId: string;
  chatId: string;
  accountId: string;
  messageId: string;
  chatType: string;
  cfg: any;
} | null = null;

// ---------------------------------------------------------------------------
// Plugin loading
// ---------------------------------------------------------------------------

/**
 * Build a minimal PluginRuntime that satisfies what channel plugins need.
 *
 * The lark plugin's message pipeline calls:
 *   LarkClient.runtime.config.loadConfig() — for config
 *   LarkClient.runtime.channel.reply.dispatchReplyFromConfig() — to send msg to LLM
 *   LarkClient.runtime.channel.reply.resolveEnvelopeFormatOptions() — for envelope format
 *   LarkClient.runtime.channel.routing.resolveAgentRoute() — for session routing
 *   LarkClient.runtime.channel.commands.isControlCommandMessage() — for /command detection
 *   LarkClient.runtime.system.enqueueSystemEvent() — for system events
 *
 * We intercept dispatchReplyFromConfig to route messages to eli instead of
 * openclaw's agent runtime.
 */
function buildPluginRuntime(config: SidecarConfig) {
  return {
    config: {
      loadConfig: () => ({
        channels: config.channels,
      }),
    },
    log: (...args: any[]) => console.log("[runtime]", ...args),
    error: (...args: any[]) => console.error("[runtime]", ...args),
    channel: {
      reply: {
        /**
         * The main intercept point. When the lark plugin finishes processing
         * an inbound message, it calls this to dispatch to the "agent".
         * We redirect to eli instead.
         */
        dispatchReplyFromConfig: async (params: any) => {
          const ctx = params.ctx ?? {};
          // OpenClaw ctx uses PascalCase keys. Body is an envelope object with a nested .body field.
          const bodyEnvelope = ctx.Body ?? {};
          const textBody = typeof bodyEnvelope === "string" ? bodyEnvelope
            : (bodyEnvelope.body ?? ctx.RawBody ?? ctx.BodyForAgent ?? "");
          const senderId = ctx.SenderId ?? "";
          const senderName = ctx.SenderName ?? senderId;
          const to = ctx.To ?? "";  // "To" contains the chat target (e.g. feishu:default:oc_xxx)
          const sessionKey = ctx.SessionKey ?? "";
          const chatType = ctx.ChatType ?? "dm";
          const accountId = ctx.AccountId ?? "default";
          const channel = ctx.OriginatingChannel ?? "feishu";
          // Extract chatId from the "To" field (format: "feishu:accountId:chatId" or just chatId)
          const chatId = to.includes(":") ? to.split(":").pop()! : to;

          console.log(`[runtime] intercepted dispatchReplyFromConfig: session=${sessionKey} to=${to} sender=${senderName} body=${String(textBody).substring(0, 100)}`);

          // Save session context for tool execution (LarkTicket).
          const messageId = ctx.MessageSid ?? ctx.MessageId ?? ctx.messageId ?? "";
          lastSessionContext = {
            senderOpenId: senderId,
            chatId,
            accountId,
            messageId,
            chatType: chatType === "group" ? "group" : "p2p",
            cfg: params.cfg ?? {},
          };

          // Pass the "To" field as-is so the outbound bridge can use it
          // as the feishu route target for sendText.
          const envelope: InboundEnvelope = {
            channel,
            accountId,
            senderId,
            senderName,
            chatType: chatType === "group" ? "group" : "direct",
            chatId,
            text: typeof textBody === "string" ? textBody : JSON.stringify(textBody),
            feishu_to: to,  // e.g. "user:ou_xxx" or "feishu:default:oc_xxx"
          };

          // Fire-and-forget typing indicator (don't block message forwarding).
          if (messageId) {
            const sessionId = `${channel}:${accountId}:${chatId}`;
            const cfg = params.cfg ?? {};
            try {
              const { addTypingIndicator } = require(
                require("path").dirname(require.resolve("@larksuite/openclaw-lark"))
                  + "/src/messaging/outbound/typing.js"
              );
              addTypingIndicator({ cfg, messageId, accountId }).then((typingState: any) => {
                pendingTyping.set(sessionId, { typingState, cfg, accountId });
              }).catch(() => {});
            } catch {}
          }

          await sendToEli(envelope);

          const dispatcher = params.dispatcher;
          if (dispatcher?.waitForIdle) {
            try { await dispatcher.waitForIdle(); } catch {}
          }

          return {
            queuedFinal: Promise.resolve(),
            counts: { sent: 1, queued: 0, dropped: 0 },
          };
        },
        resolveEnvelopeFormatOptions: (_cfg: any) => ({
          historyFormat: "plain",
          mentionFormat: "plain",
        }),
        formatAgentEnvelope: (ctx: any) => ctx,
        finalizeInboundContext: (ctx: any) => ctx,
        resolveHumanDelayConfig: () => ({ enabled: false, minMs: 0, maxMs: 0 }),
        createReplyDispatcherWithTyping: (_params: any) => ({
          dispatcher: {
            waitForIdle: async () => {},
            deliver: async () => {},
          },
          replyOptions: {},
          markDispatchIdle: () => {},
          markFullyComplete: () => {},
          abortCard: async () => {},
        }),
        /**
         * System command dispatch (for /new, /reset, etc.).
         * Route to eli as well.
         */
        dispatchReplyWithBufferedBlockDispatcher: async (params: any) => {
          const ctx = params.ctx ?? {};
          const body = ctx.Body ?? ctx.body ?? ctx.rawBody ?? "";
          const senderId = ctx.SenderId ?? ctx.senderId ?? "";
          const chatId = ctx.ChatId ?? ctx.chatId ?? "";
          const channel = ctx.Channel ?? ctx.channel ?? "feishu";
          const accountId = ctx.AccountId ?? ctx.accountId ?? "default";

          console.log(`[runtime] intercepted system command: ${body}`);

          const envelope: InboundEnvelope = {
            channel,
            accountId,
            senderId,
            chatType: "direct",
            chatId,
            text: body,
          };
          await sendToEli(envelope);
        },
      },
      routing: {
        resolveAgentRoute: (params: any) => ({
          agentId: "main",
          sessionKey: `feishu:${params.accountId ?? "default"}:${params.chatId ?? "default"}`,
        }),
      },
      commands: {
        isControlCommandMessage: (text: string, _cfg: any) => {
          return /^\/(?:new|reset|stop|help)\s*$/i.test((text ?? "").trim());
        },
        shouldComputeCommandAuthorized: () => false,
        resolveCommandAuthorizedFromAuthorizers: () => true,
      },
      text: {
        resolveTextChunkLimit: () => 4000,
        resolveChunkMode: () => "plain",
        resolveMarkdownTableMode: () => "plain",
        chunkTextWithMode: (text: string) => [text],
        convertMarkdownTables: (text: string) => text,
        chunkMarkdownText: (text: string) => [text],
      },
      groups: {
        resolveGroupPolicy: () => "open",
        resolveRequireMention: () => false,
      },
      media: {
        saveMediaBuffer: async () => null,
      },
      pairing: {
        buildPairingReply: () => null,
        readAllowFromStore: () => ["*"],
        upsertPairingRequest: async () => {},
      },
    },
    logging: {
      logInbound: () => {},
      logOutbound: () => {},
    },
    system: {
      enqueueSystemEvent: (_msg: string, _data?: any) => {
        // No-op: system events are openclaw-internal.
      },
    },
  };
}

/**
 * Discover and load all plugins listed in the config.
 * Each plugin's `register()` is called with a SidecarPluginApi instance.
 *
 * Under jiti (start.cjs), require() goes through Node's native CJS loader
 * with tryNative:true, so all CJS singletons (like LarkClient) share the
 * same module cache. No monkey-patching needed.
 */
export async function loadPlugins(config: SidecarConfig): Promise<void> {
  const pluginRuntime = buildPluginRuntime(config);

  for (const pluginName of config.plugins) {
    console.log(`[runtime] loading plugin: ${pluginName}`);
    try {
      const mod = require(pluginName);
      const plugin: OpenClawPluginDefinition = mod.default ?? mod;

      if (typeof plugin.register !== "function") {
        console.error(`[runtime] plugin "${pluginName}" has no register() — skipping`);
        continue;
      }

      // Set LarkClient.runtime before register() — same as openclaw does.
      try {
        const pluginDir = require("path").dirname(require.resolve(pluginName));
        const { LarkClient } = require(pluginDir + "/src/core/lark-client.js");
        LarkClient.setRuntime(pluginRuntime);
        console.log(`[runtime] injected runtime for ${pluginName}`);

        // Override the static getter permanently so even if a new class copy
        // appears, calls to LarkClient.runtime still work. Also patch the
        // prototype chain so instances see it too.
        const origGetter = Object.getOwnPropertyDescriptor(LarkClient, 'runtime');
        Object.defineProperty(LarkClient, 'runtime', {
          get: () => pluginRuntime,
          configurable: true,
        });

        // Also monkey-patch the require cache entry so any future require()
        // of lark-client.js returns a patched LarkClient.
        const lcKey = Object.keys(require.cache).find(
          k => k.includes("lark-client") && k.endsWith(".js")
        );
        if (lcKey) {
          const origExports = require.cache[lcKey]!.exports;
          Object.defineProperty(origExports.LarkClient, 'runtime', {
            get: () => pluginRuntime,
            configurable: true,
          });
        }
      } catch (e: any) {
        console.log(`[runtime] setRuntime skipped for ${pluginName}: ${e.message}`);
      }

      const api = new SidecarPluginApi(plugin.id ?? pluginName, config);
      plugin.register(api);
      console.log(`[runtime] plugin loaded: ${plugin.id ?? pluginName}`);
    } catch (err) {
      console.error(`[runtime] failed to load plugin "${pluginName}":`, err);
    }
  }
}

// ---------------------------------------------------------------------------
// Abort controllers per channel account (for graceful shutdown)
// ---------------------------------------------------------------------------

const abortControllers = new Map<string, AbortController>();

// ---------------------------------------------------------------------------
// Channel lifecycle
// ---------------------------------------------------------------------------

/**
 * Build the gateway context object that OpenClaw channel plugins expect.
 * This mimics the ctx object from openclaw's gateway runtime.
 */
function buildGatewayContext(
  channelId: string,
  accountId: string,
  config: SidecarConfig,
  onMessage: (envelope: InboundEnvelope) => Promise<void>,
) {
  const channelConfig = config.channels[channelId] ?? { accounts: {} };

  // Build cfg in the shape OpenClaw plugins expect:
  //   cfg.channels.feishu = { appId, appSecret, accounts: { default: {...} }, ... }
  const cfg = {
    channels: {
      [channelId]: channelConfig,
    },
  };

  const ac = new AbortController();
  abortControllers.set(`${channelId}:${accountId}`, ac);

  return {
    accountId,
    cfg,
    config: channelConfig.accounts?.[accountId] ?? channelConfig,

    // Logger scoped to this channel account.
    log: {
      info: (...args: any[]) => console.log(`[${channelId}:${accountId}]`, ...args),
      warn: (...args: any[]) => console.warn(`[${channelId}:${accountId}]`, ...args),
      error: (...args: any[]) => console.error(`[${channelId}:${accountId}]`, ...args),
      debug: (...args: any[]) => console.debug(`[${channelId}:${accountId}]`, ...args),
    },

    // Status callback — log and ignore.
    setStatus: (status: any) => {
      console.log(`[${channelId}:${accountId}] status:`, JSON.stringify(status));
    },

    // Abort signal for graceful shutdown.
    abortSignal: ac.signal,

    // Minimal runtime — plugins use runtime.log, runtime.error, and
    // runtime.config.loadConfig() for the message dispatch pipeline.
    runtime: buildPluginRuntime(config),

    // The onMessage callback for inbound messages.
    onMessage,
  };
}

/**
 * Start all registered channel gateways.
 */
export async function startChannels(config: SidecarConfig): Promise<void> {
  for (const [channelId, plugin] of registry.channels) {
    if (!plugin.gateway) {
      console.log(`[runtime] channel "${channelId}" has no gateway adapter — outbound only`);
      continue;
    }

    const channelConfig = config.channels[channelId] ?? { accounts: {} };
    let accountIds: string[];

    try {
      accountIds = await plugin.config.listAccountIds({
        channels: { [channelId]: channelConfig },
      });
    } catch {
      accountIds = Object.keys(channelConfig.accounts ?? {});
      if (accountIds.length === 0) {
        accountIds = ["default"];
      }
    }

    for (const accountId of accountIds) {
      console.log(`[runtime] starting channel "${channelId}" account "${accountId}"`);

      const onMessage = async (envelope: InboundEnvelope) => {
        envelope.channel = envelope.channel || channelId;
        envelope.accountId = envelope.accountId || accountId;
        await sendToEli(envelope);
      };

      const ctx = buildGatewayContext(channelId, accountId, config, onMessage);

      try {
        const startFn = plugin.gateway.startAccount ?? plugin.gateway.start;
        if (!startFn) {
          console.error(`[runtime] channel "${channelId}" gateway has no start/startAccount`);
          continue;
        }
        await startFn.call(plugin.gateway, ctx);
        console.log(`[runtime] channel "${channelId}" account "${accountId}" started`);
      } catch (err) {
        console.error(
          `[runtime] failed to start channel "${channelId}" account "${accountId}":`,
          err,
        );
      }
    }
  }
}

/**
 * Stop all registered channel gateways.
 */
export async function stopChannels(config: SidecarConfig): Promise<void> {
  // Signal abort to all running gateways.
  for (const [key, ac] of abortControllers) {
    ac.abort();
    abortControllers.delete(key);
  }

  for (const [channelId, plugin] of registry.channels) {
    const stopFn = plugin.gateway?.stopAccount ?? plugin.gateway?.stop;
    if (!stopFn) continue;

    const channelConfig = config.channels[channelId] ?? { accounts: {} };
    let accountIds: string[];

    try {
      accountIds = await plugin.config.listAccountIds({
        channels: { [channelId]: channelConfig },
      });
    } catch {
      accountIds = Object.keys(channelConfig.accounts ?? {});
    }

    for (const accountId of accountIds) {
      try {
        await stopFn.call(plugin.gateway, {
          accountId,
          cfg: { channels: { [channelId]: channelConfig } },
          log: {
            info: (...args: any[]) => console.log(`[${channelId}:${accountId}]`, ...args),
            warn: (...args: any[]) => console.warn(`[${channelId}:${accountId}]`, ...args),
            error: (...args: any[]) => console.error(`[${channelId}:${accountId}]`, ...args),
            debug: (...args: any[]) => console.debug(`[${channelId}:${accountId}]`, ...args),
          },
        });
        console.log(`[runtime] channel "${channelId}" account "${accountId}" stopped`);
      } catch (err) {
        console.error(
          `[runtime] failed to stop channel "${channelId}" account "${accountId}":`,
          err,
        );
      }
    }
  }
}
