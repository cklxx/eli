import type { ChannelPlugin, ToolDefinition, HookHandler, HookOptions } from "./types.js";

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
      console.warn(`[registry] channel "${id}" already registered, overwriting`);
    }
    this.channels.set(id, plugin);
    console.log(`[registry] channel registered: ${id}`);
  }

  registerTool(tool: ToolDefinition): void {
    if (this.tools.has(tool.name)) {
      console.warn(`[registry] tool "${tool.name}" already registered, overwriting`);
    }
    this.tools.set(tool.name, tool);
    console.log(`[registry] tool registered: ${tool.name}`);
  }

  registerHook(handler: HookHandler, options: HookOptions = {}): void {
    this.hooks.push({ handler, options });
    console.log(`[registry] hook registered: ${options.eventType ?? "any"}`);
  }
}

/** Global singleton registry. */
export const registry = new Registry();
