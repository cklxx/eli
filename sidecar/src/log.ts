// Structured logger with colors and level filtering.
//   13:16:25 INFO  target  message  key=value

const LEVELS = { TRACE: 0, DEBUG: 1, INFO: 2, WARN: 3, ERROR: 4 } as const;
type LevelName = keyof typeof LEVELS;

const LOG_LEVEL: number =
  LEVELS[(process.env.LOG_LEVEL?.toUpperCase() as LevelName) ?? ""] ?? LEVELS.INFO;

// ANSI colors (disabled when NO_COLOR is set or not a TTY)
const useColor = !process.env.NO_COLOR && (process.stderr.isTTY ?? false);
const c = {
  reset: useColor ? "\x1b[0m" : "",
  dim: useColor ? "\x1b[2m" : "",
  cyan: useColor ? "\x1b[36m" : "",
  green: useColor ? "\x1b[32m" : "",
  yellow: useColor ? "\x1b[33m" : "",
  red: useColor ? "\x1b[31m" : "",
  gray: useColor ? "\x1b[90m" : "",
};

const LEVEL_STYLE: Record<LevelName, { tag: string; color: string }> = {
  TRACE: { tag: "TRACE", color: c.gray },
  DEBUG: { tag: "DEBUG", color: c.gray },
  INFO:  { tag: " INFO", color: c.green },
  WARN:  { tag: " WARN", color: c.yellow },
  ERROR: { tag: "ERROR", color: c.red },
};

function fmt(level: LevelName, target: string, msg: string, kv?: Record<string, unknown>): string {
  const ts = new Date().toISOString().slice(11, 23); // HH:mm:ss.sss
  const style = LEVEL_STYLE[level];
  let line = `${c.dim}${ts}${c.reset} ${style.color}${style.tag}${c.reset} ${c.cyan}${target}${c.reset} ${msg}`;
  if (kv) {
    for (const [k, v] of Object.entries(kv)) {
      if (v === undefined) continue;
      const val = typeof v === "string" && v.includes(" ") ? JSON.stringify(v) : v;
      line += ` ${c.dim}${k}=${c.reset}${val}`;
    }
  }
  return line;
}

export interface Logger {
  trace(msg: string, kv?: Record<string, unknown>): void;
  debug(msg: string, kv?: Record<string, unknown>): void;
  info(msg: string, kv?: Record<string, unknown>): void;
  warn(msg: string, kv?: Record<string, unknown>): void;
  error(msg: string, kv?: Record<string, unknown>): void;
}

export function logger(target: string): Logger {
  return {
    trace: (msg, kv?) => { if (LOG_LEVEL <= LEVELS.TRACE) console.error(fmt("TRACE", target, msg, kv)); },
    debug: (msg, kv?) => { if (LOG_LEVEL <= LEVELS.DEBUG) console.error(fmt("DEBUG", target, msg, kv)); },
    info:  (msg, kv?) => { if (LOG_LEVEL <= LEVELS.INFO)  console.error(fmt("INFO",  target, msg, kv)); },
    warn:  (msg, kv?) => { if (LOG_LEVEL <= LEVELS.WARN)  console.error(fmt("WARN",  target, msg, kv)); },
    error: (msg, kv?) => { console.error(fmt("ERROR", target, msg, kv)); },
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
