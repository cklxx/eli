// Structured logger matching Rust tracing format:
//   2026-03-24T13:16:25.924Z  INFO target message key=value key=value

type Level = "TRACE" | "DEBUG" | " INFO" | " WARN" | "ERROR";

function fmt(level: Level, target: string, msg: string, kv?: Record<string, unknown>): string {
  const ts = new Date().toISOString();
  let line = `${ts} ${level} ${target} ${msg}`;
  if (kv) {
    for (const [k, v] of Object.entries(kv)) {
      if (v === undefined) continue;
      line += ` ${k}=${typeof v === "string" && v.includes(" ") ? JSON.stringify(v) : v}`;
    }
  }
  return line;
}

export interface Logger {
  info(msg: string, kv?: Record<string, unknown>): void;
  warn(msg: string, kv?: Record<string, unknown>): void;
  error(msg: string, kv?: Record<string, unknown>): void;
  debug(msg: string, kv?: Record<string, unknown>): void;
}

export function logger(target: string): Logger {
  return {
    info:  (msg, kv?) => console.error(fmt(" INFO", target, msg, kv)),
    warn:  (msg, kv?) => console.error(fmt(" WARN", target, msg, kv)),
    error: (msg, kv?) => console.error(fmt("ERROR", target, msg, kv)),
    debug: (msg, kv?) => console.error(fmt("DEBUG", target, msg, kv)),
  };
}

/** Create a PluginLogger-compatible adapter (variadic args). */
export function pluginLogger(target: string) {
  const log = logger(target);
  return {
    info:  (...args: any[]) => log.info(args.map(String).join(" ")),
    warn:  (...args: any[]) => log.warn(args.map(String).join(" ")),
    error: (...args: any[]) => log.error(args.map(String).join(" ")),
    debug: (...args: any[]) => log.debug(args.map(String).join(" ")),
  };
}
