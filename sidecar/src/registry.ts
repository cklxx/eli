import type { ChannelPlugin, ToolDefinition, HookHandler, HookOptions } from "./types.js";
import { logger } from "./log.js";

const log = logger("registry");

export interface RegisteredHook {
  handler: HookHandler;
  options: HookOptions;
}

class Registry {
  readonly channels = new Map<string, ChannelPlugin>();
  readonly tools = new Map<string, ToolDefinition>();
  readonly hooks: RegisteredHook[] = [];

  registerChannel(plugin: ChannelPlugin): void {
    const id = plugin.meta.id;
    if (this.channels.has(id)) {
      log.warn("channel already registered, overwriting", { id });
    }
    this.channels.set(id, plugin);
    log.info("channel registered", { id });
  }

  registerTool(tool: ToolDefinition): void {
    if (this.tools.has(tool.name)) {
      log.warn("tool already registered, overwriting", { name: tool.name });
    }
    this.tools.set(tool.name, tool);
    log.info("tool registered", { name: tool.name });
  }

  registerHook(handler: HookHandler, options: HookOptions = {}): void {
    this.hooks.push({ handler, options });
    log.info("hook registered", { event: options.eventType ?? "any" });
  }
}

/** Global singleton registry. */
export const registry = new Registry();
