import type {
  OpenClawPluginApi,
  ChannelPlugin,
  ToolDefinition,
  HookHandler,
  HookOptions,
  PluginLogger,
} from "./types.js";
import { registry } from "./registry.js";
import type { SidecarConfig } from "./config.js";

const pluginEventListeners = new Map<string, Array<(...args: any[]) => any>>();

export function removePluginEventListener(event: string, handler: (...args: any[]) => any): void {
  const handlers = pluginEventListeners.get(event);
  if (!handlers) return;
  const idx = handlers.indexOf(handler);
  if (idx >= 0) handlers.splice(idx, 1);
}

export function removeAllPluginEventListeners(): void {
  pluginEventListeners.clear();
}

export async function emitPluginEvent(event: string, ...args: any[]): Promise<void> {
  for (const handler of pluginEventListeners.get(event) ?? []) {
    try {
      await Promise.resolve(handler(...args));
    } catch (err) {
      console.error(`[sidecar] event handler error for "${event}":`, err);
    }
  }
}

/**
 * Mini implementation of OpenClawPluginApi — the object passed to each
 * plugin's `register()` call. Implements only the subset that real channel
 * plugins actually use; unsupported methods log a warning and no-op.
 */
export class SidecarPluginApi implements OpenClawPluginApi {
  readonly logger: PluginLogger;
  readonly config: any;

  /** Simple event emitter for plugin lifecycle events (before_tool_call, etc.). */
  constructor(
    private pluginId: string,
    private sidecarConfig: SidecarConfig,
  ) {
    const prefix = `[${pluginId}]`;
    this.logger = {
      info: (...args: any[]) => console.log(prefix, ...args),
      warn: (...args: any[]) => console.warn(prefix, ...args),
      error: (...args: any[]) => console.error(prefix, ...args),
      debug: (...args: any[]) => console.debug(prefix, ...args),
    };

    // Expose config in the shape OpenClaw plugins expect:
    //   api.config.channels.<channelId>.accounts.<accountId>
    this.config = {
      channels: sidecarConfig.channels,
      get: (key: string) => (sidecarConfig as any)[key],
    };
  }

  /** Subscribe to plugin events (e.g. 'before_tool_call', 'after_tool_call'). */
  on(event: string, handler: (...args: any[]) => void): void {
    if (!pluginEventListeners.has(event)) {
      pluginEventListeners.set(event, []);
    }
    pluginEventListeners.get(event)!.push(handler);
  }

  /** Emit a plugin event. */
  emit(event: string, ...args: any[]): void {
    void emitPluginEvent(event, ...args);
  }

  registerChannel(opts: { plugin: ChannelPlugin }): void {
    registry.registerChannel(opts.plugin);
  }

  registerTool(tool: ToolDefinition, _options?: { optional?: boolean }): void {
    registry.registerTool(tool);
  }

  registerHook(handler: HookHandler, options?: HookOptions): void {
    registry.registerHook(handler, options);
  }

  // -- Unsupported methods: no-op with warning ---------------------------------

  registerCommand(command: any): void {
    this.logger.debug("registerCommand ignored (no CLI in sidecar)", command?.name);
  }

  registerCli(cli: any): void {
    this.logger.debug("registerCli ignored (no CLI in sidecar)", cli?.name);
  }

  registerGatewayMethod(name: string, _handler: any): void {
    this.logger.debug(`registerGatewayMethod ignored: ${name}`);
  }

  registerGatewayRequestHandler(path: string, _handler: any): void {
    this.logger.debug(`registerGatewayRequestHandler ignored: ${path}`);
  }

  registerProviderAuthMethod(..._args: any[]): void {
    this.logger.debug("registerProviderAuthMethod ignored");
  }

  registerMemoryPromptSection(_builder: any): void {
    this.logger.debug("registerMemoryPromptSection ignored");
  }

  registerService(_service: any): void {
    this.logger.debug("registerService ignored");
  }
}
