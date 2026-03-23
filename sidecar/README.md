# eli-sidecar

OpenClaw plugin bridge — load any [OpenClaw](https://github.com/nicepkg/openclaw) channel plugin (Feishu, DingTalk, Discord, Slack, ...) and expose channels + tools over HTTP. Works with [eli](https://github.com/cklxx/eli) or any agent framework.

## How it works

```
Feishu/DingTalk/Slack/...
    ↕ (OpenClaw plugin)
┌────────────────────────┐
│  eli-sidecar (:3101)   │
│  ├ Plugin loader (jiti) │
│  ├ Channel registry     │
│  ├ Tool registry        │
│  ├ Skill synthesis      │
│  └ HTTP bridge          │
└────────┬───────────────┘
         │ HTTP JSON
         ▼
┌────────────────────────┐
│  Any agent / LLM app   │
└────────────────────────┘
```

Tools from plugins are exposed as **skills** with progressive disclosure — the LLM sees grouped summaries, not 35 individual tool schemas. When needed, it activates a skill to get tool details, then calls via HTTP.

## Quick start

```bash
npm install eli-sidecar @larksuite/openclaw-lark
npx eli-sidecar
```

## Agent integration (3 steps)

Any agent framework can consume sidecar tools via HTTP:

```ts
// 1. Fetch skill summaries — show to LLM for discovery
const skills = await fetch("http://localhost:3101/skills").then(r => r.json());
// → [{ name: "feishu-calendar", description: "4 tools: ...", tools: [...], body: "..." }, ...]

// 2. When LLM picks a skill, inject skill.body into context
//    (contains tool docs + parameter schemas)

// 3. LLM calls a tool
const result = await fetch("http://localhost:3101/tools/feishu_calendar_event", {
  method: "POST",
  headers: { "Content-Type": "application/json" },
  body: JSON.stringify({ params: { action: "list", start_time: "2026-03-23" } }),
}).then(r => r.json());
```

### With eli (automatic)

```bash
eli gateway --enable-channel webhook
```

Eli auto-starts the sidecar. First run prompts for credentials interactively.

### Programmatic

```ts
import { createSidecar } from "eli-sidecar";

const sidecar = await createSidecar({
  eli_url: "http://localhost:3100",  // optional — only needed for eli integration
  port: 3101,
  plugins: ["@larksuite/openclaw-lark"],
  channels: {
    feishu: {
      appId: "cli_xxx",
      appSecret: "xxx",
      domain: "feishu",
      accounts: { default: { appId: "cli_xxx", appSecret: "xxx", domain: "feishu" } },
    },
  },
});

// later:
await sidecar.stop();
```

### Auto-discovery

If no `plugins` array is specified, the sidecar scans `node_modules` for packages with an `openclaw` field in their `package.json`.

## HTTP API

| Endpoint | Description |
|----------|-------------|
| `GET /health` | Status + registered channels/tools |
| `GET /skills` | **Grouped tool summaries with progressive disclosure** |
| `GET /tools` | Raw tool schemas with group info |
| `POST /tools/:name` | Execute a tool |
| `POST /outbound` | Receive agent replies, route to channel (internal) |

### GET /skills

Returns pre-grouped skills for LLM consumption:

```json
[
  {
    "name": "feishu-calendar",
    "description": "4 tools: feishu_calendar_calendar, feishu_calendar_event, ...",
    "tools": ["feishu_calendar_calendar", "feishu_calendar_event", ...],
    "body": "Call tools in this group via: POST /tools/<name> with { \"params\": {...} }\n\n## feishu_calendar_event\n..."
  }
]
```

**Progressive disclosure pattern:**
1. Show LLM only `name` + `description` of each skill (compact)
2. When LLM picks one, inject `body` into context (full tool docs)
3. LLM calls `POST /tools/:name` with params

## Config

Create `sidecar.json` (or set env vars):

| Field | Env | Default | Description |
|-------|-----|---------|-------------|
| `eli_url` | `SIDECAR_ELI_URL` | `http://127.0.0.1:3100` | Agent webhook endpoint |
| `port` | `SIDECAR_PORT` | `3101` | Sidecar server port |
| `plugins` | — | auto-discover | OpenClaw plugin packages |
| `channels` | — | `{}` | Per-channel credentials |

## Adding plugins

Install any OpenClaw channel plugin as a dependency:

```bash
npm install @larksuite/openclaw-lark     # Feishu/Lark
npm install openclaw-channel-dingtalk    # DingTalk
npm install openclaw-channel-discord     # Discord
```

The sidecar loads them automatically — no per-plugin adaptation needed.

## License

Apache-2.0
