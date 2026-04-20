/**
 * Built-in Telegram channel plugin for the sidecar.
 *
 * Uses the Telegram Bot API directly via node:https.
 * This avoids undici/connect-timeout regressions seen on some macOS/VPN/IPv6
 * paths while keeping the channel dependency-free.
 *
 * Config: SIDECAR_TELEGRAM_TOKEN (or ELI_TELEGRAM_TOKEN for backward compat).
 * Optional: SIDECAR_TELEGRAM_ALLOW_USERS, SIDECAR_TELEGRAM_ALLOW_CHATS,
 * SIDECAR_TELEGRAM_IP_FAMILY.
 */

import { writeFileSync, mkdirSync, existsSync } from "node:fs";
import { request as httpsRequest } from "node:https";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { randomBytes } from "node:crypto";
import type {
  ChannelPlugin,
  InboundEnvelope,
  OutboundTextParams,
  OutboundMediaParams,
  OutboundResult,
  OutboundTarget,
} from "../src/types.js";
import { logger } from "../src/log.js";

const log = logger("telegram");
const DEFAULT_REQUEST_TIMEOUT_MS = 60_000;
const LONG_POLL_GRACE_MS = 5_000;
const DEFAULT_IP_FAMILY = 4;

type TelegramIpFamily = 4 | 6;

// ---------------------------------------------------------------------------
// Telegram Bot API helpers
// ---------------------------------------------------------------------------

function apiUrl(token: string, method: string): string {
  return `https://api.telegram.org/bot${token}/${method}`;
}

export function parseIpFamily(value: unknown): TelegramIpFamily {
  return String(value) === "6" ? 6 : 4;
}

export function resolveApiTimeoutMs(method: string, params: Record<string, any>): number {
  if (method !== "getUpdates") return DEFAULT_REQUEST_TIMEOUT_MS;
  const longPollMs = Number(params.timeout ?? 0) * 1000;
  return Math.max(DEFAULT_REQUEST_TIMEOUT_MS, longPollMs + LONG_POLL_GRACE_MS);
}

function requestBuffer(
  url: string,
  options: {
    method?: string;
    headers?: Record<string, string>;
    body?: Buffer;
    family?: TelegramIpFamily;
    timeoutMs?: number;
  } = {},
): Promise<{ statusCode: number; body: Buffer }> {
  return new Promise((resolve, reject) => {
    const req = httpsRequest(url, {
      family: options.family ?? DEFAULT_IP_FAMILY,
      method: options.method ?? "GET",
      headers: options.headers,
    }, (res) => {
      const chunks: Buffer[] = [];
      res.on("data", (chunk) => chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk)));
      res.on("end", () => {
        resolve({
          statusCode: res.statusCode ?? 0,
          body: Buffer.concat(chunks),
        });
      });
    });
    req.setTimeout(options.timeoutMs ?? DEFAULT_REQUEST_TIMEOUT_MS, () => {
      req.destroy(new Error("request timeout"));
    });
    req.on("error", reject);
    if (options.body) req.write(options.body);
    req.end();
  });
}

async function requestJson<T>(
  url: string,
  options: {
    method?: string;
    headers?: Record<string, string>;
    body?: Buffer;
    family?: TelegramIpFamily;
    timeoutMs?: number;
  } = {},
): Promise<T> {
  const response = await requestBuffer(url, options);
  const text = response.body.toString("utf8");
  return JSON.parse(text) as T;
}

async function callApi(
  token: string,
  method: string,
  params: Record<string, any> = {},
  family: TelegramIpFamily = DEFAULT_IP_FAMILY,
): Promise<any> {
  const body = Buffer.from(JSON.stringify(params), "utf8");
  const data = await requestJson<any>(apiUrl(token, method), {
    method: "POST",
    headers: {
      "Content-Type": "application/json",
      "Content-Length": String(body.length),
    },
    body,
    family,
    timeoutMs: resolveApiTimeoutMs(method, params),
  });
  if (!data.ok) {
    throw new Error(`Telegram API ${method}: ${data.description ?? "unknown error"}`);
  }
  return data.result;
}

async function downloadFile(
  token: string,
  fileId: string,
  family: TelegramIpFamily = DEFAULT_IP_FAMILY,
): Promise<Buffer> {
  const file = await callApi(token, "getFile", { file_id: fileId }, family);
  const filePath = file.file_path;
  const url = `https://api.telegram.org/file/bot${token}/${filePath}`;
  const response = await requestBuffer(url, { family });
  if (response.statusCode < 200 || response.statusCode >= 300) {
    throw new Error(`download failed: ${response.statusCode}`);
  }
  return response.body;
}

function saveTempFile(data: Buffer, ext: string): string {
  const dir = join(tmpdir(), "eli-telegram-media");
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  const name = `tg-${Date.now()}-${randomBytes(4).toString("hex")}${ext}`;
  const path = join(dir, name);
  writeFileSync(path, data);
  return path;
}

// ---------------------------------------------------------------------------
// Message parsing
// ---------------------------------------------------------------------------

interface TgMessage {
  message_id: number;
  chat: { id: number; type: string };
  from?: { id: number; username?: string; first_name?: string; last_name?: string };
  text?: string;
  caption?: string;
  photo?: Array<{ file_id: string; width: number; height: number }>;
  audio?: { file_id: string; title?: string; performer?: string; duration: number; mime_type?: string; file_name?: string };
  video?: { file_id: string; duration: number; mime_type?: string; file_name?: string };
  voice?: { file_id: string; duration: number; mime_type?: string };
  document?: { file_id: string; mime_type?: string; file_name?: string };
  sticker?: { file_id: string; emoji?: string; set_name?: string; is_animated?: boolean };
  video_note?: { file_id: string; duration: number };
  reply_to_message?: TgMessage;
  entities?: Array<{ type: string; offset: number; length: number; url?: string }>;
  caption_entities?: Array<{ type: string; offset: number; length: number; url?: string }>;
  new_chat_member?: { id: number };
}

interface TgUpdate {
  update_id: number;
  message?: TgMessage;
  my_chat_member?: {
    chat: { id: number; type: string };
    old_chat_member: { status: string };
    new_chat_member: { status: string; user: { id: number } };
  };
}

function fullName(from: TgMessage["from"]): string {
  if (!from) return "";
  const parts = [from.first_name, from.last_name].filter(Boolean);
  return parts.join(" ");
}

function detectMediaType(msg: TgMessage): { type: string; fileId: string; mime: string; ext: string } | null {
  if (msg.photo?.length) {
    const best = msg.photo[msg.photo.length - 1];
    return { type: "image", fileId: best.file_id, mime: "image/jpeg", ext: ".jpg" };
  }
  if (msg.audio) return { type: "audio", fileId: msg.audio.file_id, mime: msg.audio.mime_type ?? "audio/mpeg", ext: ".mp3" };
  if (msg.voice) return { type: "audio", fileId: msg.voice.file_id, mime: msg.voice.mime_type ?? "audio/ogg", ext: ".ogg" };
  if (msg.video) return { type: "video", fileId: msg.video.file_id, mime: msg.video.mime_type ?? "video/mp4", ext: ".mp4" };
  if (msg.video_note) return { type: "video", fileId: msg.video_note.file_id, mime: "video/mp4", ext: ".mp4" };
  if (msg.document) return { type: "file", fileId: msg.document.file_id, mime: msg.document.mime_type ?? "application/octet-stream", ext: "" };
  if (msg.sticker) return { type: "image", fileId: msg.sticker.file_id, mime: msg.sticker.is_animated ? "video/webm" : "image/webp", ext: msg.sticker.is_animated ? ".webm" : ".webp" };
  return null;
}

function formatContent(msg: TgMessage): string {
  if (msg.text) return msg.text;
  const caption = msg.caption ?? "";
  if (msg.photo) return caption ? `[Photo] ${caption}` : "[Photo]";
  if (msg.audio) {
    const title = msg.audio.title ?? "Unknown";
    const performer = msg.audio.performer ?? "";
    return performer ? `[Audio: ${performer} - ${title}]` : `[Audio: ${title}]`;
  }
  if (msg.voice) return `[Voice: ${msg.voice.duration}s]`;
  if (msg.video) return caption ? `[Video: ${msg.video.duration}s] ${caption}` : `[Video: ${msg.video.duration}s]`;
  if (msg.video_note) return `[Video note: ${msg.video_note.duration}s]`;
  if (msg.document) {
    const name = msg.document.file_name ?? "unknown";
    return caption ? `[Document: ${name}] ${caption}` : `[Document: ${name}]`;
  }
  if (msg.sticker) {
    const emoji = msg.sticker.emoji ?? "";
    return emoji ? `[Sticker: ${emoji}]` : "[Sticker]";
  }
  return caption || "";
}

function stripEliPrefix(text: string): string {
  return text.startsWith("/eli ") ? text.slice(5) : text;
}

// ---------------------------------------------------------------------------
// Access control & group filtering
// ---------------------------------------------------------------------------

interface TelegramConfig {
  token: string;
  allow_users: Set<string>;
  allow_chats: Set<string>;
  ip_family: TelegramIpFamily;
}

function parseConfig(channelConfig: any): TelegramConfig {
  const token =
    channelConfig?.token ??
    channelConfig?.accounts?.default?.token ??
    process.env.ELI_TELEGRAM_TOKEN ??
    "";

  const parseSet = (val: string | undefined): Set<string> =>
    new Set((val ?? "").split(",").map(s => s.trim()).filter(Boolean));

  const allow_users = parseSet(
    channelConfig?.allow_users ??
    channelConfig?.accounts?.default?.allow_users ??
    process.env.ELI_TELEGRAM_ALLOW_USERS ??
    process.env.SIDECAR_TELEGRAM_ALLOW_USERS
  );
  const allow_chats = parseSet(
    channelConfig?.allow_chats ??
    channelConfig?.accounts?.default?.allow_chats ??
    process.env.ELI_TELEGRAM_ALLOW_CHATS ??
    process.env.SIDECAR_TELEGRAM_ALLOW_CHATS
  );

  const ip_family = parseIpFamily(
    channelConfig?.ip_family ??
    channelConfig?.accounts?.default?.ip_family ??
    process.env.ELI_TELEGRAM_IP_FAMILY ??
    process.env.SIDECAR_TELEGRAM_IP_FAMILY
  );

  return { token, allow_users, allow_chats, ip_family };
}

function checkAccess(msg: TgMessage, cfg: TelegramConfig): "allowed" | "denied_chat" | "denied_user" | "start" {
  const chatId = String(msg.chat.id);

  if (cfg.allow_chats.size > 0 && !cfg.allow_chats.has(chatId)) {
    return "denied_chat";
  }

  if (cfg.allow_users.size > 0 && msg.from) {
    const uid = String(msg.from.id);
    const uname = msg.from.username ?? "";
    if (!cfg.allow_users.has(uid) && !cfg.allow_users.has(uname)) {
      return "denied_user";
    }
  }

  if (msg.text?.startsWith("/start")) return "start";
  return "allowed";
}

function shouldProcessGroupMessage(
  msg: TgMessage,
  botId: number,
  botUsername: string,
): boolean {
  const content = (msg.text ?? msg.caption ?? "").toLowerCase();
  const mentionsBot =
    content.includes("eli") ||
    (botUsername && content.includes(`@${botUsername.toLowerCase()}`));
  const repliesToBot = msg.reply_to_message?.from?.id === botId;

  // Media-only messages without caption: only process if replying to bot
  if (!msg.text && !msg.caption) return repliesToBot;

  return mentionsBot || repliesToBot;
}

// ---------------------------------------------------------------------------
// Polling gateway
// ---------------------------------------------------------------------------

async function pollLoop(
  cfg: TelegramConfig,
  onMessage: (envelope: InboundEnvelope) => Promise<void>,
  abortSignal?: AbortSignal,
): Promise<void> {
  const me = await callApi(cfg.token, "getMe", {}, cfg.ip_family);
  const botId: number = me.id;
  const botUsername: string = me.username ?? "";
  log.info("bot identity resolved", { id: botId, username: botUsername });

  let offset = 0;

  while (!abortSignal?.aborted) {
    let updates: TgUpdate[];
    try {
      updates = await callApi(cfg.token, "getUpdates", {
        offset,
        timeout: 30,
        allowed_updates: ["message", "my_chat_member"],
      }, cfg.ip_family);
    } catch (err: any) {
      if (abortSignal?.aborted) break;
      // Telegram returns "Conflict: terminated by other getUpdates request"
      // when another process is polling the same bot token. Retrying is
      // futile — the other instance will keep winning. Exit cleanly.
      if (err.message?.includes("Conflict:")) {
        log.error(
          "another bot instance is polling this token — stopping this poller. " +
          "Kill the other `eli gateway` process or check for stale sidecar processes.",
          { err: err.message },
        );
        break;
      }
      log.error("polling error", { err: err.message });
      await new Promise(r => setTimeout(r, 3000));
      continue;
    }

    for (const update of updates) {
      offset = update.update_id + 1;

      // Join event
      if (update.my_chat_member) {
        const cm = update.my_chat_member;
        const wasAbsent = ["left", "kicked"].includes(cm.old_chat_member.status);
        const isPresent = ["member", "administrator", "creator"].includes(cm.new_chat_member.status);
        if (wasAbsent && isPresent && cm.new_chat_member.user.id === botId) {
          const chatId = String(cm.chat.id);
          await onMessage({
            channel: "telegram",
            accountId: "default",
            senderId: "",
            chatType: cm.chat.type === "private" ? "direct" : "group",
            chatId,
            text: "",
          });
        }
        continue;
      }

      const msg = update.message;
      if (!msg) continue;

      // Access control
      const access = checkAccess(msg, cfg);
      if (access === "denied_chat" || access === "denied_user") {
        if (access === "denied_chat" && msg.text?.startsWith("/start")) {
          await callApi(cfg.token, "sendMessage", {
            chat_id: msg.chat.id,
            text: "You are not allowed to chat with me. Please deploy your own instance of Eli.",
          }, cfg.ip_family).catch(() => {});
        }
        if (access === "denied_user") {
          await callApi(cfg.token, "sendMessage", {
            chat_id: msg.chat.id,
            text: "Access denied.",
          }, cfg.ip_family).catch(() => {});
        }
        continue;
      }

      if (access === "start") {
        await callApi(cfg.token, "sendMessage", {
          chat_id: msg.chat.id,
          text: "Eli is online. Send text to start.",
        }, cfg.ip_family).catch(() => {});
        continue;
      }

      // Group chat filtering
      const isGroup = msg.chat.type !== "private";
      if (isGroup && !shouldProcessGroupMessage(msg, botId, botUsername)) {
        continue;
      }

      // Build envelope
      const chatId = String(msg.chat.id);
      const content = stripEliPrefix(formatContent(msg));

      // Download media to temp files
      const mediaPaths: string[] = [];
      const mediaTypes: string[] = [];

      for (const source of [msg, msg.reply_to_message].filter(Boolean) as TgMessage[]) {
        const media = detectMediaType(source);
        if (!media) continue;
        try {
          const data = await downloadFile(cfg.token, media.fileId, cfg.ip_family);
          const path = saveTempFile(data, media.ext);
          mediaPaths.push(path);
          mediaTypes.push(media.type);
        } catch (err: any) {
          log.error("media download failed", { err: err.message });
        }
      }

      // Send typing indicator
      await callApi(cfg.token, "sendChatAction", {
        chat_id: msg.chat.id,
        action: "typing",
      }, cfg.ip_family).catch(() => {});

      const envelope: InboundEnvelope = {
        channel: "telegram",
        accountId: "default",
        senderId: msg.from ? String(msg.from.id) : "",
        senderName: fullName(msg.from),
        chatType: isGroup ? "group" : "direct",
        chatId,
        text: content,
        media_paths: mediaPaths.length > 0 ? mediaPaths : undefined,
        media_types: mediaTypes.length > 0 ? mediaTypes : undefined,
      };

      try {
        await onMessage(envelope);
      } catch (err: any) {
        log.error("onMessage error", { err: err.message });
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Outbound: send text & media
// ---------------------------------------------------------------------------

async function sendText(params: OutboundTextParams): Promise<OutboundResult> {
  const token = resolveToken(params.cfg);
  const family = resolveIpFamily(params.cfg);
  if (!token) return { ok: false, error: "no telegram token" };

  const chatId = Number(params.to);
  if (!chatId) return { ok: false, error: `invalid chat_id: ${params.to}` };

  if (!params.text?.trim()) return { ok: true };

  // Try MarkdownV2 first, fall back to plain text
  try {
    await callApi(token, "sendMessage", {
      chat_id: chatId,
      text: params.text,
      parse_mode: "MarkdownV2",
    }, family);
  } catch {
    await callApi(token, "sendMessage", {
      chat_id: chatId,
      text: params.text,
    }, family);
  }

  return { ok: true };
}

async function sendMedia(params: OutboundMediaParams): Promise<OutboundResult> {
  const token = resolveToken(params.config);
  const family = resolveIpFamily(params.config);
  if (!token) return { ok: false, error: "no telegram token" };

  const chatId = Number(params.target.chatId);
  if (!chatId) return { ok: false, error: `invalid chat_id: ${params.target.chatId}` };

  const { readFileSync } = await import("node:fs");
  const data = readFileSync(params.mediaPath);
  const filename = params.mediaPath.split("/").pop() ?? "file";

  const method = {
    image: "sendPhoto",
    video: "sendVideo",
    audio: "sendAudio",
  }[params.mediaType] ?? "sendDocument";

  const fieldName = {
    sendPhoto: "photo",
    sendVideo: "video",
    sendAudio: "audio",
    sendDocument: "document",
  }[method] ?? "document";

  const boundary = `----eli-tg-${randomBytes(12).toString("hex")}`;
  const body = buildMultipartBody(boundary, [
    partFromText("chat_id", String(chatId)),
    partFromFile(fieldName, filename, data),
  ]);
  const response = await requestJson<any>(apiUrl(token, method), {
    method: "POST",
    headers: {
      "Content-Type": `multipart/form-data; boundary=${boundary}`,
      "Content-Length": String(body.length),
    },
    body,
    family,
  });
  if (!response.ok) {
    return { ok: false, error: response.description };
  }
  return { ok: true };
}

function buildMultipartBody(
  boundary: string,
  parts: Array<{ headers: string[]; body: Buffer }>,
): Buffer {
  const chunks = parts.flatMap((part) => [
    Buffer.from(`--${boundary}\r\n${part.headers.join("\r\n")}\r\n\r\n`, "utf8"),
    part.body,
    Buffer.from("\r\n", "utf8"),
  ]);
  chunks.push(Buffer.from(`--${boundary}--\r\n`, "utf8"));
  return Buffer.concat(chunks);
}

function partFromText(name: string, value: string): { headers: string[]; body: Buffer } {
  return {
    headers: [
      `Content-Disposition: form-data; name="${name}"`,
    ],
    body: Buffer.from(value, "utf8"),
  };
}

function partFromFile(name: string, filename: string, body: Buffer): { headers: string[]; body: Buffer } {
  return {
    headers: [
      `Content-Disposition: form-data; name="${name}"; filename="${filename}"`,
      "Content-Type: application/octet-stream",
    ],
    body,
  };
}

function resolveToken(cfg: any): string {
  return (
    cfg?.channels?.telegram?.token ??
    cfg?.channels?.telegram?.accounts?.default?.token ??
    process.env.SIDECAR_TELEGRAM_TOKEN ??
    process.env.ELI_TELEGRAM_TOKEN ??
    ""
  );
}

export function resolveIpFamily(cfg: any): TelegramIpFamily {
  return parseIpFamily(
    cfg?.channels?.telegram?.ip_family ??
    cfg?.channels?.telegram?.accounts?.default?.ip_family ??
    process.env.SIDECAR_TELEGRAM_IP_FAMILY ??
    process.env.ELI_TELEGRAM_IP_FAMILY
  );
}

// ---------------------------------------------------------------------------
// Channel plugin export
// ---------------------------------------------------------------------------

export const telegramPlugin: ChannelPlugin = {
  meta: {
    id: "telegram",
    label: "Telegram",
    blurb: "Telegram Bot API channel",
  },
  config: {
    listAccountIds: () => ["default"],
    resolveAccount: (_cfg, _accountId) => ({}),
  },
  capabilities: {
    chatTypes: ["direct", "group"],
  },
  outbound: {
    sendText,
    sendMedia,
  },
  gateway: {
    async start(params) {
      const channelCfg = params.cfg?.channels?.telegram ?? {};
      const cfg = parseConfig(channelCfg);
      if (!cfg.token) {
        log.error("no telegram token configured, gateway not started");
        return;
      }
      log.info("starting telegram polling", {
        allow_users: cfg.allow_users.size,
        allow_chats: cfg.allow_chats.size,
      });
      await pollLoop(cfg, params.onMessage, params.abortSignal);
    },
  },
  lifecycle: {
    resolveOutboundTarget(_context: Record<string, any>, chatId: string): string {
      return chatId;
    },
  },
};
