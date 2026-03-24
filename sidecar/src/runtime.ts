import { SidecarPluginApi } from "./api.js";
import { registry } from "./registry.js";
import { sendToEli } from "./bridge.js";
import type { SidecarConfig } from "./config.js";
import type {
  ChannelPlugin,
  InboundEnvelope,
  OpenClawPluginDefinition,
  SessionContext,
} from "./types.js";

// ---------------------------------------------------------------------------
// Typing indicator state — keyed by session so outbound can clean up
// ---------------------------------------------------------------------------

/** Pending typing indicator states, keyed by session_id. */
export const pendingTyping = new Map<string, { typingState: any; cfg: any; accountId: string }>();

/**
 * Serialize typing lifecycle operations per session.
 *
 * Purpose:
 * The Feishu typing indicator is reaction-based, so "start typing" and
 * "stop typing" are two separate async API calls.
 *
 * Scenario:
 * `dispatchReplyFromConfig()` starts the typing reaction and immediately
 * forwards the inbound message to eli. The webhook `/inbound` handler only
 * enqueues that message and returns `200`, so the final outbound reply can
 * reach the sidecar before `addTypingIndicator()` has finished. Without a
 * queue, outbound cleanup checks `pendingTyping` too early, sees nothing,
 * and never calls remove.
 */
const typingQueues = new Map<string, Promise<void>>();

/** Per-session context — used by tool execution for channel-specific auth wrapping. */
export const sessionContexts = new Map<string, SessionContext>();

/** TTL for session context entries (30 minutes). */
const SESSION_CONTEXT_TTL_MS = 30 * 60 * 1000;

/** Track TTL timers so we can cancel the old one when a session refreshes. */
const sessionTimers = new Map<string, ReturnType<typeof setTimeout>>();

function queueTypingTask(sessionId: string, task: () => Promise<void>): Promise<void> {
  const previous = typingQueues.get(sessionId) ?? Promise.resolve();
  let tracked: Promise<void>;
  tracked = previous
    .catch(() => {})
    .then(task)
    .catch(() => {})
    .finally(() => {
      if (typingQueues.get(sessionId) === tracked) {
        typingQueues.delete(sessionId);
      }
    });
  typingQueues.set(sessionId, tracked);
  return tracked;
}

async function addTypingState(params: {
  channelPlugin?: ChannelPlugin;
  cfg: any;
  messageId: string;
  accountId: string;
  sessionId: string;
}): Promise<any> {
  const { channelPlugin, cfg, messageId, accountId, sessionId } = params;

  if (channelPlugin?.lifecycle?.onInboundMessage) {
    try {
      return await channelPlugin.lifecycle.onInboundMessage({ cfg, messageId, accountId, sessionId });
    } catch {
      return null;
    }
  }

  try {
    const { addTypingIndicator } = require(
      require("path").dirname(require.resolve("@larksuite/openclaw-lark"))
        + "/src/messaging/outbound/typing.js"
    );
    const state = await addTypingIndicator({ cfg, messageId, accountId });
    if (state?.messageId && !state?.reactionId) {
      console.log(
        `[runtime] typing indicator added without reactionId for message ${state.messageId}; cleanup will require fallback lookup`,
      );
    }
    return state;
  } catch {
    return null;
  }
}

async function removeTypingState(
  channelPlugin: ChannelPlugin | undefined,
  typing: { typingState: any; cfg: any; accountId: string },
): Promise<void> {
  if (channelPlugin?.lifecycle?.onOutboundReply) {
    try {
      await channelPlugin.lifecycle.onOutboundReply({
        cfg: typing.cfg,
        typingState: typing.typingState,
        accountId: typing.accountId,
      });
    } catch (e: any) {
      console.log(`[runtime] lifecycle typing removal failed: ${e.message}`);
    }
    return;
  }

  try {
    const pluginDir = require("path").dirname(require.resolve("@larksuite/openclaw-lark"));
    const { listReactionsFeishu, removeReactionFeishu } = require(
      pluginDir + "/src/messaging/outbound/reactions.js"
    );
    const messageId = typing.typingState?.messageId;
    const reactionId = typing.typingState?.reactionId;

    if (!messageId) {
      return;
    }

    if (reactionId) {
      await removeReactionFeishu({
        cfg: typing.cfg,
        messageId,
        reactionId,
        accountId: typing.accountId,
      });
      return;
    }

    const reactions = await listReactionsFeishu({
      cfg: typing.cfg,
      messageId,
      emojiType: "Typing",
      accountId: typing.accountId,
    });
    const appTypingReactions = reactions.filter((reaction: any) =>
      reaction?.operatorType === "app" && typeof reaction?.reactionId === "string" && reaction.reactionId.length > 0
    );

    if (appTypingReactions.length === 0) {
      console.log(
        `[runtime] typing cleanup skipped for message ${messageId}: no reactionId in state and no app-owned Typing reactions found`,
      );
      return;
    }

    for (const reaction of appTypingReactions) {
      await removeReactionFeishu({
        cfg: typing.cfg,
        messageId,
        reactionId: reaction.reactionId,
        accountId: typing.accountId,
      });
    }
    console.log(
      `[runtime] typing cleanup fallback removed ${appTypingReactions.length} app-owned Typing reaction(s) for message ${messageId}`,
    );
  } catch (e: any) {
    console.log(`[runtime] typing indicator removal failed: ${e.message}`);
  }
}

export function beginPendingTyping(params: {
  channelPlugin?: ChannelPlugin;
  cfg: any;
  messageId: string;
  accountId: string;
  sessionId: string;
}): Promise<void> {
  return queueTypingTask(params.sessionId, async () => {
    // Queue the add operation so a later cleanup for the same session cannot
    // overtake it and get dropped before the reaction state is recorded.
    const typingState = await addTypingState(params);
    if (!typingState) {
      pendingTyping.delete(params.sessionId);
      return;
    }
    pendingTyping.set(params.sessionId, {
      typingState,
      cfg: params.cfg,
      accountId: params.accountId,
    });
  });
}

export function endPendingTyping(params: {
  sessionId: string;
  channelPlugin?: ChannelPlugin;
}): Promise<void> {
  return queueTypingTask(params.sessionId, async () => {
    // Cleanup shares the same queue as beginPendingTyping(). If outbound
    // arrives before typing setup finishes, this waits behind the add step
    // and still removes the reaction once the state is available.
    const typing = pendingTyping.get(params.sessionId);
    if (!typing) {
      return;
    }

    pendingTyping.delete(params.sessionId);
    await removeTypingState(params.channelPlugin, typing);
  });
}

// ---------------------------------------------------------------------------
// Plugin loading
// ---------------------------------------------------------------------------

/**
 * Build a minimal PluginRuntime that satisfies what channel plugins need.
 *
 * OpenClaw channel plugins call into runtime methods during their message
 * pipeline. We intercept dispatchReplyFromConfig to route messages to eli
 * instead of openclaw's agent runtime.
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
         * The main intercept point. When a channel plugin finishes processing
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
          const to = ctx.To ?? "";  // "To" contains the chat target (e.g. channel:account:chatId)
          const sessionKey = ctx.SessionKey ?? "";
          const chatType = ctx.ChatType ?? "dm";
          const accountId = ctx.AccountId ?? "default";
          const channel = ctx.OriginatingChannel ?? "unknown";
          // Extract chatId from the "To" field (format: "channel:accountId:chatId" or just chatId)
          const chatId = to.includes(":") ? to.split(":").pop()! : to;

          console.log(`[runtime] intercepted dispatchReplyFromConfig: session=${sessionKey} to=${to} sender=${senderName} body=${String(textBody).substring(0, 100)}`);

          // Save session context for tool execution.
          const messageId = ctx.MessageSid ?? ctx.MessageId ?? ctx.messageId ?? "";
          const sessionId = `${channel}:${accountId}:${chatId}`;
          const sessionCtx: SessionContext = {
            channel,
            senderId,
            chatId,
            channelTarget: to,
            accountId,
            messageId,
            chatType: chatType === "group" ? "group" : "p2p",
            cfg: params.cfg ?? {},
          };
          sessionContexts.set(sessionId, sessionCtx);
          // Cancel previous TTL timer to prevent it from deleting a refreshed entry.
          const prevTimer = sessionTimers.get(sessionId);
          if (prevTimer) clearTimeout(prevTimer);
          const timer = setTimeout(() => {
            sessionContexts.delete(sessionId);
            sessionTimers.delete(sessionId);
          }, SESSION_CONTEXT_TTL_MS);
          sessionTimers.set(sessionId, timer);

          // Pass the "To" field as-is so the outbound bridge can use it
          // as the channel route target for sendText.
          const envelope: InboundEnvelope = {
            channel,
            accountId,
            senderId,
            senderName,
            chatType: chatType === "group" ? "group" : "direct",
            chatId,
            text: typeof textBody === "string" ? textBody : JSON.stringify(textBody),
            channel_target: to,  // e.g. "user:ou_xxx" or "channel:account:chatId"
          };

          // Fire-and-forget typing indicator (don't block message forwarding).
          if (messageId) {
            const cfg = params.cfg ?? {};
            const channelPlugin = registry.channels.get(channel);
            void beginPendingTyping({ channelPlugin, cfg, messageId, accountId, sessionId });
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
          const channel = ctx.Channel ?? ctx.channel ?? "unknown";
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
          sessionKey: `${params.channel ?? "channel"}:${params.accountId ?? "default"}:${params.chatId ?? "default"}`,
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

      // Pre-registration: inject runtime via lifecycle hook or legacy fallback.
      if (plugin.lifecycle?.initRuntime) {
        try {
          plugin.lifecycle.initRuntime(pluginRuntime, pluginName);
          console.log(`[runtime] lifecycle.initRuntime called for ${pluginName}`);
        } catch (e: any) {
          console.log(`[runtime] lifecycle.initRuntime failed for ${pluginName}: ${e.message}`);
        }
      } else {
        // Legacy fallback: try Lark-specific runtime injection.
        try {
          const pluginDir = require("path").dirname(require.resolve(pluginName));
          const { LarkClient } = require(pluginDir + "/src/core/lark-client.js");
          LarkClient.setRuntime(pluginRuntime);
          console.log(`[runtime] injected runtime for ${pluginName}`);

          // Override the static getter permanently so even if a new class copy
          // appears, calls to LarkClient.runtime still work.
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
      }

      const api = new SidecarPluginApi(plugin.id ?? pluginName, config);
      plugin.register(api);
      console.log(`[runtime] plugin loaded: ${plugin.id ?? pluginName}`);

      // Discover SKILL.md files from the plugin's skills/ directory.
      installPluginSkills(pluginName);
    } catch (err) {
      console.error(`[runtime] failed to load plugin "${pluginName}":`, err);
    }
  }
}

/** Tracks SKILL.md files written by the sidecar for cleanup on shutdown. */
const writtenSkillDirs: string[] = [];

/**
 * Copy a plugin's SKILL.md files to .agents/skills/ so any agent discovers
 * them via the standard skills protocol (discover_skills).
 *
 * This is the standard integration path — no custom endpoints or registration.
 * Agent reads SKILL.md → sees tool names → calls via sidecar bridge tool.
 */
function installPluginSkills(pluginName: string): void {
  const path = require("path");
  const fs = require("fs");

  let pluginDir: string;
  try {
    pluginDir = path.dirname(require.resolve(pluginName));
  } catch {
    return;
  }

  const srcSkillsDir = path.join(pluginDir, "skills");
  if (!fs.existsSync(srcSkillsDir)) return;

  // Write to SIDECAR_SKILLS_DIR if set (eli passes workspace path),
  // otherwise fall back to cwd.
  const skillsRoot = process.env.SIDECAR_SKILLS_DIR || process.cwd();
  const destSkillsDir = path.join(skillsRoot, ".agents", "skills");
  fs.mkdirSync(destSkillsDir, { recursive: true });

  for (const entry of fs.readdirSync(srcSkillsDir, { withFileTypes: true }) as any[]) {
    if (!entry.isDirectory()) continue;
    const srcSkillMd = path.join(srcSkillsDir, entry.name, "SKILL.md");
    if (!fs.existsSync(srcSkillMd)) continue;

    const destDir = path.join(destSkillsDir, entry.name);
    const destFile = path.join(destDir, "SKILL.md");

    // Don't overwrite user's own SKILL.md files.
    if (fs.existsSync(destFile)) {
      console.log(`[skills] ${entry.name}: skipped (already exists)`);
      continue;
    }

    try {
      fs.mkdirSync(destDir, { recursive: true });

      // Read source and inject tool-calling instruction after frontmatter.
      let content: string = fs.readFileSync(srcSkillMd, "utf-8");
      const toolCallHint = `\n> **Tool calling:** Use \`sidecar(tool="<tool_name>", params={...})\` to call tools in this skill.\n`;
      const fmEnd = content.indexOf("\n---\n");
      if (fmEnd !== -1) {
        const insertAt = fmEnd + 5; // after "---\n"
        content = content.slice(0, insertAt) + toolCallHint + content.slice(insertAt);
      } else {
        content = toolCallHint + content;
      }
      fs.writeFileSync(destFile, content);

      writtenSkillDirs.push(destDir);
      console.log(`[skills] installed: ${entry.name}`);
    } catch (e: any) {
      console.log(`[skills] failed to install ${entry.name}: ${e.message}`);
    }
  }

  // Also generate SKILL.md files for tool groups that don't have one.
  generateMissingSkills(destSkillsDir, srcSkillsDir);
}

/**
 * For tools not covered by any plugin SKILL.md, auto-generate a minimal one.
 */
function generateMissingSkills(destSkillsDir: string, srcSkillsDir: string): void {
  const path = require("path");
  const fs = require("fs");

  // Find which tools are covered by existing SKILL.md files.
  const coveredPrefixes = new Set<string>();
  if (fs.existsSync(srcSkillsDir)) {
    for (const entry of fs.readdirSync(srcSkillsDir, { withFileTypes: true }) as any[]) {
      if (entry.isDirectory()) coveredPrefixes.add(entry.name.replace(/-/g, "_"));
    }
  }

  // Group uncovered tools.
  const uncovered = new Map<string, Array<{ name: string; description: string }>>();
  for (const t of registry.tools.values()) {
    const isCovered = Array.from(coveredPrefixes).some((p) => t.name.startsWith(p));
    if (isCovered) continue;

    const parts = t.name.split("_");
    const group = parts.length >= 3 ? `${parts[0]}-${parts[1]}` : parts[0];
    if (!uncovered.has(group)) uncovered.set(group, []);
    uncovered.get(group)!.push({ name: t.name, description: t.description });
  }

  for (const [groupName, tools] of uncovered) {
    const destDir = path.join(destSkillsDir, groupName);
    const destFile = path.join(destDir, "SKILL.md");
    if (fs.existsSync(destFile)) continue;

    const toolNames = tools.map((t) => t.name).join(", ");
    let body = `---\nname: ${groupName}\ndescription: "${tools.length} tools: ${toolNames}"\n---\n\n`;
    body += `Call tools via: sidecar(tool="<name>", params={...})\n\n`;
    for (const t of tools) {
      body += `## ${t.name}\n${t.description || "(no description)"}\n\n`;
    }

    try {
      fs.mkdirSync(destDir, { recursive: true });
      fs.writeFileSync(destFile, body);
      writtenSkillDirs.push(destDir);
      console.log(`[skills] generated: ${groupName}`);
    } catch {}
  }
}

/**
 * Remove SKILL.md files that the sidecar installed (cleanup on shutdown).
 * Only removes files the sidecar wrote — never touches user-created skills.
 */
export function cleanupInstalledSkills(): void {
  const fs = require("fs");
  for (const dir of writtenSkillDirs) {
    try {
      fs.rmSync(dir, { recursive: true, force: true });
    } catch {}
  }
  writtenSkillDirs.length = 0;
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
