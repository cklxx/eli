// Structured logger with colors and level filtering.
//   13:16:25.924 INFO  target message key=value

type Level = "TRACE" | "DEBUG" | "INFO" | "WARN" | "ERROR";

const LEVEL_ORDER: Record<Level, number> = {
  TRACE: 0,
  DEBUG: 10,
  INFO: 20,
  WARN: 30,
  ERROR: 40,
};

const LOG_LEVEL = resolveLogLevel();
const COLORS = resolveColors();

function resolveLogLevel(): Level {
  const raw = (
    process.env.ELI_SIDECAR_LOG_LEVEL ??
    process.env.LOG_LEVEL ??
    "INFO"
  ).toUpperCase();
  if (raw === "TRACE" || raw === "DEBUG" || raw === "WARN" || raw === "ERROR") return raw;
  return "INFO";
}

function resolveColors() {
  const enabled = !process.env.NO_COLOR && (process.stderr.isTTY ?? false);
  return {
    cyan: enabled ? "\x1b[36m" : "",
    dim: enabled ? "\x1b[2m" : "",
    gray: enabled ? "\x1b[90m" : "",
    green: enabled ? "\x1b[32m" : "",
    red: enabled ? "\x1b[31m" : "",
    reset: enabled ? "\x1b[0m" : "",
    yellow: enabled ? "\x1b[33m" : "",
  };
}

function paint(text: string, color: string): string {
  return color ? `${color}${text}${COLORS.reset}` : text;
}

function levelColor(level: Level): string {
  if (level === "INFO") return COLORS.green;
  if (level === "WARN") return COLORS.yellow;
  if (level === "ERROR") return COLORS.red;
  return COLORS.gray;
}

function formatValue(value: unknown): string {
  if (typeof value === "string" && value.includes(" ")) return JSON.stringify(value);
  return String(value);
}

function formatFields(kv?: Record<string, unknown>): string {
  return Object.entries(kv ?? {})
    .filter(([, value]) => value !== undefined)
    .map(([key, value]) => ` ${paint(`${key}=`, COLORS.dim)}${formatValue(value)}`)
    .join("");
}

function fmt(level: Level, target: string, msg: string, kv?: Record<string, unknown>): string {
  const timestamp = paint(new Date().toISOString().slice(11, 23), COLORS.dim);
  const tag = paint(level.padEnd(5), levelColor(level));
  const scope = paint(target, COLORS.cyan);
  return `${timestamp} ${tag} ${scope} ${msg}${formatFields(kv)}`;
}

function shouldLog(level: Level): boolean {
  return LEVEL_ORDER[level] >= LEVEL_ORDER[LOG_LEVEL];
}

function write(level: Level, target: string, msg: string, kv?: Record<string, unknown>): void {
  if (shouldLog(level)) console.error(fmt(level, target, msg, kv));
}

export interface Logger {
  trace(msg: string, kv?: Record<string, unknown>): void;
  info(msg: string, kv?: Record<string, unknown>): void;
  warn(msg: string, kv?: Record<string, unknown>): void;
  error(msg: string, kv?: Record<string, unknown>): void;
  debug(msg: string, kv?: Record<string, unknown>): void;
}

export function logger(target: string): Logger {
  return {
    trace: (msg, kv?) => write("TRACE", target, msg, kv),
    info: (msg, kv?) => write("INFO", target, msg, kv),
    warn: (msg, kv?) => write("WARN", target, msg, kv),
    error: (msg, kv?) => write("ERROR", target, msg, kv),
    debug: (msg, kv?) => write("DEBUG", target, msg, kv),
  };
}

/** Create a PluginLogger-compatible adapter (variadic args). */
export function pluginLogger(target: string) {
  const log = logger(target);
  return {
    info: (...args: any[]) => log.debug(args.map(String).join(" ")),
    warn: (...args: any[]) => log.warn(args.map(String).join(" ")),
    error: (...args: any[]) => log.error(args.map(String).join(" ")),
    debug: (...args: any[]) => log.debug(args.map(String).join(" ")),
  };
}
