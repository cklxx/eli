// ---------------------------------------------------------------------------
// OpenClaw plugin interface subset — just enough to host real plugins.
// ---------------------------------------------------------------------------

/** Metadata adapter for a channel plugin. */
export interface ChannelMeta {
  id: string;
  label: string;
  selectionLabel?: string;
  docsPath?: string;
  blurb?: string;
  aliases?: string[];
}

/** Config adapter — resolves account credentials. */
export interface ChannelConfig {
  listAccountIds(cfg: any): string[] | Promise<string[]>;
  resolveAccount(cfg: any, accountId: string): any;
}

/** Capabilities declaration. */
export interface ChannelCapabilities {
  chatTypes: Array<"direct" | "group">;
  [key: string]: any;
}

/** Outbound adapter — sends messages TO the platform. */
export interface ChannelOutbound {
  deliveryMode?: "direct" | "queued";
  sendText(params: OutboundTextParams): Promise<OutboundResult>;
  sendMedia?(params: OutboundMediaParams): Promise<OutboundResult>;
  [key: string]: any;
}

export interface OutboundTextParams {
  text: string;
  target: OutboundTarget;
  config: any;
  accountId: string;
}

export interface OutboundMediaParams {
  text?: string;
  mediaPath: string;
  mediaType: string;
  target: OutboundTarget;
  config: any;
  accountId: string;
}

export interface OutboundTarget {
  chatId: string;
  chatType: "direct" | "group";
  senderId?: string;
  [key: string]: any;
}

export interface OutboundResult {
  ok: boolean;
  error?: string;
  [key: string]: any;
}

/** Gateway adapter — receives messages FROM the platform. */
export interface ChannelGateway {
  start?(params: GatewayStartParams): Promise<void>;
  stop?(params: GatewayStopParams): Promise<void>;
  /** OpenClaw uses startAccount/stopAccount in some plugins. */
  startAccount?(params: GatewayStartParams): Promise<void>;
  stopAccount?(params: GatewayStopParams): Promise<void>;
}

export interface GatewayStartParams {
  accountId: string;
  config: any;
  onMessage: (envelope: InboundEnvelope) => void | Promise<void>;
  [key: string]: any;
}

export interface GatewayStopParams {
  accountId: string;
  config: any;
}

/** The composable channel plugin object registered via api.registerChannel(). */
export interface ChannelPlugin {
  id?: string;
  meta: ChannelMeta;
  config: ChannelConfig;
  capabilities: ChannelCapabilities;
  outbound?: ChannelOutbound;
  gateway?: ChannelGateway;
  security?: any;
  groups?: any;
  mentions?: any;
  threading?: any;
  actions?: any;
}

// ---------------------------------------------------------------------------
// Inbound envelope — normalized message from any channel.
// ---------------------------------------------------------------------------

export interface InboundEnvelope {
  channel: string;
  accountId: string;
  senderId: string;
  senderName?: string;
  chatType: "direct" | "group";
  chatId?: string;
  groupLabel?: string;
  text: string;
  mediaPath?: string;
  replyToId?: string;
  [key: string]: any;
}

// ---------------------------------------------------------------------------
// Tool definition — subset of OpenClaw tool registration.
// ---------------------------------------------------------------------------

export interface ToolDefinition {
  name: string;
  description: string;
  parameters: Record<string, any>;
  execute(id: string, params: any): Promise<ToolResult>;
}

export interface ToolResult {
  content: Array<{ type: string; text: string }>;
}

// ---------------------------------------------------------------------------
// Hook handler.
// ---------------------------------------------------------------------------

export interface HookHandler {
  (event: HookEvent): void | Promise<void>;
}

export interface HookEvent {
  type: string;
  action: string;
  sessionKey?: string;
  timestamp: Date;
  messages?: string[];
  context?: Record<string, any>;
}

export interface HookOptions {
  eventType?: string;
  [key: string]: any;
}

// ---------------------------------------------------------------------------
// Plugin definition — the default export of an OpenClaw plugin.
// ---------------------------------------------------------------------------

export interface OpenClawPluginDefinition {
  id: string;
  name?: string;
  description?: string;
  configSchema?: Record<string, any>;
  register(api: OpenClawPluginApi): void;
}

// ---------------------------------------------------------------------------
// Plugin API — the object passed to plugin.register().
// ---------------------------------------------------------------------------

export interface OpenClawPluginApi {
  registerChannel(opts: { plugin: ChannelPlugin }): void;
  registerTool(tool: ToolDefinition, options?: { optional?: boolean }): void;
  registerHook(handler: HookHandler, options?: HookOptions): void;
  registerCommand?(command: any): void;
  registerCli?(cli: any): void;
  registerGatewayMethod?(name: string, handler: any): void;
  registerGatewayRequestHandler?(path: string, handler: any): void;
  registerProviderAuthMethod?(...args: any[]): void;
  registerMemoryPromptSection?(builder: any): void;
  registerService?(service: any): void;
  logger: PluginLogger;
  config: any;
  runtime?: any;
}

export interface PluginLogger {
  info(...args: any[]): void;
  warn(...args: any[]): void;
  error(...args: any[]): void;
  debug(...args: any[]): void;
}

// ---------------------------------------------------------------------------
// Eli bridge message format.
// ---------------------------------------------------------------------------

export interface EliChannelMessage {
  session_id: string;
  channel: string;
  content: string;
  chat_id: string;
  is_active: boolean;
  kind?: "normal" | "error" | "command";
  context: Record<string, any>;
  output_channel: string;
}
