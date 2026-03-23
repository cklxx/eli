import type { InboundEnvelope, EliChannelMessage } from "./types.js";

/**
 * Convert an OpenClaw-style inbound envelope to eli's ChannelMessage format
 * for POSTing to the webhook channel.
 */
export function envelopeToEliMessage(env: InboundEnvelope): EliChannelMessage {
  const chatId = env.chatId ?? env.senderId;
  const sessionId = `${env.channel}:${env.accountId}:${chatId}`;

  return {
    session_id: sessionId,
    channel: "webhook",
    content: typeof env.text === "string" ? env.text : JSON.stringify(env.text),
    chat_id: chatId,
    is_active: true,
    kind: "normal",
    context: {
      source_channel: env.channel,
      account_id: env.accountId,
      sender_id: env.senderId,
      sender_name: env.senderName ?? "",
      chat_type: env.chatType,
      group_label: env.groupLabel ?? "",
      reply_to_id: env.replyToId ?? "",
      feishu_to: (env as any).feishu_to ?? "",
    },
    output_channel: "webhook",
  };
}

/**
 * Extract routing info from an eli outbound message to determine which
 * channel plugin + account to send through.
 */
export function parseOutboundTarget(msg: EliChannelMessage): {
  sourceChannel: string;
  accountId: string;
  chatId: string;
  chatType: "direct" | "group";
} {
  return {
    sourceChannel: msg.context?.source_channel ?? "",
    accountId: msg.context?.account_id ?? "default",
    chatId: msg.chat_id,
    chatType: (msg.context?.chat_type as "direct" | "group") ?? "direct",
  };
}
