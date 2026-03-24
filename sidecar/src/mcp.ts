/**
 * MCP (Model Context Protocol) server mode.
 *
 * Exposes all sidecar tools + a `send_message` meta-tool as MCP tools,
 * so external agents (Claude Code, Cursor, custom agents) can discover
 * and call them over stdio or SSE.
 *
 * Usage:
 *   npx eli-sidecar --mcp            # stdio (Claude Code / Cursor)
 *   npx eli-sidecar --mcp=sse        # SSE on sidecar port + /mcp
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  ListToolsRequestSchema,
  CallToolRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { registry } from "./registry.js";
import type { SidecarConfig } from "./config.js";
import type { SessionContext } from "./types.js";
import { sessionContexts } from "./runtime.js";

// ---------------------------------------------------------------------------
// Tool group inference (shared with bridge.ts)
// ---------------------------------------------------------------------------

function inferToolGroup(name: string): string {
  const parts = name.split("_");
  if (parts.length >= 3) return `${parts[0]}-${parts[1]}`;
  return parts[0];
}

// ---------------------------------------------------------------------------
// send_message meta-tool
// ---------------------------------------------------------------------------

const SEND_MESSAGE_TOOL = {
  name: "send_message",
  description:
    "Send a text message to a connected channel (Feishu, WeChat, etc.).\n\n" +
    "Use `list_channels` first to see available channels.",
  inputSchema: {
    type: "object" as const,
    properties: {
      channel: {
        type: "string",
        description: 'Channel ID, e.g. "feishu", "weixin".',
      },
      to: {
        type: "string",
        description:
          "Recipient: chat ID, user open_id, or group chat_id. " +
          'For Feishu direct messages, prefix with "user:" (e.g. "user:ou_xxx").',
      },
      text: { type: "string", description: "Message content (plain text or Markdown)." },
      account_id: {
        type: "string",
        description: 'Account ID within the channel (default: "default").',
      },
    },
    required: ["channel", "to", "text"],
  },
};

const LIST_CHANNELS_TOOL = {
  name: "list_channels",
  description:
    "List all connected channels and their status. " +
    "Call this first to discover which channels are available for send_message.",
  inputSchema: { type: "object" as const, properties: {} },
};

// ---------------------------------------------------------------------------
// MCP server factory
// ---------------------------------------------------------------------------

export interface McpServerOptions {
  transport?: "stdio" | "sse";
  config: SidecarConfig;
}

export async function startMcpServer(options: McpServerOptions): Promise<void> {
  const { config } = options;
  const transport = options.transport ?? "stdio";

  const server = new Server(
    { name: "eli-sidecar", version: "0.2.0" },
    { capabilities: { tools: {} } },
  );

  // -----------------------------------------------------------------------
  // tools/list — expose all sidecar tools + meta-tools
  // -----------------------------------------------------------------------

  server.setRequestHandler(ListToolsRequestSchema, async () => {
    const tools: Array<{
      name: string;
      description: string;
      inputSchema: Record<string, unknown>;
    }> = [];

    // Meta-tools
    tools.push(LIST_CHANNELS_TOOL);
    tools.push(SEND_MESSAGE_TOOL);

    // All plugin-registered tools
    for (const [name, tool] of registry.tools) {
      const group = tool.group ?? inferToolGroup(name);
      tools.push({
        name,
        description: tool.description + (group ? ` [${group}]` : ""),
        inputSchema: tool.parameters ?? { type: "object", properties: {} },
      });
    }

    return { tools };
  });

  // -----------------------------------------------------------------------
  // tools/call — execute tool
  // -----------------------------------------------------------------------

  server.setRequestHandler(CallToolRequestSchema, async (request) => {
    const { name } = request.params;
    const args = (request.params.arguments ?? {}) as Record<string, any>;

    // -- Meta: list_channels --
    if (name === "list_channels") {
      return handleListChannels(config);
    }

    // -- Meta: send_message --
    if (name === "send_message") {
      return handleSendMessage(args, config);
    }

    // -- Plugin tool --
    const tool = registry.tools.get(name);
    if (!tool) {
      return {
        content: [{ type: "text", text: `Unknown tool: "${name}". Use tools/list to see available tools.` }],
        isError: true,
      };
    }

    return executePluginTool(tool, args, config);
  });

  // -----------------------------------------------------------------------
  // Connect transport
  // -----------------------------------------------------------------------

  if (transport === "stdio") {
    const stdioTransport = new StdioServerTransport();
    await server.connect(stdioTransport);
    // Keep process alive; MCP client will close stdin when done.
  } else {
    // SSE: future — for now, only stdio is supported.
    throw new Error(`MCP transport "${transport}" is not yet supported. Use --mcp (stdio).`);
  }
}

// ---------------------------------------------------------------------------
// Handler: list_channels
// ---------------------------------------------------------------------------

function handleListChannels(config: SidecarConfig) {
  const channels: Array<Record<string, any>> = [];

  for (const [id, plugin] of registry.channels) {
    const channelConf = config.channels[id];
    const accountIds = channelConf?.accounts
      ? Object.keys(channelConf.accounts)
      : ["default"];

    channels.push({
      id,
      label: plugin.meta.label,
      accounts: accountIds,
      hasOutbound: !!plugin.outbound?.sendText,
      hasGateway: !!plugin.gateway,
    });
  }

  // Also list tool groups for discoverability.
  const toolGroups = new Map<string, string[]>();
  for (const [name, tool] of registry.tools) {
    const group = tool.group ?? inferToolGroup(name);
    if (!toolGroups.has(group)) toolGroups.set(group, []);
    toolGroups.get(group)!.push(name);
  }

  return {
    content: [
      {
        type: "text",
        text: JSON.stringify({ channels, toolGroups: Object.fromEntries(toolGroups) }, null, 2),
      },
    ],
  };
}

// ---------------------------------------------------------------------------
// Handler: send_message
// ---------------------------------------------------------------------------

async function handleSendMessage(
  args: Record<string, any>,
  config: SidecarConfig,
) {
  const { channel, to, text, account_id } = args;

  if (!channel || !to || !text) {
    return {
      content: [{ type: "text", text: "Missing required parameters: channel, to, text" }],
      isError: true,
    };
  }

  const plugin = registry.channels.get(channel);
  if (!plugin) {
    const available = Array.from(registry.channels.keys()).join(", ");
    return {
      content: [
        {
          type: "text",
          text: `Channel "${channel}" not found. Available: ${available || "(none)"}`,
        },
      ],
      isError: true,
    };
  }

  if (!plugin.outbound?.sendText) {
    return {
      content: [{ type: "text", text: `Channel "${channel}" has no outbound adapter.` }],
      isError: true,
    };
  }

  const accountId = account_id ?? "default";
  const cfg = { channels: config.channels };

  try {
    const result = await plugin.outbound.sendText({ cfg, to, text, accountId });
    return {
      content: [{ type: "text", text: JSON.stringify(result) }],
    };
  } catch (err: any) {
    return {
      content: [{ type: "text", text: `Send failed: ${err?.message ?? err}` }],
      isError: true,
    };
  }
}

// ---------------------------------------------------------------------------
// Handler: plugin tool execution
// ---------------------------------------------------------------------------

async function executePluginTool(
  tool: { name: string; execute(id: string, params: any): Promise<any> },
  args: Record<string, any>,
  config: SidecarConfig,
) {
  // Try to find session context for channel-auth wrapping.
  // External agents can pass _session_id to use an existing session,
  // or _channel + _account_id to create a synthetic one.
  const sessionId = args._session_id as string | undefined;
  const channelHint = args._channel as string | undefined;
  const accountHint = args._account_id as string | undefined;

  // Strip meta-params before passing to the actual tool.
  const cleanArgs = { ...args };
  delete cleanArgs._session_id;
  delete cleanArgs._channel;
  delete cleanArgs._account_id;

  let sessionCtx: SessionContext | null = null;

  if (sessionId) {
    sessionCtx = sessionContexts.get(sessionId) ?? null;
  }

  // Synthetic session for app-level tool calls (no user session needed).
  if (!sessionCtx && channelHint) {
    const plugin = registry.channels.get(channelHint);
    if (plugin) {
      sessionCtx = {
        channel: channelHint,
        accountId: accountHint ?? "default",
        chatId: "",
        senderId: "mcp-agent",
        messageId: "",
        chatType: "p2p",
        cfg: { channels: config.channels },
      };
    }
  }

  try {
    let result;
    const channelPlugin = sessionCtx
      ? registry.channels.get(sessionCtx.channel)
      : undefined;

    if (sessionCtx && channelPlugin?.lifecycle?.wrapToolExecution) {
      result = await channelPlugin.lifecycle.wrapToolExecution(sessionCtx, () =>
        tool.execute(`mcp_${Date.now()}`, cleanArgs),
      );
    } else {
      result = await tool.execute(`mcp_${Date.now()}`, cleanArgs);
    }

    // Normalize result to MCP format.
    if (result?.content && Array.isArray(result.content)) {
      return { content: result.content };
    }
    return {
      content: [{ type: "text", text: JSON.stringify(result, null, 2) }],
    };
  } catch (err: any) {
    return {
      content: [{ type: "text", text: `Tool error: ${err?.message ?? err}` }],
      isError: true,
    };
  }
}
